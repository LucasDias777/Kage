//! Uma skin carregada: seções, variáveis, medidas e medidores, e a montagem do [`Frame`].

use std::path::{Path, PathBuf};

use crate::color::Color;
use crate::draw::{
    Align, BackgroundFit, BackgroundImage, Effect, Frame, ImageItem, Item, LineItem, ShapeItem, ShapeKind, TextItem,
    TextStyle,
};
use crate::formula;
use crate::ini::{self, Section};
use crate::strftime::{self, DateTime};
use crate::system::{NumberFormat, SystemProbe, format_value};
use crate::vars::Variables;

/// Quem desenha mede: o motor responde com o DirectWrite e o WIC; os testes, com valores fixos.
pub trait TextMeasurer {
    /// Largura e altura do texto com esse estilo, em pixels.
    fn measure_text(&self, text: &str, style: &TextStyle) -> (f32, f32);
    /// Tamanho natural da imagem, se ela abrir.
    fn image_size(&self, path: &Path) -> Option<(f32, f32)>;
}

/// Profundidade máxima de `@include` dentro de `@include`.
const MAX_INCLUDE_DEPTH: usize = 8;

#[derive(Clone, Debug, PartialEq)]
enum MeasureKind {
    Time,
    Calc,
    Cpu,
    PhysicalMemory,
    SwapMemory,
    FreeDiskSpace,
    NetIn,
    NetOut,
    NetTotal,
    /// Plugin NowPlaying ou WebNowPlaying: o `PlayerType` diz qual campo. O `PlayerName` é
    /// ignorado: o Kage lê o que o Windows diz que está tocando, de qualquer player.
    NowPlaying(NowField),
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NowField {
    Artist,
    Title,
    Album,
    Cover,
    Duration,
    Position,
    Progress,
    State,
    Status,
    Other,
}

impl NowField {
    fn parse(player_type: &str) -> Self {
        match player_type.trim().to_ascii_uppercase().as_str() {
            "ARTIST" => Self::Artist,
            "TITLE" => Self::Title,
            "ALBUM" => Self::Album,
            "COVER" => Self::Cover,
            "DURATION" => Self::Duration,
            "POSITION" => Self::Position,
            "PROGRESS" => Self::Progress,
            "STATE" => Self::State,
            "STATUS" => Self::Status,
            _ => Self::Other,
        }
    }
}

/// Segundos como o NowPlaying escreve: `3:05`, ou `1:02:03` com hora.
fn clock_text(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// (valor, mínimo, máximo, texto) do campo, com ou sem música tocando.
fn now_playing_value(field: NowField, now: Option<&crate::system::NowPlaying>) -> (f64, f64, f64, String) {
    let Some(n) = now else {
        return match field {
            NowField::Duration | NowField::Position => (0.0, 0.0, 1.0, clock_text(0.0)),
            NowField::Progress | NowField::State | NowField::Status => (0.0, 0.0, 1.0, "0".into()),
            _ => (0.0, 0.0, 1.0, String::new()),
        };
    };
    match field {
        NowField::Artist => (0.0, 0.0, 1.0, n.artist.clone()),
        NowField::Title => (0.0, 0.0, 1.0, n.title.clone()),
        NowField::Album => (0.0, 0.0, 1.0, n.album.clone()),
        NowField::Cover => (0.0, 0.0, 1.0, n.cover.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()),
        NowField::Duration => (n.duration, 0.0, n.duration.max(1.0), clock_text(n.duration)),
        NowField::Position => (n.position, 0.0, n.duration.max(1.0), clock_text(n.position)),
        NowField::Progress => {
            let p = if n.duration > 0.0 { (n.position / n.duration * 100.0).clamp(0.0, 100.0) } else { 0.0 };
            (p, 0.0, 100.0, plain_number(p))
        }
        NowField::State => (f64::from(n.state), 0.0, 2.0, n.state.to_string()),
        NowField::Status => (1.0, 0.0, 1.0, "1".into()),
        NowField::Other => (0.0, 0.0, 1.0, String::new()),
    }
}

/// Número sem zeros sobrando (o texto de uma variável de seção `[Medida]`).
fn plain_number(value: f64) -> String {
    let text = format!("{value:.5}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.to_string() }
}

#[derive(Clone, Debug)]
struct Measure {
    section: String,
    kind: MeasureKind,
    divider: u32,
    ticks: u32,
    value: f64,
    min: f64,
    max: f64,
    text: String,
    numeric: bool,
}

pub struct Skin {
    /// Nome da configuração, como o Rainmeter mostra (`Mond\Clock`).
    pub config: String,
    pub ini: PathBuf,
    /// Intervalo de atualização em milissegundos; `None` = só uma vez (Update=-1).
    pub update_ms: Option<u64>,
    /// Medidas e medidores que o Kage ainda não sabe fazer.
    pub warnings: Vec<String>,
    sections: Vec<Section>,
    vars: Variables,
    measures: Vec<Measure>,
    meters: Vec<String>,
    current_dir: PathBuf,
    fonts_dir: PathBuf,
    /// Onde cada medidor visível ficou no último desenho, na ordem de desenho (para o mouse).
    hits: Vec<(String, MeterRect)>,
    /// Tamanho do último desenho.
    size: (f32, f32),
}

/// O que o mouse fez em cima da skin (cada um tem a sua ação no INI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEvent {
    LeftDown,
    LeftUp,
    LeftDoubleClick,
    RightUp,
    MiddleUp,
    ScrollUp,
    ScrollDown,
}

impl MouseEvent {
    fn option(self) -> &'static str {
        match self {
            Self::LeftDown => "LeftMouseDownAction",
            Self::LeftUp => "LeftMouseUpAction",
            Self::LeftDoubleClick => "LeftMouseDoubleClickAction",
            Self::RightUp => "RightMouseUpAction",
            Self::MiddleUp => "MiddleMouseUpAction",
            Self::ScrollUp => "MouseScrollUpAction",
            Self::ScrollDown => "MouseScrollDownAction",
        }
    }
}

/// Um passo de uma ação: um bang do Rainmeter (`!Nome arg "arg com espaço"`) ou um programa,
/// arquivo ou endereço para abrir.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Nome em minúsculas, sem o `!` (e sem o prefixo antigo `Rainmeter`).
    Bang { name: String, args: Vec<String> },
    Run(String),
}

/// Retângulo de um medidor: o "âncora" serve ao posicionamento relativo do próximo;
/// o desenhado, ao tamanho da janela.
#[derive(Clone, Copy, Debug, Default)]
struct MeterRect {
    anchor_x: f32,
    anchor_y: f32,
    left: f32,
    top: f32,
    w: f32,
    h: f32,
}

impl Skin {
    pub fn load(ini_path: &Path, skins_root: &Path) -> Result<Self, String> {
        let current_dir = ini_path.parent().ok_or("caminho da skin sem pasta")?.to_path_buf();
        let config = current_dir
            .strip_prefix(skins_root)
            .map(|p| p.to_string_lossy().trim_matches('\\').to_string())
            .unwrap_or_else(|_| current_dir.to_string_lossy().to_string());
        let root_name = config.split('\\').next().unwrap_or_default().to_string();
        let root_dir = skins_root.join(&root_name);
        let resources = root_dir.join("@Resources");

        let mut vars = Variables::default();
        let with_slash = |p: &Path| format!("{}\\", p.to_string_lossy().trim_end_matches('\\'));
        vars.set("CURRENTPATH", &with_slash(&current_dir));
        vars.set("CURRENTFILE", &ini_path.file_name().unwrap_or_default().to_string_lossy());
        vars.set("CURRENTCONFIG", &config);
        vars.set("ROOTCONFIG", &root_name);
        vars.set("ROOTCONFIGPATH", &with_slash(&root_dir));
        vars.set("SKINSPATH", &with_slash(skins_root));
        vars.set("@", &with_slash(&resources));
        vars.set("CRLF", "\n");

        let mut sections: Vec<Section> = Vec::new();
        let mut warnings = Vec::new();
        merge_file(ini_path, &current_dir, &mut sections, &mut vars, &mut warnings, 0)?;

        let mut skin = Self {
            config,
            ini: ini_path.to_path_buf(),
            update_ms: Some(1000),
            warnings,
            sections,
            vars,
            measures: Vec::new(),
            meters: Vec::new(),
            current_dir,
            fonts_dir: resources.join("Fonts"),
            hits: Vec::new(),
            size: (0.0, 0.0),
        };
        skin.update_ms = match skin.number("Rainmeter", "Update") {
            Some(ms) if ms < 0.0 => None,
            Some(ms) => Some((ms as u64).max(16)),
            None => Some(1000),
        };

        for section in skin.sections.clone() {
            if let Some(kind) = skin.option(&section.name, "Measure") {
                let kind = match kind.to_ascii_lowercase().as_str() {
                    "time" => MeasureKind::Time,
                    "calc" => MeasureKind::Calc,
                    "cpu" => MeasureKind::Cpu,
                    "physicalmemory" => MeasureKind::PhysicalMemory,
                    "swapmemory" => MeasureKind::SwapMemory,
                    "freediskspace" => MeasureKind::FreeDiskSpace,
                    "netin" => MeasureKind::NetIn,
                    "netout" => MeasureKind::NetOut,
                    "nettotal" => MeasureKind::NetTotal,
                    "plugin" if is_now_playing_plugin(&skin.option(&section.name, "Plugin").unwrap_or_default()) => {
                        MeasureKind::NowPlaying(NowField::parse(&skin.option(&section.name, "PlayerType").unwrap_or_default()))
                    }
                    other => {
                        let plugin = skin.option(&section.name, "Plugin").unwrap_or_default();
                        let what = if plugin.is_empty() { other.to_string() } else { format!("{other} ({plugin})") };
                        skin.warnings.push(format!("[{}] medida {what} ainda não suportada", section.name));
                        MeasureKind::Unsupported
                    }
                };
                let divider = skin.number(&section.name, "UpdateDivider").unwrap_or(1.0).max(1.0) as u32;
                skin.measures.push(Measure {
                    section: section.name.clone(),
                    // As do "tocando agora" têm texto próprio (3:05, o título...); o número fica
                    // para fórmulas, como no plugin do Rainmeter.
                    numeric: !matches!(kind, MeasureKind::Time | MeasureKind::Unsupported | MeasureKind::NowPlaying(_)),
                    kind,
                    divider,
                    ticks: divider,
                    value: 0.0,
                    min: 0.0,
                    max: 1.0,
                    text: String::new(),
                });
            } else if let Some(kind) = skin.option(&section.name, "Meter") {
                if !matches!(kind.to_ascii_lowercase().as_str(), "string" | "image" | "shape" | "bar") {
                    skin.warnings.push(format!("[{}] medidor {kind} ainda não suportado", section.name));
                }
                skin.meters.push(section.name.clone());
            }
        }
        Ok(skin)
    }

    /// Define ou troca uma variável (o equivalente a `!SetVariable`).
    pub fn set_variable(&mut self, name: &str, value: &str) {
        self.vars.set(name, value);
    }

    pub fn variable(&self, name: &str) -> Option<String> {
        self.vars.get(name).map(|v| self.vars.substitute(v))
    }

    /// Atualiza as medidas com a hora `now` e os valores do sistema, e monta o que desenhar.
    pub fn update(&mut self, now: &DateTime, measurer: &dyn TextMeasurer, system: &dyn SystemProbe) -> Frame {
        self.update_measures(now, system);
        let (frame, hits) = self.build_frame(measurer);
        self.hits = hits;
        self.size = (frame.width, frame.height);
        frame
    }

    // ---------- mouse ----------

    /// Ação do mouse no ponto (x, y) da skin: a do medidor mais de cima que tem essa ação ali,
    /// senão a da seção [Rainmeter]. Já com `$MouseX$`, `$MouseY$`, `$MouseX:%$` e `$MouseY:%$`
    /// trocados (relativos ao medidor, ou à skin inteira).
    pub fn mouse_action(&self, event: MouseEvent, x: f32, y: f32) -> Option<String> {
        let key = event.option();
        for (meter, r) in self.hits.iter().rev() {
            if x >= r.left
                && x < r.left + r.w
                && y >= r.top
                && y < r.top + r.h
                && let Some(action) = self.option(meter, key).filter(|a| !a.trim().is_empty())
            {
                return Some(mouse_variables(&action, x - r.left, y - r.top, r.w, r.h));
            }
        }
        let action = self.option("Rainmeter", key).filter(|a| !a.trim().is_empty())?;
        Some(mouse_variables(&action, x, y, self.size.0, self.size.1))
    }

    /// Faz o que é só da skin e devolve `true`; devolve `false` para o que é com o motor
    /// (`!CommandMeasure`, `!ToggleConfig`, `!Refresh`...).
    pub fn apply_bang(&mut self, name: &str, args: &[String]) -> bool {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
        match name {
            "setvariable" if !arg(0).is_empty() => {
                let value = self.vars.substitute(arg(1));
                // Valor entre parênteses é fórmula (como no Rainmeter): "(#Scale#+0.1)".
                let value = match value.trim().starts_with('(') {
                    true => formula::eval_with(&value, &|n| self.measure_value(n)).map(plain_number).unwrap_or(value),
                    false => value,
                };
                self.vars.set(arg(0), &value);
                true
            }
            "setoption" if !arg(0).is_empty() && !arg(1).is_empty() => {
                if let Some(section) = self.sections.iter_mut().find(|s| s.name.eq_ignore_ascii_case(arg(0))) {
                    section.set(arg(1), arg(2));
                }
                true
            }
            "showmeter" | "hidemeter" | "togglemeter" if !arg(0).is_empty() => {
                let hide = match name {
                    "showmeter" => false,
                    "hidemeter" => true,
                    _ => !self.flag(arg(0), "Hidden", false),
                };
                if let Some(section) = self.sections.iter_mut().find(|s| s.name.eq_ignore_ascii_case(arg(0))) {
                    section.set("Hidden", if hide { "1" } else { "0" });
                }
                true
            }
            "update" | "redraw" | "updatemeter" | "updatemeasure" | "updatemetergroup" | "updatemeasuregroup" => true,
            _ => false,
        }
    }

    /// A medida é do plugin "tocando agora" (para o motor mandar os comandos ao player).
    pub fn is_now_playing(&self, measure: &str) -> bool {
        self.measure(measure).is_some_and(|m| matches!(m.kind, MeasureKind::NowPlaying(_)))
    }

    /// Usa medidas que mudam a cada atualização (CPU, rede...): o relógio sozinho não precisa
    /// redesenhar a cada segundo.
    pub fn has_system_measures(&self) -> bool {
        self.measures.iter().any(|m| !matches!(m.kind, MeasureKind::Time | MeasureKind::Calc | MeasureKind::Unsupported))
    }

    // ---------- opções ----------

    fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Valor bruto da opção, procurando também nos estilos do `MeterStyle`.
    fn raw(&self, section: &str, key: &str) -> Option<String> {
        let s = self.section(section)?;
        if let Some(v) = s.get(key) {
            return Some(v.to_string());
        }
        let styles = s.get("MeterStyle")?;
        self.vars
            .substitute(styles)
            .split('|')
            .filter_map(|style| self.section(style.trim())?.get(key).map(str::to_string))
            .next_back()
    }

    /// Opção com variáveis trocadas (as aspas em volta já saíram na leitura do INI).
    fn option(&self, section: &str, key: &str) -> Option<String> {
        self.raw(section, key).map(|v| self.section_vars(&self.vars.substitute(&v)))
    }

    /// Variáveis de seção: `[Medida]` vira o valor da medida (o texto, ou o número sem zeros
    /// sobrando, já com o Substitute) e `[Medida:%]`, o percentual. O que não for nome de medida
    /// fica como está, inclusive os `[!Bang ...]` das ações.
    fn section_vars(&self, text: &str) -> String {
        if self.measures.is_empty() || !text.contains('[') {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find('[') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let Some(end) = after.find(']') else {
                out.push_str(&rest[start..]);
                return out;
            };
            let inner = &after[..end];
            let (name, modifier) = inner.split_once(':').unwrap_or((inner, ""));
            match self.measure(name.trim()) {
                Some(m) => {
                    let value = match modifier.trim() {
                        "%" => plain_number(if m.max > m.min { (m.value - m.min) / (m.max - m.min) * 100.0 } else { 0.0 }),
                        _ if !m.numeric => m.text.clone(),
                        _ => self.substituted(&m.section, plain_number(m.value)),
                    };
                    out.push_str(&value);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('[');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Regra do `Substitute`. Como a leitura do INI tira as aspas das pontas, o Rainmeter repõe
    /// aspas duplas quando o valor não começa e termina com aspas diferentes (Measure.cpp).
    fn substitute_rule(&self, section: &str) -> Option<String> {
        let value = self.vars.substitute(&self.raw(section, "Substitute")?);
        let mixed = (value.starts_with('"') && value.ends_with('\'')) || (value.starts_with('\'') && value.ends_with('"'));
        Some(if mixed { value } else { format!("\"{value}\"") })
    }

    fn number(&self, section: &str, key: &str) -> Option<f64> {
        formula::number(&self.option(section, key)?)
    }

    fn color(&self, section: &str, key: &str) -> Option<Color> {
        Color::parse(&self.option(section, key)?)
    }

    fn flag(&self, section: &str, key: &str, default: bool) -> bool {
        self.number(section, key).map(|n| n != 0.0).unwrap_or(default)
    }

    fn resolve_path(&self, value: &str) -> PathBuf {
        let p = PathBuf::from(value);
        if p.is_absolute() { p } else { self.current_dir.join(p) }
    }

    // ---------- medidas ----------

    fn update_measures(&mut self, now: &DateTime, system: &dyn SystemProbe) {
        for i in 0..self.measures.len() {
            let m = &mut self.measures[i];
            m.ticks += 1;
            if m.ticks < m.divider {
                continue;
            }
            m.ticks = 0;
            let section = m.section.clone();
            if self.flag(&section, "Disabled", false) {
                continue;
            }
            let total = self.flag(&section, "Total", false);
            // (valor, mínimo, máximo, texto) antes de MinValue, MaxValue e InvertMeasure.
            let (value, min, max, text) = match self.measures[i].kind {
                MeasureKind::Time => {
                    let format = self.option(&section, "Format").unwrap_or_else(|| "%H:%M:%S".into());
                    (0.0, 0.0, 1.0, strftime::format(&format, now))
                }
                MeasureKind::Calc => {
                    let expr = self.option(&section, "Formula").unwrap_or_default();
                    let value = formula::eval_with(&expr, &|name| self.measure_value(name)).unwrap_or(0.0);
                    (value, 0.0, 1.0, String::new())
                }
                MeasureKind::Cpu => (system.cpu_percent(), 0.0, 100.0, String::new()),
                MeasureKind::PhysicalMemory | MeasureKind::SwapMemory => {
                    let (used, size) =
                        if self.measures[i].kind == MeasureKind::PhysicalMemory { system.memory() } else { system.swap() };
                    (if total { size } else { used }, 0.0, size, String::new())
                }
                MeasureKind::FreeDiskSpace => {
                    let drive = self.option(&section, "Drive").filter(|d| !d.is_empty()).unwrap_or_else(|| "C:".into());
                    let (free, size) = system.disk(&drive).unwrap_or((0.0, 0.0));
                    (if total { size } else { free }, 0.0, size, String::new())
                }
                MeasureKind::NetIn | MeasureKind::NetOut | MeasureKind::NetTotal => {
                    let (inbound, outbound) = system.network();
                    let value = match self.measures[i].kind {
                        MeasureKind::NetIn => inbound,
                        MeasureKind::NetOut => outbound,
                        _ => inbound + outbound,
                    };
                    (value, 0.0, 0.0, String::new())
                }
                MeasureKind::NowPlaying(field) => now_playing_value(field, system.now_playing().as_ref()),
                MeasureKind::Unsupported => (0.0, 0.0, 1.0, String::new()),
            };
            let min = self.number(&section, "MinValue").unwrap_or(min);
            let max = self.number(&section, "MaxValue").unwrap_or(max);
            let value = if self.flag(&section, "InvertMeasure", false) { max - (value - min) } else { value };
            let text = self.substituted(&section, text);
            let m = &mut self.measures[i];
            m.value = value;
            m.min = min;
            m.max = max;
            m.text = text;
        }
    }

    fn measure(&self, name: &str) -> Option<&Measure> {
        self.measures.iter().find(|m| m.section.eq_ignore_ascii_case(name))
    }

    fn measure_value(&self, name: &str) -> Option<f64> {
        self.measure(name).map(|m| m.value)
    }

    /// Texto da medida para um medidor: números no formato pedido pelo medidor.
    fn measure_text(&self, name: &str, format: &NumberFormat) -> String {
        let Some(m) = self.measure(name) else { return String::new() };
        if !m.numeric {
            return m.text.clone();
        }
        self.substituted(&m.section, format_value(m.value, m.min, m.max, format))
    }

    /// Texto depois do `Substitute` da seção (com `RegExpSubstitute=1`, cada par é uma expressão).
    fn substituted(&self, section: &str, text: String) -> String {
        match self.substitute_rule(section) {
            Some(rule) => apply_substitute(&text, &rule, self.flag(section, "RegExpSubstitute", false)),
            None => text,
        }
    }

    // ---------- medidores ----------

    fn build_frame(&self, measurer: &dyn TextMeasurer) -> (Frame, Vec<(String, MeterRect)>) {
        let mut items = Vec::new();
        let mut hits = Vec::new();
        let mut previous = MeterRect::default();
        let (mut right, mut bottom) = (0f32, 0f32);

        for meter in &self.meters {
            let kind = self.option(meter, "Meter").unwrap_or_default().to_ascii_lowercase();
            let x = self.position(meter, "X", previous.anchor_x, previous.w);
            let y = self.position(meter, "Y", previous.anchor_y, previous.h);
            let mut meter_items = Vec::new();
            let rect = match kind.as_str() {
                "string" => self.string_meter(meter, x, y, measurer, &mut meter_items),
                "image" => self.image_meter(meter, x, y, measurer, &mut meter_items),
                "shape" => self.shape_meter(meter, x, y, &mut meter_items),
                "bar" => self.bar_meter(meter, x, y, &mut meter_items),
                _ => MeterRect { anchor_x: x, anchor_y: y, left: x, top: y, ..Default::default() },
            };
            if self.flag(meter, "Hidden", false) {
                // Escondido: conta como anterior com W e H zerados, e o X dele ainda alarga a
                // janela (Skin.cpp: UpdateRelativeMeters e ResizeWindow).
                previous = MeterRect { w: 0.0, h: 0.0, ..rect };
                right = right.max(rect.anchor_x);
                bottom = bottom.max(rect.anchor_y);
                continue;
            }
            previous = rect;
            hits.push((meter.clone(), rect));
            if let Some(color) = self.color(meter, "SolidColor").filter(|c| c.a > 0) {
                items.push(Item::Shape(ShapeItem {
                    kind: ShapeKind::Rectangle { radius: 0.0 },
                    x: rect.left,
                    y: rect.top,
                    w: rect.w,
                    h: rect.h,
                    fill: Some(color),
                    stroke: None,
                }));
            }
            items.extend(meter_items);
            right = right.max(rect.left + rect.w);
            bottom = bottom.max(rect.top + rect.h);
        }

        // Tamanho da janela como o ResizeWindow do Rainmeter: começa na margem esquerda (de
        // cima), pega o fim dos medidores, soma a margem direita (de baixo) e no mínimo cobre a
        // imagem de fundo.
        let margins = self.background_margins();
        let background_path = self
            .option("Rainmeter", "Background")
            .filter(|b| !b.is_empty())
            .map(|b| self.resolve_path(&b));
        let mode = self
            .number("Rainmeter", "BackgroundMode")
            .map(|m| m as i32)
            .unwrap_or(if background_path.is_some() { 0 } else { 1 });
        let background_image = match (mode, background_path) {
            (0 | 3 | 4, Some(path)) => Some(BackgroundImage {
                fit: match mode {
                    3 => BackgroundFit::Scaled,
                    4 => BackgroundFit::Tiled,
                    _ => BackgroundFit::Natural,
                },
                path,
                margins,
            }),
            _ => None,
        };
        let image_size = background_image
            .as_ref()
            .and_then(|b| measurer.image_size(&b.path))
            .unwrap_or((0.0, 0.0));
        let width = right.max(margins.0) + margins.2;
        let height = bottom.max(margins.1) + margins.3;
        let width = self.number("Rainmeter", "SkinWidth").map(|w| w as f32).unwrap_or(width.max(image_size.0));
        let height = self.number("Rainmeter", "SkinHeight").map(|h| h as f32).unwrap_or(height.max(image_size.1));
        let background = if mode == 2 { self.color("Rainmeter", "SolidColor") } else { None };
        let frame = Frame {
            width: width.max(1.0).ceil(),
            height: height.max(1.0).ceil(),
            background,
            background_image,
            items,
        };
        (frame, hits)
    }

    /// `BackgroundMargins=esquerda,cima,direita,baixo`.
    fn background_margins(&self) -> (f32, f32, f32, f32) {
        let Some(value) = self.option("Rainmeter", "BackgroundMargins") else { return (0.0, 0.0, 0.0, 0.0) };
        let n: Vec<f32> = split_args(&value).iter().map(|p| formula::number(p).unwrap_or(0.0) as f32).collect();
        let at = |i: usize| n.get(i).copied().unwrap_or(0.0);
        (at(0), at(1), at(2), at(3))
    }

    /// X ou Y do medidor. Sufixo `r`: relativo à posição do anterior; `R`: ao fim dele.
    fn position(&self, meter: &str, key: &str, prev_pos: f32, prev_size: f32) -> f32 {
        let Some(raw) = self.option(meter, key) else { return 0.0 };
        let raw = raw.trim();
        let (value, base) = if let Some(v) = raw.strip_suffix('r') {
            (v, prev_pos)
        } else if let Some(v) = raw.strip_suffix('R') {
            (v, prev_pos + prev_size)
        } else {
            (raw, 0.0)
        };
        base + formula::number(value).unwrap_or(0.0) as f32
    }

    fn string_meter(
        &self,
        meter: &str,
        x: f32,
        y: f32,
        measurer: &dyn TextMeasurer,
        items: &mut Vec<Item>,
    ) -> MeterRect {
        // Sem NumOfDecimals: 0 casas se Scale não tem ponto e (Percentual=1 ou AutoScale=0),
        // senão 1 (MeterString.cpp).
        let percentual = self.flag(meter, "Percentual", false);
        // AutoScale=1 (base 1024), 2 (base 1000); "1k" e "2k" contam como 1 e 2.
        let autoscale = self
            .option(meter, "AutoScale")
            .and_then(|a| a.trim().chars().next()?.to_digit(10))
            .map_or(0, |d| d.min(2) as u8);
        let decimals = match self.number(meter, "NumOfDecimals") {
            Some(n) => n.max(0.0) as usize,
            None => {
                let scale_has_dot = self.option(meter, "Scale").is_some_and(|s| s.contains('.'));
                if !scale_has_dot && (percentual || autoscale == 0) { 0 } else { 1 }
            }
        };
        let format = NumberFormat {
            decimals,
            autoscale,
            percentual,
            scale: self.number(meter, "Scale").unwrap_or(1.0),
        };
        let mut values = Vec::new();
        if let Some(first) = self.option(meter, "MeasureName") {
            values.push(self.measure_text(&first, &format));
            for n in 2.. {
                match self.option(meter, &format!("MeasureName{n}")) {
                    Some(name) => values.push(self.measure_text(&name, &format)),
                    None => break,
                }
            }
        }
        let template = self.option(meter, "Text").unwrap_or_else(|| {
            let prefix = self.option(meter, "Prefix").unwrap_or_default();
            let postfix = self.option(meter, "Postfix").unwrap_or_default();
            if values.is_empty() { format!("{prefix}{postfix}") } else { format!("{prefix}%1{postfix}") }
        });
        let mut text = template;
        // Do maior para o menor, para "%1" não comer o começo de "%10".
        for (i, value) in values.iter().enumerate().rev() {
            text = text.replace(&format!("%{}", i + 1), value);
        }
        let text = match self.option(meter, "StringCase").unwrap_or_default().to_ascii_lowercase().as_str() {
            "upper" => text.to_uppercase(),
            "lower" => text.to_lowercase(),
            "proper" => proper_case(&text),
            _ => text,
        };

        let style_name = self.option(meter, "StringStyle").unwrap_or_default().to_ascii_lowercase();
        // FontSize padrão 10; negativo também vira 10 (MeterString.cpp).
        let points = self.number(meter, "FontSize").filter(|p| *p >= 0.0).unwrap_or(10.0) as f32;
        let style = TextStyle {
            face: self.option(meter, "FontFace").filter(|f| !f.is_empty()).unwrap_or_else(|| "Arial".into()),
            size_px: points * 4.0 / 3.0,
            bold: style_name.contains("bold"),
            italic: style_name.contains("italic"),
            spacing: self.character_spacing(meter),
            fonts_dir: Some(self.fonts_dir.clone()),
            gdi_emulation: !self.flag("Rainmeter", "AccurateText", false),
        };
        let (text_w, text_h) = if text.is_empty() { (0.0, 0.0) } else { measurer.measure_text(&text, &style) };
        let box_w = self.number(meter, "W").map(|w| w as f32).unwrap_or(text_w);
        let box_h = self.number(meter, "H").map(|h| h as f32).unwrap_or(text_h);

        let align = self.option(meter, "StringAlign").unwrap_or_default().to_ascii_lowercase();
        let (h_align, horizontal) = if align.starts_with("center") {
            (Align::Center, 0.5)
        } else if align.starts_with("right") {
            (Align::End, 1.0)
        } else {
            (Align::Start, 0.0)
        };
        let (v_align, vertical) = if align.len() > 6 && align.ends_with("center") {
            (Align::Center, 0.5)
        } else if align.ends_with("bottom") {
            (Align::End, 1.0)
        } else {
            (Align::Start, 0.0)
        };
        let left = x - box_w * horizontal;
        let top = y - box_h * vertical;

        if !text.is_empty() {
            let effect = match self.option(meter, "StringEffect").unwrap_or_default().to_ascii_lowercase().as_str() {
                "shadow" => Some(Effect::Shadow),
                "border" => Some(Effect::Border),
                _ => None,
            };
            items.push(Item::Text(TextItem {
                text,
                style,
                // Padrão do Rainmeter: preto.
                color: self.color(meter, "FontColor").unwrap_or(Color::BLACK),
                left,
                top,
                width: box_w,
                height: box_h,
                h_align,
                v_align,
                antialias: self.flag(meter, "AntiAlias", false),
                effect: effect.map(|e| (e, self.color(meter, "FontEffectColor").unwrap_or(Color::BLACK))),
            }));
        }
        MeterRect { anchor_x: x, anchor_y: y, left, top, w: box_w, h: box_h }
    }

    /// `InlineSetting=CharacterSpacing | antes | depois | avanço mínimo`, em qualquer
    /// InlineSettingN. Os valores estão em pontos e viram pixels com x 4/3
    /// (TextInlineFormatCharacterSpacing.cpp); só o primeiro é obrigatório.
    fn character_spacing(&self, meter: &str) -> Option<(f32, f32, f32)> {
        let keys = std::iter::once("InlineSetting".to_string()).chain((2..10).map(|n| format!("InlineSetting{n}")));
        for key in keys {
            let Some(value) = self.option(meter, &key) else { continue };
            let parts: Vec<&str> = value.split('|').map(str::trim).collect();
            if parts.first().is_some_and(|p| p.eq_ignore_ascii_case("CharacterSpacing")) && parts.len() >= 2 {
                let num = |i: usize| parts.get(i).and_then(|p| formula::number(p)).unwrap_or(0.0) as f32 * 4.0 / 3.0;
                return Some((num(1), num(2), num(3).max(0.0)));
            }
        }
        None
    }

    fn image_meter(
        &self,
        meter: &str,
        x: f32,
        y: f32,
        measurer: &dyn TextMeasurer,
        items: &mut Vec<Item>,
    ) -> MeterRect {
        // Sem imagem, o medidor ocupa o W e H declarados (o separador do illustro é assim).
        let declared = MeterRect {
            anchor_x: x,
            anchor_y: y,
            left: x,
            top: y,
            w: self.number(meter, "W").unwrap_or(0.0) as f32,
            h: self.number(meter, "H").unwrap_or(0.0) as f32,
        };
        let Some(name) = self.option(meter, "ImageName").filter(|n| !n.is_empty()) else { return declared };
        let folder = self.option(meter, "ImagePath").unwrap_or_default();
        let path = self.resolve_path(&format!("{folder}{}{name}", if folder.is_empty() || folder.ends_with('\\') { "" } else { "\\" }));
        let natural = measurer.image_size(&path).unwrap_or((0.0, 0.0));
        let w_opt = self.number(meter, "W").map(|v| v as f32);
        let h_opt = self.number(meter, "H").map(|v| v as f32);
        let aspect = if natural.1 > 0.0 { natural.0 / natural.1 } else { 1.0 };
        let (w, h) = match (w_opt, h_opt) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w / aspect),
            (None, Some(h)) => (h * aspect, h),
            (None, None) => natural,
        };
        let alpha = self.number(meter, "ImageAlpha").unwrap_or(255.0).clamp(0.0, 255.0) as u8;
        items.push(Item::Image(ImageItem { path, x, y, w, h, alpha }));
        MeterRect { anchor_x: x, anchor_y: y, left: x, top: y, w, h }
    }

    fn shape_meter(&self, meter: &str, x: f32, y: f32, items: &mut Vec<Item>) -> MeterRect {
        let keys = std::iter::once("Shape".to_string()).chain((2..64).map(|n| format!("Shape{n}")));
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for key in keys {
            let Some(definition) = self.option(meter, &key) else {
                if key != "Shape" {
                    break;
                }
                continue;
            };
            let Some(item) = parse_shape(&definition, x, y) else { continue };
            let (l, t, r, b) = match &item {
                Item::Shape(s) => (s.x, s.y, s.x + s.w, s.y + s.h),
                Item::Line(l) => (l.from.0.min(l.to.0), l.from.1.min(l.to.1), l.from.0.max(l.to.0), l.from.1.max(l.to.1)),
                _ => continue,
            };
            min_x = min_x.min(l);
            min_y = min_y.min(t);
            max_x = max_x.max(r);
            max_y = max_y.max(b);
            items.push(item);
        }
        if min_x == f32::MAX {
            return MeterRect { anchor_x: x, anchor_y: y, left: x, top: y, ..Default::default() };
        }
        MeterRect { anchor_x: x, anchor_y: y, left: x, top: y, w: max_x - x, h: max_y - y }
    }

    fn bar_meter(&self, meter: &str, x: f32, y: f32, items: &mut Vec<Item>) -> MeterRect {
        let w = self.number(meter, "W").unwrap_or(0.0) as f32;
        let h = self.number(meter, "H").unwrap_or(0.0) as f32;
        let fraction = self
            .option(meter, "MeasureName")
            .and_then(|name| {
                let m = self.measure(&name)?;
                Some(if m.max > m.min { ((m.value - m.min) / (m.max - m.min)).clamp(0.0, 1.0) } else { 0.0 })
            })
            .unwrap_or(0.0) as f32;
        let horizontal = self.option(meter, "BarOrientation").is_some_and(|o| o.eq_ignore_ascii_case("horizontal"));
        let (bw, bh, by) = if horizontal { (w * fraction, h, y) } else { (w, h * fraction, y + h * (1.0 - fraction)) };
        items.push(Item::Shape(ShapeItem {
            kind: ShapeKind::Rectangle { radius: 0.0 },
            x,
            y: by,
            w: bw,
            h: bh,
            fill: Some(self.color(meter, "BarColor").unwrap_or(Color { r: 0, g: 255, b: 0, a: 255 })),
            stroke: None,
        }));
        MeterRect { anchor_x: x, anchor_y: y, left: x, top: y, w, h }
    }
}

/// Lê um arquivo de skin e junta as seções dele, seguindo os `@include`.
fn merge_file(
    path: &Path,
    current_dir: &Path,
    sections: &mut Vec<Section>,
    vars: &mut Variables,
    warnings: &mut Vec<String>,
    depth: usize,
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("não abriu {}: {e}", path.display()))?;
    for section in ini::parse(&ini::decode(&bytes)) {
        let is_variables = section.name.eq_ignore_ascii_case("Variables");
        if !sections.iter().any(|s| s.name.eq_ignore_ascii_case(&section.name)) {
            sections.push(Section { name: section.name.clone(), keys: Vec::new() });
        }
        for (key, value) in section.keys {
            if key.to_ascii_lowercase().starts_with("@include") {
                let file = vars.substitute(ini::unquote(&value));
                let file = PathBuf::from(&file);
                let file = if file.is_absolute() { file } else { current_dir.join(file) };
                if depth >= MAX_INCLUDE_DEPTH {
                    warnings.push(format!("@include profundo demais: {}", file.display()));
                } else if let Err(e) = merge_file(&file, current_dir, sections, vars, warnings, depth + 1) {
                    warnings.push(e);
                }
                continue;
            }
            if is_variables {
                vars.set(&key, &value);
            }
            if let Some(target) = sections.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&section.name)) {
                target.set(&key, &value);
            }
        }
    }
    Ok(())
}

/// `Substitute="Friday":"Sexta","November":"Novembro"`: troca cada par, na ordem. Com `regex`
/// (o `RegExpSubstitute=1`), cada par é expressão e substituição, com `\1` para o grupo 1.
fn apply_substitute(text: &str, rule: &str, regex: bool) -> String {
    let mut quoted = Vec::new();
    let mut chars = rule.chars();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let token: String = chars.by_ref().take_while(|&d| d != c).collect();
            quoted.push(token);
        }
    }
    let mut out = text.to_string();
    for [from, to] in quoted.as_chunks::<2>().0 {
        if from.is_empty() {
            continue;
        }
        if regex {
            // Expressão que não compila é ignorada, como uma substituição sem efeito.
            if let Ok(re) = regex::Regex::new(from) {
                out = re.replace_all(&out, regex_replacement(to).as_str()).into_owned();
            }
        } else {
            out = out.replace(from, to);
        }
    }
    out
}

/// Substituição no estilo do Rainmeter (PCRE: `\1`) para a do crate regex (`${1}`).
fn regex_replacement(to: &str) -> String {
    let mut out = String::with_capacity(to.len() + 4);
    let mut chars = to.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek().is_some_and(char::is_ascii_digit) => {
                let mut group = String::new();
                while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                    group.push(d);
                    chars.next();
                }
                out.push_str(&format!("${{{group}}}"));
            }
            '$' => out.push_str("$$"),
            _ => out.push(c),
        }
    }
    out
}

/// Troca `$MouseX$` e `$MouseY$` (pixels) e `$MouseX:%$` e `$MouseY:%$` (percentual) na ação.
fn mouse_variables(action: &str, x: f32, y: f32, w: f32, h: f32) -> String {
    let percent = |v: f32, size: f32| if size > 0.0 { (v / size * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    let pairs = [
        ("$mousex:%$", (percent(x, w).round() as i32).to_string()),
        ("$mousey:%$", (percent(y, h).round() as i32).to_string()),
        ("$mousex$", (x.round() as i32).to_string()),
        ("$mousey$", (y.round() as i32).to_string()),
    ];
    let mut out = action.to_string();
    for (from, to) in pairs {
        // Minúsculas só no ASCII: as posições do texto não mudam.
        while let Some(at) = out.to_ascii_lowercase().find(from) {
            out.replace_range(at..at + from.len(), &to);
        }
    }
    out
}

/// Lê uma ação do Rainmeter: `[!Bang1 a "b c"][!Bang2]`, um bang solto (`!Bang a`) ou algo
/// para abrir (`["https://..."]`, um programa, um arquivo).
pub fn parse_actions(text: &str) -> Vec<Action> {
    let text = text.trim();
    let groups = if text.starts_with('[') { bracket_groups(text) } else { vec![text.to_string()] };
    groups.iter().filter_map(|g| parse_action(g.trim())).collect()
}

/// Conteúdo de cada `[...]` de primeiro nível (colchetes dentro de aspas não contam).
fn bracket_groups(text: &str) -> Vec<String> {
    let (mut groups, mut current) = (Vec::new(), String::new());
    let (mut depth, mut quoted) = (0usize, false);
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                if depth > 0 {
                    current.push(c);
                }
            }
            '[' if !quoted => {
                if depth > 0 {
                    current.push(c);
                }
                depth += 1;
            }
            ']' if !quoted && depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    groups.push(std::mem::take(&mut current));
                } else {
                    current.push(c);
                }
            }
            _ if depth > 0 => current.push(c),
            _ => {}
        }
    }
    groups
}

fn parse_action(text: &str) -> Option<Action> {
    if text.is_empty() {
        return None;
    }
    let Some(rest) = text.strip_prefix('!') else {
        return Some(Action::Run(text.trim_matches('"').to_string()));
    };
    let mut tokens = split_quoted(rest);
    if tokens.is_empty() {
        return None;
    }
    let mut name = tokens.remove(0).to_ascii_lowercase();
    // Nomes antigos: !RainmeterToggleConfig é o !ToggleConfig.
    if let Some(short) = name.strip_prefix("rainmeter").filter(|s| !s.is_empty()) {
        name = short.to_string();
    }
    Some(Action::Bang { name, args: tokens })
}

/// Argumentos separados por espaço; `"com espaço"` e `"""com "aspas" dentro"""` contam como um.
fn split_quoted(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let (token, next) = if let Some(r) = rest.strip_prefix("\"\"\"") {
            r.find("\"\"\"").map_or((r, ""), |e| (&r[..e], &r[e + 3..]))
        } else if let Some(r) = rest.strip_prefix('"') {
            r.find('"').map_or((r, ""), |e| (&r[..e], &r[e + 1..]))
        } else {
            let e = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (&rest[..e], &rest[e..])
        };
        out.push(token.to_string());
        rest = next.trim_start();
    }
    out
}

fn is_now_playing_plugin(plugin: &str) -> bool {
    let name = plugin.rsplit(['\\', '/']).next().unwrap_or(plugin).to_ascii_lowercase();
    let name = name.strip_suffix(".dll").unwrap_or(&name);
    matches!(name, "nowplaying" | "webnowplaying")
}

fn proper_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for c in text.chars() {
        if start {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        start = c.is_whitespace();
    }
    out
}

/// Divide por vírgulas fora de parênteses (`0,0,(Max(1,2)),10`).
fn split_args(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    parts.push(current);
    parts
}

/// `Rectangle 0,0,100,50,5 | Fill Color 0,0,0,128 | StrokeWidth 2 | Stroke Color 255,255,255`
fn parse_shape(definition: &str, x: f32, y: f32) -> Option<Item> {
    let mut parts = definition.split('|').map(str::trim);
    let head = parts.next()?;
    let (kind, args) = head.split_once(char::is_whitespace)?;
    let nums: Vec<f32> = split_args(args).iter().map(|a| formula::number(a).unwrap_or(0.0) as f32).collect();
    let mut fill = Some(Color::WHITE);
    let mut stroke_color = Color::BLACK;
    let mut stroke_width = 1.0f32;
    for modifier in parts {
        let lower = modifier.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("fill color") {
            fill = Color::parse(rest.trim());
        } else if let Some(rest) = lower.strip_prefix("stroke color") {
            stroke_color = Color::parse(rest.trim()).unwrap_or(stroke_color);
        } else if let Some(rest) = lower.strip_prefix("strokewidth") {
            stroke_width = formula::number(rest.trim()).unwrap_or(1.0) as f32;
        }
    }
    let stroke = (stroke_width > 0.0 && stroke_color.a > 0).then_some((stroke_color, stroke_width));
    let at = |i: usize| nums.get(i).copied().unwrap_or(0.0);
    match kind.to_ascii_lowercase().as_str() {
        "rectangle" => Some(Item::Shape(ShapeItem {
            kind: ShapeKind::Rectangle { radius: at(4) },
            x: x + at(0),
            y: y + at(1),
            w: at(2),
            h: at(3),
            fill,
            stroke,
        })),
        "ellipse" => {
            let (rx, ry) = (at(2), if nums.len() > 3 { at(3) } else { at(2) });
            Some(Item::Shape(ShapeItem {
                kind: ShapeKind::Ellipse,
                x: x + at(0) - rx,
                y: y + at(1) - ry,
                w: rx * 2.0,
                h: ry * 2.0,
                fill,
                stroke,
            }))
        }
        "line" => Some(Item::Line(LineItem {
            from: (x + at(0), y + at(1)),
            to: (x + at(2), y + at(3)),
            color: stroke_color,
            width: stroke_width,
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mede como uma fonte monoespaçada: 0,6 x tamanho por caractere, mais o espaçamento.
    struct FakeMeasurer;

    impl TextMeasurer for FakeMeasurer {
        fn measure_text(&self, text: &str, style: &TextStyle) -> (f32, f32) {
            let per_char = style.size_px * 0.6 + style.spacing.map_or(0.0, |(a, b, _)| a + b);
            (text.chars().count() as f32 * per_char, style.size_px * 1.25)
        }
        fn image_size(&self, _path: &Path) -> Option<(f32, f32)> {
            Some((200.0, 100.0))
        }
    }

    fn friday_evening() -> DateTime {
        DateTime { year: 2026, month: 11, day: 13, weekday: 5, hour: 20, minute: 47, second: 5, day_of_year: 317 }
    }

    /// Monta uma pasta de skins de teste com a mesma estrutura da Mond (sem copiar a Mond).
    fn write_skin(name: &str, clock_ini: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("kage-skins-{}-{name}", std::process::id()));
        let clock = root.join("Relogio").join("Clock");
        let resources = root.join("Relogio").join("@Resources");
        std::fs::create_dir_all(&clock).unwrap();
        std::fs::create_dir_all(resources.join("Language")).unwrap();
        std::fs::write(resources.join("Variables.inc"), "[Variables]\nLanguage=Português\nFormat=H\nTextColor=255,255,255\nHidden=0\nHidden2=1\n").unwrap();
        // Arquivo de idioma em UTF-16, como os da Mond.
        let language = "[Variables]\nDate=\"Friday\":\"Sexta\",\"November\":\"Novembro\",\"Saturday\":\"Sábado\"\n";
        let mut bytes = vec![0xFF, 0xFE];
        for unit in language.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(resources.join("Language").join("Português.inc"), bytes).unwrap();
        let ini = clock.join("Clock.ini");
        std::fs::write(&ini, clock_ini).unwrap();
        (ini, root)
    }

    const CLOCK: &str = r#"[Rainmeter]
Update=1000
BackgroundMode=2
SolidColor=0,0,0,1

[Variables]
@include=#@#Variables.inc
@include2=#@#Language\#Language#.inc
Scale=0.86

[MeasureTime]
Measure=Time
Format="%#Format#:%M"

[MeasureDay]
Measure=Time
Format=%A
Substitute=#Date#

[MeasureDate]
Measure=Time
Format=%d  %B,  %Y.
Substitute=#Date#

[Meter24hClock]
Meter=String
MeasureName=MeasureTime
StringAlign=Center
StringCase=Upper
FontFace=Quicksand
FontColor=#TextColor#
FontSize=(14*#Scale#)
X=(340*#Scale#)
Y=(120*#Scale#)
Text="- %1 -"
AntiAlias=1
Hidden=#Hidden#

[Meter12hClock]
Meter=String
MeasureName=MeasureTime
Text="- %1 %2 -"
X=(340*#Scale#)
Y=(120*#Scale#)
Hidden=#Hidden2#

[MeterDay]
Meter=String
MeasureName=MeasureDay
StringAlign=Center
StringCase=Upper
FontFace=Anurati
FontSize=(40*#Scale#)
X=(340*#Scale#)
Y=(0*#Scale#)
Text="%1"
InlineSetting=CharacterSpacing | 10 | 10
AntiAlias=1
DynamicVariables=1

[MeterDate]
Meter=String
MeasureName=MeasureDate
StringAlign=Center
StringCase=Upper
FontFace=Quicksand
FontSize=(14*#Scale#)
X=(340*#Scale#)
Y=(75*#Scale#)
"#;

    fn texts(frame: &Frame) -> Vec<&TextItem> {
        frame.items.iter().filter_map(|i| if let Item::Text(t) = i { Some(t) } else { None }).collect()
    }

    #[test]
    fn mond_like_clock_renders_portuguese_texts() {
        let (ini, root) = write_skin("mond", CLOCK);
        let mut skin = Skin::load(&ini, &root).unwrap();
        assert_eq!(skin.config, "Relogio\\Clock");
        assert_eq!(skin.update_ms, Some(1000));
        assert!(skin.warnings.is_empty(), "{:?}", skin.warnings);

        let frame = skin.update(&friday_evening(), &FakeMeasurer, &crate::NoSystem);
        let t = texts(&frame);
        let strings: Vec<&str> = t.iter().map(|t| t.text.as_str()).collect();
        // O relógio de 12 h está escondido (Hidden2=1); a ordem é a do arquivo.
        assert_eq!(strings, vec!["- 20:47 -", "SEXTA", "13  NOVEMBRO,  2026."]);

        // Fonte em pontos vira pixels; a escala 0,86 entra nas fórmulas.
        let day = t[1];
        assert!((day.style.size_px - 40.0 * 0.86 * 4.0 / 3.0).abs() < 1e-3);
        assert_eq!(day.style.face, "Anurati");
        let spacing = 10.0 * 4.0 / 3.0;
        assert_eq!(day.style.spacing, Some((spacing, spacing, 0.0)));
        assert!(day.style.fonts_dir.as_ref().unwrap().ends_with("Relogio\\@Resources\\Fonts"));

        // Centralizado em X = 292,4: o retângulo do medidor fica meio para cada lado.
        let (w, _) = FakeMeasurer.measure_text(&day.text, &day.style);
        assert!((day.width - w).abs() < 1e-3);
        assert!((day.left + day.width / 2.0 - 292.4).abs() < 1e-3);
        assert_eq!(day.h_align, Align::Center);
        assert_eq!(frame.background, Some(Color { r: 0, g: 0, b: 0, a: 1 }));
        assert!(frame.width >= 292.4 + w / 2.0 - 0.5);
    }

    #[test]
    fn variables_can_change_at_runtime() {
        let (ini, root) = write_skin("scale", CLOCK);
        let mut skin = Skin::load(&ini, &root).unwrap();
        skin.set_variable("Scale", "1.06");
        let frame = skin.update(&friday_evening(), &FakeMeasurer, &crate::NoSystem);
        let day = texts(&frame)[1].clone();
        assert!((day.style.size_px - 40.0 * 1.06 * 4.0 / 3.0).abs() < 1e-3);
        assert_eq!(skin.variable("Language").as_deref(), Some("Português"));
    }

    #[test]
    fn relative_positions_styles_calc_and_shapes() {
        let ini_text = r#"[Rainmeter]
Update=500
[Style]
FontSize=12
FontColor=255,0,0
[MeasureHalf]
Measure=Calc
Formula=10 / 4
[MeasureUnknown]
Measure=Plugin
Plugin=WebParser
[First]
Meter=String
MeterStyle=Style
Text=AB
X=10
Y=20
[Second]
Meter=String
MeterStyle=Style
MeasureName=MeasureHalf
NumOfDecimals=1
Postfix=%
X=5R
Y=0r
[Box]
Meter=Shape
X=0
Y=100
Shape=Rectangle 0,0,(20*2),10,3 | Fill Color 0,0,255,128 | StrokeWidth 0
Shape2=Ellipse 50,5,5
[Gauge]
Meter=Bar
MeasureName=MeasureHalf
W=10
H=40
Y=200
"#;
        let (ini, root) = write_skin("relative", ini_text);
        let mut skin = Skin::load(&ini, &root).unwrap();
        assert_eq!(skin.update_ms, Some(500));
        assert_eq!(skin.warnings.len(), 1, "a medida WebParser ainda não é suportada");

        let frame = skin.update(&friday_evening(), &FakeMeasurer, &crate::NoSystem);
        let t = texts(&frame);
        assert_eq!(t[0].text, "AB");
        assert_eq!(t[0].color, Color { r: 255, g: 0, b: 0, a: 255 }, "cor herdada do MeterStyle");
        assert_eq!(t[1].text, "2.5%");
        let first_w = FakeMeasurer.measure_text("AB", &t[0].style).0;
        assert!((t[1].left - (10.0 + first_w + 5.0)).abs() < 1e-3, "5R = fim do anterior + 5");
        assert!((t[1].top - 20.0).abs() < 1e-3, "0r = mesma altura do anterior");

        let shapes: Vec<&ShapeItem> = frame.items.iter().filter_map(|i| if let Item::Shape(s) = i { Some(s) } else { None }).collect();
        assert_eq!(shapes[0].w, 40.0);
        assert_eq!(shapes[0].kind, ShapeKind::Rectangle { radius: 3.0 });
        assert_eq!(shapes[0].fill, Some(Color { r: 0, g: 0, b: 255, a: 128 }));
        assert_eq!(shapes[0].stroke, None);
        assert_eq!((shapes[1].x, shapes[1].y, shapes[1].w), (45.0, 100.0, 10.0));
        // A barra sem MinValue/MaxValue usa 0 a 1: 2,5 enche tudo.
        assert_eq!(shapes[2].h, 40.0);
    }

    #[test]
    fn hidden_meter_counts_with_zero_size_and_defaults_match_rainmeter() {
        let ini_text = r#"[Rainmeter]
AccurateText=1
[MeasureNum]
Measure=Calc
Formula=7/2
[Hidden]
Meter=String
Text=Escondido
X=500
Y=40
Hidden=1
[Next]
Meter=String
Text=Oi
X=10R
Y=0r
[Num]
Meter=String
MeasureName=MeasureNum
Scale=1.0
"#;
        let (ini, root) = write_skin("hidden", ini_text);
        let mut skin = Skin::load(&ini, &root).unwrap();
        let frame = skin.update(&friday_evening(), &FakeMeasurer, &crate::NoSystem);
        let t = texts(&frame);
        let strings: Vec<&str> = t.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(strings, vec!["Oi", "3.5"], "escondido não aparece; Scale com ponto dá 1 casa");
        // R depois de um escondido: âncora dele (500) + largura 0 + 10.
        assert_eq!((t[0].left, t[0].top), (510.0, 40.0));
        assert_eq!(t[0].color, Color::BLACK, "cor padrão do Rainmeter");
        assert_eq!(t[0].style.face, "Arial");
        assert_eq!(t[0].style.size_px, 10.0 * 4.0 / 3.0);
        assert!(!t[0].style.gdi_emulation, "AccurateText=1 desliga a imitação do GDI");
        assert!(!t[0].antialias, "AntiAlias padrão é 0");
        assert!(frame.width >= 510.0);
    }

    #[test]
    fn substitute_quoting_matches_rainmeter() {
        // As duas primeiras funcionam no Rainmeter 4.5; as duas últimas falham lá também.
        let ini_text = r#"[Rainmeter]
[A]
Measure=Time
Format=%A
Substitute="Friday":"Sexta"
[B]
Measure=Time
Format=%A
Substitute='Friday':"Sexta"
[C]
Measure=Time
Format=%A
Substitute='Friday':'Sexta'
[D]
Measure=Time
Format=%A
Substitute=Friday:Sexta
[MA]
Meter=String
MeasureName=A
[MB]
Meter=String
MeasureName=B
[MC]
Meter=String
MeasureName=C
[MD]
Meter=String
MeasureName=D
"#;
        let (ini, root) = write_skin("quotes", ini_text);
        let mut skin = Skin::load(&ini, &root).unwrap();
        let frame = skin.update(&friday_evening(), &FakeMeasurer, &crate::NoSystem);
        let strings: Vec<String> = texts(&frame).iter().map(|t| t.text.clone()).collect();
        assert_eq!(strings, vec!["Sexta", "Sexta", "Friday", "Friday"]);
    }

    struct FakeSystem;

    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    impl SystemProbe for FakeSystem {
        fn cpu_percent(&self) -> f64 {
            37.4
        }
        fn memory(&self) -> (f64, f64) {
            (4.0 * GIB, 16.0 * GIB)
        }
        fn swap(&self) -> (f64, f64) {
            (8.0 * GIB, 32.0 * GIB)
        }
        fn disk(&self, drive: &str) -> Option<(f64, f64)> {
            (drive.eq_ignore_ascii_case("C:")).then_some((100.0 * GIB, 500.0 * GIB))
        }
        fn network(&self) -> (f64, f64) {
            (2048.0, 1024.0)
        }
    }

    #[test]
    fn system_measures_like_illustro() {
        let ini_text = r#"[Rainmeter]
[measureCPU]
Measure=CPU
[measureRAM]
Measure=PhysicalMemory
[measureUsedDisk]
Measure=FreeDiskSpace
Drive=C:
InvertMeasure=1
[measureNet]
Measure=NetTotal
[mCPU]
Meter=String
MeasureName=measureCPU
Text=%1%
[mRAMPercent]
Meter=String
MeasureName=measureRAM
Percentual=1
[mRAMSize]
Meter=String
MeasureName=measureRAM
AutoScale=1
[mDisk]
Meter=String
MeasureName=measureUsedDisk
AutoScale=1
NumOfDecimals=0
[mNet]
Meter=String
MeasureName=measureNet
AutoScale=1k
[barCPU]
Meter=Bar
MeasureName=measureCPU
W=10
H=100
"#;
        let (ini, root) = write_skin("system", ini_text);
        let mut skin = Skin::load(&ini, &root).unwrap();
        assert!(skin.warnings.is_empty(), "{:?}", skin.warnings);
        assert!(skin.has_system_measures());
        let frame = skin.update(&friday_evening(), &FakeMeasurer, &FakeSystem);
        let strings: Vec<String> = texts(&frame).iter().map(|t| t.text.clone()).collect();
        assert_eq!(strings, vec!["37%", "25", "4.0 G", "400 G", "3.0 k"]);
        let bar = frame.items.iter().find_map(|i| if let Item::Shape(s) = i { Some(s) } else { None }).unwrap();
        assert!((bar.h - 37.4).abs() < 1e-3, "barra vertical com 37,4% de 100 px");
    }

    #[test]
    fn substitute_pairs_apply_in_order() {
        assert_eq!(apply_substitute("Friday", "\"Friday\":\"Sexta\",\"Sexta\":\"6a\"", false), "6a");
        assert_eq!(apply_substitute("13 November", "'November':'Novembro'", false), "13 Novembro");
        assert_eq!(proper_case("SEXTA feira"), "Sexta Feira");
    }

    /// Uma música tocando: 3:20 de duração, em 3:05, tocando.
    struct Playing;

    impl SystemProbe for Playing {
        fn now_playing(&self) -> Option<crate::system::NowPlaying> {
            Some(crate::system::NowPlaying {
                artist: "Artista".into(),
                title: "Musica".into(),
                album: "Disco".into(),
                position: 185.0,
                duration: 200.0,
                state: 1,
                cover: None,
            })
        }
        fn cpu_percent(&self) -> f64 {
            0.0
        }
        fn memory(&self) -> (f64, f64) {
            (0.0, 0.0)
        }
        fn swap(&self) -> (f64, f64) {
            (0.0, 0.0)
        }
        fn disk(&self, _drive: &str) -> Option<(f64, f64)> {
            None
        }
        fn network(&self) -> (f64, f64) {
            (0.0, 0.0)
        }
    }

    const PLAYER: &str = r#"[Rainmeter]
Update=100
LeftMouseDoubleClickAction=!ToggleConfig "Relogio\Settings" "Settings.ini"

[MeasureTitle]
Measure=Plugin
Plugin=NowPlaying.dll
PlayerName=Winamp
PlayerType=TITLE

[MeasureDuration]
Measure=Plugin
Plugin=NowPlaying.dll
PlayerType=DURATION

[MeasurePosition]
Measure=Plugin
Plugin=NowPlaying.dll
PlayerType=POSITION

[MeasureProgress]
Measure=Plugin
Plugin=NowPlaying.dll
PlayerType=PROGRESS

[MeasureState]
Measure=Plugin
Plugin=NowPlaying.dll
PlayerType=STATE
Substitute="0":"Play.png","1":"Pause.png","2":"Play.png"

[MeasureSeconds]
Measure=Calc
Formula=((MeasureDuration - MeasurePosition) % 60)
RegExpSubstitute=1
Substitute="^(.)$":"0\1"

[MeterTitle]
Meter=String
MeasureName=MeasureTitle
Text="%1"
FontSize=10

[MeterLeft]
Meter=String
MeasureName=MeasureDuration
MeasureName2=MeasureSeconds
Y=20
Text="%1 -0:%2"
FontSize=10

[MeterBar]
Meter=Shape
Y=50
Shape=Rectangle 0,0,([MeasureProgress]*2),4
LeftMouseUpAction=[!CommandMeasure "MeasureProgress" "SetPosition $MouseX:%$"]

[MeterPlayPause]
Meter=Image
ImageName=[MeasureState]
X=10
Y=70
W=40
H=40
SolidColor=0,0,0,1
LeftMouseUpAction=[!CommandMeasure "MeasureState" "PlayPause"][!SetVariable Clicks "(1+1)"]
"#;

    #[test]
    fn now_playing_measures_feed_texts_images_and_formulas() {
        let (ini, root) = write_skin("player", PLAYER);
        let mut skin = Skin::load(&ini, &root).unwrap();
        let frame = skin.update(&friday_evening(), &FakeMeasurer, &Playing);
        let texts: Vec<String> = frame.items.iter().filter_map(|i| if let Item::Text(t) = i { Some(t.text.clone()) } else { None }).collect();
        assert_eq!(texts, vec!["Musica", "3:20 -0:15"], "duração em m:ss e segundos restantes com 2 dígitos");
        let image = frame.items.iter().find_map(|i| if let Item::Image(m) = i { Some(m.path.clone()) } else { None }).unwrap();
        assert!(image.ends_with("Pause.png"), "tocando: o botão mostra Pausa ({})", image.display());
        let bar = frame.items.iter().find_map(|i| if let Item::Shape(s) = i { Some(s.w) } else { None }).unwrap();
        assert!((bar - 185.0).abs() < 0.01, "[MeasureProgress] = 92,5% vira largura 185 ({bar})");
        assert!(skin.is_now_playing("measurestate") && !skin.is_now_playing("MeasureSeconds"));
        assert!(skin.warnings.is_empty(), "nada de 'não suportada': {:?}", skin.warnings);
    }

    #[test]
    fn mouse_actions_come_from_the_meter_under_the_mouse_or_the_skin() {
        let (ini, root) = write_skin("player-mouse", PLAYER);
        let mut skin = Skin::load(&ini, &root).unwrap();
        skin.update(&friday_evening(), &FakeMeasurer, &Playing);
        let button = skin.mouse_action(MouseEvent::LeftUp, 30.0, 90.0).unwrap();
        assert_eq!(
            parse_actions(&button),
            vec![
                Action::Bang { name: "commandmeasure".into(), args: vec!["MeasureState".into(), "PlayPause".into()] },
                Action::Bang { name: "setvariable".into(), args: vec!["Clicks".into(), "(1+1)".into()] },
            ]
        );
        let seek = skin.mouse_action(MouseEvent::LeftUp, 100.0, 51.0).unwrap();
        assert!(seek.contains("SetPosition 54"), "100 de 185 px = 54% ({seek})");
        // Fora dos medidores: a ação da skin inteira; sem ação nenhuma: nada.
        let double = skin.mouse_action(MouseEvent::LeftDoubleClick, 150.0, 30.0).unwrap();
        assert_eq!(
            parse_actions(&double),
            vec![Action::Bang { name: "toggleconfig".into(), args: vec!["Relogio\\Settings".into(), "Settings.ini".into()] }]
        );
        assert_eq!(skin.mouse_action(MouseEvent::ScrollUp, 30.0, 90.0), None);
        assert!(skin.apply_bang("setvariable", &["Clicks".into(), "(1+1)".into()]));
        assert_eq!(skin.variable("Clicks").as_deref(), Some("2"), "valor entre parênteses é fórmula");
        assert!(!skin.apply_bang("toggleconfig", &[]), "o resto é com o motor");
    }

    #[test]
    fn actions_split_into_bangs_and_things_to_open() {
        assert_eq!(
            parse_actions(r#"[!PluginBang "MeasureStateButton Next"]["https://exemplo.com"]"#),
            vec![
                Action::Bang { name: "pluginbang".into(), args: vec!["MeasureStateButton Next".into()] },
                Action::Run("https://exemplo.com".into()),
            ]
        );
        assert_eq!(
            parse_actions(r#"!RainmeterActivateConfig Mond\Settings"#),
            vec![Action::Bang { name: "activateconfig".into(), args: vec!["Mond\\Settings".into()] }]
        );
        assert_eq!(
            parse_actions(r#"[!SetOption Meter Text """Diz "oi" [MeasureX]"""]"#),
            vec![Action::Bang { name: "setoption".into(), args: vec!["Meter".into(), "Text".into(), "Diz \"oi\" [MeasureX]".into()] }]
        );
        assert_eq!(mouse_variables("$MouseX$,$mousey$ $MouseX:%$", 50.4, 10.0, 200.0, 20.0), "50,10 25");
    }

    #[test]
    fn regexp_substitute_uses_backslash_groups() {
        // Mond\Player: "^(.)$":"0\1" deixa os segundos com dois dígitos.
        assert_eq!(apply_substitute("5", "\"^(.)$\":\"0\\1\"", true), "05");
        assert_eq!(apply_substitute("35", "\"^(.)$\":\"0\\1\"", true), "35");
        assert_eq!(apply_substitute("a$b", "\"b$\":\"$\"", true), "a$$", "o $ da substituição é literal");
    }
}
