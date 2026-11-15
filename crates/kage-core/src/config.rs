//! O `config.json` do Kage.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    #[default]
    Video,
    Image,
    Gif,
}

pub fn kind_of(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" => Some(MediaKind::Video),
        "png" | "jpg" | "jpeg" | "bmp" | "webp" | "tif" | "tiff" | "jxr" => Some(MediaKind::Image),
        "gif" => Some(MediaKind::Gif),
        _ => None,
    }
}

/// Extensões aceitas no seletor de arquivos.
pub const SUPPORTED_EXTENSIONS: &[&str] =
    &["mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff", "gif"];

/// Ajustes de um wallpaper (o "Customizar"). Tudo em percentual: 100 = como o arquivo é.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    /// Velocidade do vídeo ou do GIF.
    pub speed: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self { speed: 100.0, brightness: 100.0, contrast: 100.0, saturation: 100.0 }
    }
}

impl Look {
    /// Cores sem ajuste: o vídeo vai direto do decodificador para a tela, sem passo extra.
    pub fn colors_untouched(&self) -> bool {
        self.brightness == 100.0 && self.contrast == 100.0 && self.saturation == 100.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wallpaper {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub look: Look,
}

/// Como os wallpapers se distribuem pelos monitores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arrangement {
    /// Cada monitor com o próprio wallpaper.
    #[default]
    PerScreen,
    /// Um wallpaper esticado por todos os monitores.
    Span,
    /// O mesmo wallpaper em todos.
    Duplicate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Fill,
    Fit,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rule {
    Pause,
    Continue,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    PerScreen,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppAction {
    Pause,
    Ignore,
    Release,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppRule {
    /// Executável, como "obs64.exe".
    pub exe: String,
    pub action: AppAction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Performance {
    pub fullscreen: Rule,
    pub focus: Rule,
    pub battery: Rule,
    pub battery_saver: Rule,
    pub remote_desktop: Rule,
    pub scope: Scope,
    /// Segundos pausado até liberar o decodificador; `None` = nunca.
    pub release_after_secs: Option<u64>,
    pub apps: Vec<AppRule>,
}

impl Default for Performance {
    fn default() -> Self {
        Self {
            fullscreen: Rule::Pause,
            focus: Rule::Continue,
            battery: Rule::Pause,
            battery_saver: Rule::Pause,
            remote_desktop: Rule::Pause,
            scope: Scope::PerScreen,
            release_after_secs: Some(120),
            apps: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenAssignment {
    /// Nome do monitor no Windows, como `\\.\DISPLAY1`.
    pub device: String,
    pub wallpaper: Option<String>,
}

/// Onde a skin fica.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinAnchor {
    /// No X e Y escolhidos (como no Rainmeter).
    #[default]
    Free,
    /// Centralizada na horizontal num monitor, numa altura em percentual.
    Centered,
}

/// Camada da janela da skin (o `AlwaysOnTop` do Rainmeter, de -2 a 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinLayer {
    /// Logo acima dos ícones da área de trabalho, e continua visível no Win+D.
    #[default]
    Desktop,
    /// Logo acima da área de trabalho.
    Bottom,
    /// Como uma janela comum.
    Normal,
    /// Acima das janelas.
    Top,
    /// Acima de tudo, reafirmado de tempos em tempos.
    Topmost,
}

impl SkinLayer {
    /// A partir do `AlwaysOnTop` do Rainmeter.ini.
    pub fn from_rainmeter(value: i32) -> Self {
        match value {
            -2 => Self::Desktop,
            -1 => Self::Bottom,
            1 => Self::Top,
            2 => Self::Topmost,
            _ => Self::Normal,
        }
    }
}

impl SkinHover {
    /// A partir do `OnHover` do Rainmeter.ini.
    pub fn from_rainmeter(value: i32) -> Self {
        match value {
            1 => Self::Hide,
            2 => Self::FadeIn,
            3 => Self::FadeOut,
            _ => Self::None,
        }
    }
}

/// O que a skin faz com o mouse em cima (o `OnHover` do Rainmeter).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinHover {
    #[default]
    None,
    Hide,
    FadeIn,
    FadeOut,
}

/// Uma skin numa tela. A mesma skin pode estar em várias telas, cada uma com as próprias opções.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkinEntry {
    /// Identifica esta instância (a skin numa tela).
    pub id: String,
    /// `Mond\Clock`.
    pub config: String,
    /// Variante: um dos `.ini` da pasta.
    pub ini: PathBuf,
    /// Canto da skin na tela virtual (posição livre).
    pub x: f32,
    pub y: f32,
    pub active: bool,
    pub anchor: SkinAnchor,
    /// Monitor da skin (`\\.\DISPLAY1`); sem monitor, o principal.
    pub monitor: Option<String>,
    /// Altura do centro da skin, em percentual da altura do monitor (50 = centro).
    pub height_percent: f32,
    /// Opacidade de 1 a 255 (o `AlphaValue` do Rainmeter).
    pub alpha: u8,
    pub layer: SkinLayer,
    pub on_hover: SkinHover,
    pub click_through: bool,
    pub draggable: bool,
    pub keep_on_screen: bool,
    pub save_position: bool,
    pub snap_edges: bool,
}

impl Default for SkinEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            config: String::new(),
            ini: PathBuf::new(),
            x: 0.0,
            y: 0.0,
            active: false,
            anchor: SkinAnchor::Free,
            monitor: None,
            height_percent: 50.0,
            alpha: 255,
            layer: SkinLayer::Desktop,
            on_hover: SkinHover::None,
            click_through: false,
            draggable: true,
            keep_on_screen: true,
            save_position: true,
            snap_edges: true,
        }
    }
}

/// Quantas skins podem ficar ligadas ao mesmo tempo, somando todas as telas.
pub const MAX_ACTIVE_SKINS: usize = 3;

/// A mesma skin em pacotes copiados para outra tela (`Mond` e `Mond2`) tem a mesma chave:
/// `mond\clock`. Os números no fim do nome do pacote saem.
pub fn skin_key(config: &str) -> String {
    let lower = config.to_lowercase().replace('/', "\\");
    match lower.split_once('\\') {
        Some((suite, rest)) => format!("{}\\{rest}", suite.trim_end_matches(|c: char| c.is_ascii_digit())),
        None => lower.trim_end_matches(|c: char| c.is_ascii_digit()).to_string(),
    }
}

/// Id novo para uma skin posta numa tela.
pub fn new_skin_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Tema da janela.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Segue o modo claro ou escuro do Windows.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Skins {
    pub enabled: bool,
    pub items: Vec<SkinEntry>,
    /// Chaves (`skin_key`) das skins marcadas como favoritas.
    pub favorites: Vec<String>,
    /// Chaves das skins tiradas da lista do Kage. Os arquivos continuam na pasta da skin.
    pub hidden: Vec<String>,
}

/// Fundo da janela do Kage (diálogo "Tema").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backdrop {
    /// Cor lisa do tema.
    #[default]
    Default,
    /// O wallpaper que está na tela, desfocado.
    Dynamic,
    /// Uma imagem escolhida.
    Image(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub start_with_windows: bool,
    pub theme: ThemeChoice,
    /// Com mais de um monitor, perguntar onde colocar o wallpaper clicado.
    pub ask_monitor: bool,
    pub backdrop: Backdrop,
    /// Imagens de fundo que o usuário já abriu, para escolher de novo.
    pub backdrops: Vec<PathBuf>,
    /// Ícone na bandeja do sistema (o motor continua rodando sem ele).
    pub tray_icon: bool,
    /// Procurar versão nova no GitHub (janela ao abrir, motor de tempos em tempos).
    pub check_updates: bool,
    /// Última versão cujas novidades a janela já mostrou.
    pub seen_version: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            theme: ThemeChoice::System,
            ask_monitor: true,
            backdrop: Backdrop::Default,
            backdrops: Vec::new(),
            tray_icon: true,
            check_updates: true,
            seen_version: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub library: Vec<Wallpaper>,
    pub arrangement: Arrangement,
    pub fit: Fit,
    /// Wallpaper de cada monitor (disposição Tela).
    pub screens: Vec<ScreenAssignment>,
    /// Wallpaper das disposições Span e Duplicar.
    pub shared: Option<String>,
    pub performance: Performance,
    pub skins: Skins,
    pub general: General,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            library: Vec::new(),
            arrangement: Arrangement::PerScreen,
            fit: Fit::Fill,
            screens: Vec::new(),
            shared: None,
            performance: Performance::default(),
            skins: Skins::default(),
            general: General::default(),
        }
    }
}

impl Config {
    /// Lê o arquivo; sem arquivo, a configuração padrão.
    pub fn load_from(path: &Path) -> Result<Self, String> {
        let mut config: Self = match std::fs::read_to_string(path) {
            // Editores do Windows (e o PowerShell 5) gravam UTF-8 com BOM: o serde não aceita.
            Ok(text) => serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
                .map_err(|e| format!("config.json inválido: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(format!("não abriu {}: {e}", path.display())),
        };
        config.ensure_skin_ids();
        Ok(config)
    }

    /// Lê do disco, muda e grava de volta. É como o motor guarda o que o usuário fez direto
    /// na skin (arrastar, menu do clique direito) sem apagar o que a janela gravou.
    pub fn edit_file(path: &Path, change: impl FnOnce(&mut Self)) -> Result<(), String> {
        let mut config = Self::load_from(path)?;
        change(&mut config);
        config.save_to(path)
    }

    /// Skins de arquivos antigos, sem id: o id sai do nome, do `.ini` e da ordem, igual em
    /// quem ler o arquivo (janela e motor), até a janela gravar de novo.
    pub fn ensure_skin_ids(&mut self) {
        for (index, entry) in self.skins.items.iter_mut().enumerate() {
            if entry.id.is_empty() {
                let mut hasher = DefaultHasher::new();
                entry.config.to_lowercase().hash(&mut hasher);
                entry.ini.to_string_lossy().to_lowercase().hash(&mut hasher);
                index.hash(&mut hasher);
                entry.id = format!("{:016x}", hasher.finish());
            }
        }
    }

    pub fn skin(&self, id: &str) -> Option<&SkinEntry> {
        self.skins.items.iter().find(|e| e.id == id)
    }

    pub fn skin_mut(&mut self, id: &str) -> Option<&mut SkinEntry> {
        self.skins.items.iter_mut().find(|e| e.id == id)
    }

    pub fn is_favorite(&self, key: &str) -> bool {
        self.skins.favorites.iter().any(|f| f == key)
    }

    pub fn set_favorite(&mut self, key: &str, favorite: bool) {
        self.skins.favorites.retain(|f| f != key);
        if favorite {
            self.skins.favorites.push(key.to_string());
        }
    }

    pub fn is_hidden(&self, key: &str) -> bool {
        self.skins.hidden.iter().any(|h| h == key)
    }

    /// Tira a skin da lista do Kage (e de todas as telas) ou devolve à lista.
    pub fn set_hidden(&mut self, key: &str, hidden: bool) {
        self.skins.hidden.retain(|h| h != key);
        if hidden {
            self.skins.hidden.push(key.to_string());
            for e in self.skins.items.iter_mut().filter(|e| skin_key(&e.config) == key) {
                e.active = false;
            }
        }
    }

    pub fn load() -> Self {
        Self::load_from(&crate::config_path()).unwrap_or_default()
    }

    /// Grava num arquivo temporário e troca de nome: o motor nunca lê um arquivo pela metade.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, path).map_err(|e| e.to_string())
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&crate::config_path())
    }

    pub fn wallpaper(&self, id: &str) -> Option<&Wallpaper> {
        self.library.iter().find(|w| w.id == id)
    }

    /// Acrescenta um arquivo à biblioteca. Se já estiver lá, devolve o existente. Confere também
    /// o caminho: um arquivo que mudou de pasta guarda o id antigo (e o monitor onde estava).
    pub fn add_file(&mut self, path: &Path) -> Option<&Wallpaper> {
        let kind = kind_of(path)?;
        let id = wallpaper_id(path);
        let same_path = |w: &Wallpaper| w.path.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy());
        if let Some(existing) = self.library.iter().position(|w| w.id == id || same_path(w)) {
            return self.library.get(existing);
        }
        self.library.push(Wallpaper {
            title: path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            path: path.to_path_buf(),
            kind,
            look: Look::default(),
            id: id.clone(),
        });
        self.wallpaper(&id)
    }

    /// Tira da biblioteca e de qualquer monitor que o use.
    pub fn remove(&mut self, id: &str) {
        self.library.retain(|w| w.id != id);
        for s in &mut self.screens {
            if s.wallpaper.as_deref() == Some(id) {
                s.wallpaper = None;
            }
        }
        if self.shared.as_deref() == Some(id) {
            self.shared = None;
        }
    }

    pub fn screen_wallpaper(&self, device: &str) -> Option<&str> {
        self.screens.iter().find(|s| s.device.eq_ignore_ascii_case(device))?.wallpaper.as_deref()
    }

    /// Põe o wallpaper num monitor (disposição Tela).
    pub fn assign(&mut self, device: &str, id: Option<&str>) {
        let id = id.map(str::to_string);
        match self.screens.iter_mut().find(|s| s.device.eq_ignore_ascii_case(device)) {
            Some(s) => s.wallpaper = id,
            None => self.screens.push(ScreenAssignment { device: device.to_string(), wallpaper: id }),
        }
    }

    /// Coloca o wallpaper só em um monitor. Vindo de Span ou Duplicar, os outros monitores
    /// ficam com o que já mostravam.
    pub fn put_on_screen(&mut self, devices: &[String], device: &str, id: &str) {
        if self.arrangement != Arrangement::PerScreen {
            if let Some(shared) = self.shared.clone() {
                for d in devices {
                    self.assign(d, Some(&shared));
                }
            }
            self.arrangement = Arrangement::PerScreen;
        }
        self.assign(device, Some(id));
    }

    /// Fecha todos os wallpapers (volta o papel de parede normal do Windows).
    pub fn clear_wallpapers(&mut self) {
        self.screens.iter_mut().for_each(|s| s.wallpaper = None);
        self.shared = None;
    }

    /// O wallpaper na tela agora: o de Span e Duplicar, ou o do primeiro monitor que tem um.
    pub fn current_wallpaper(&self) -> Option<&str> {
        let id = match self.arrangement {
            Arrangement::PerScreen => self.screens.iter().find_map(|s| s.wallpaper.as_deref()),
            _ => self.shared.as_deref(),
        }?;
        self.wallpaper(id).map(|w| w.id.as_str())
    }

    /// "Alterar wallpaper" da bandeja: um wallpaper sorteado da biblioteca em cada monitor
    /// (disposição Tela) ou no de Span e Duplicar, diferente do que estava quando dá.
    /// `seed` vem do relógio. Sem nada na biblioteca, não muda nada e devolve falso.
    pub fn random_wallpapers(&mut self, devices: &[String], seed: u64) -> bool {
        if self.library.is_empty() {
            return false;
        }
        let mut state = seed | 1;
        let mut pick = |current: Option<&str>| -> String {
            // xorshift64: basta para sortear sem depender de crate.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let others: Vec<&Wallpaper> = self.library.iter().filter(|w| Some(w.id.as_str()) != current).collect();
            let pool = if others.is_empty() { self.library.iter().collect() } else { others };
            pool[(state % pool.len() as u64) as usize].id.clone()
        };
        match self.arrangement {
            Arrangement::PerScreen => {
                let chosen: Vec<(String, String)> =
                    devices.iter().map(|d| (d.clone(), pick(self.screen_wallpaper(d)))).collect();
                for (device, id) in chosen {
                    self.assign(&device, Some(&id));
                }
            }
            _ => self.shared = Some(pick(self.shared.as_deref())),
        }
        true
    }
}

/// Identificador estável a partir do caminho (o mesmo arquivo não entra duas vezes).
pub fn wallpaper_id(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_wallpaper_changes_what_is_on_screen() {
        let mut cfg = Config::default();
        assert!(!cfg.random_wallpapers(&["A".into()], 7), "biblioteca vazia não muda nada");
        let a = cfg.add_file(Path::new(r"C:a.mp4")).unwrap().id.clone();
        let b = cfg.add_file(Path::new(r"C:.mp4")).unwrap().id.clone();
        cfg.assign("A", Some(&a));
        for seed in 1..20 {
            cfg.assign("A", Some(&a));
            cfg.random_wallpapers(&["A".into(), "B".into()], seed);
            assert_eq!(cfg.screen_wallpaper("A"), Some(b.as_str()), "sempre um diferente do que estava");
            assert!(cfg.screen_wallpaper("B").is_some(), "monitor sem wallpaper também ganha um");
        }
        assert_eq!(cfg.current_wallpaper(), Some(b.as_str()));
        cfg.arrangement = Arrangement::Duplicate;
        cfg.shared = Some(b.clone());
        cfg.random_wallpapers(&[], 3);
        assert_eq!(cfg.shared.as_deref(), Some(a.as_str()));
        cfg.clear_wallpapers();
        assert_eq!(cfg.current_wallpaper(), None);
    }

    #[test]
    fn a_file_saved_with_bom_still_opens() {
        let dir = std::env::temp_dir().join(format!("kage-core-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut cfg = Config::default();
        cfg.general.seen_version = "0.1.0".into();
        std::fs::write(&path, format!("\u{feff}{}", serde_json::to_string(&cfg).unwrap())).unwrap();
        assert_eq!(Config::load_from(&path).unwrap().general.seen_version, "0.1.0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_gives_defaults_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("kage-core-{}", std::process::id()));
        let path = dir.join("config.json");
        let _ = std::fs::remove_file(&path);
        let mut cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg, Config::default());
        assert_eq!(cfg.performance.release_after_secs, Some(120));

        let video = PathBuf::from(r"C:\Videos\Montanha.MP4");
        let id = cfg.add_file(&video).unwrap().id.clone();
        assert_eq!(cfg.add_file(&video).unwrap().id, id, "o mesmo arquivo não duplica");
        assert_eq!(cfg.library.len(), 1);
        assert_eq!(cfg.library[0].title, "Montanha");
        assert_eq!(cfg.library[0].kind, MediaKind::Video);
        assert!(cfg.add_file(Path::new("leia.txt")).is_none());

        cfg.assign(r"\\.\DISPLAY1", Some(&id));
        cfg.arrangement = Arrangement::Span;
        cfg.shared = Some(id.clone());
        cfg.save_to(&path).unwrap();
        let back = Config::load_from(&path).unwrap();
        assert_eq!(back, cfg);
        assert_eq!(back.screen_wallpaper(r"\\.\display1"), Some(id.as_str()));
    }

    #[test]
    fn remove_clears_assignments() {
        let mut cfg = Config::default();
        let id = cfg.add_file(Path::new(r"C:\a.gif")).unwrap().id.clone();
        cfg.assign(r"\\.\DISPLAY2", Some(&id));
        cfg.shared = Some(id.clone());
        cfg.remove(&id);
        assert!(cfg.library.is_empty());
        assert_eq!(cfg.screen_wallpaper(r"\\.\DISPLAY2"), None);
        assert_eq!(cfg.shared, None);
    }

    #[test]
    fn choosing_one_screen_after_duplicate_keeps_the_others() {
        let mut cfg = Config::default();
        let a = cfg.add_file(Path::new(r"C:\a.mp4")).unwrap().id.clone();
        let b = cfg.add_file(Path::new(r"C:\b.mp4")).unwrap().id.clone();
        cfg.arrangement = Arrangement::Duplicate;
        cfg.shared = Some(a.clone());
        let devices = vec![r"\\.\DISPLAY1".to_string(), r"\\.\DISPLAY2".to_string()];
        cfg.put_on_screen(&devices, r"\\.\DISPLAY1", &b);
        assert_eq!(cfg.arrangement, Arrangement::PerScreen);
        assert_eq!(cfg.screen_wallpaper(r"\\.\DISPLAY1"), Some(b.as_str()));
        assert_eq!(cfg.screen_wallpaper(r"\\.\DISPLAY2"), Some(a.as_str()), "o monitor 2 continua com o que mostrava");
        assert!(cfg.general.ask_monitor);
    }

    #[test]
    fn skin_entries_default_to_free_and_center_height() {
        let cfg: Config = serde_json::from_str(r#"{ "skins": { "items": [ { "config": "Mond\\Clock", "x": 984, "y": 300 } ] } }"#).unwrap();
        let entry = &cfg.skins.items[0];
        assert_eq!(entry.anchor, SkinAnchor::Free);
        assert_eq!(entry.height_percent, 50.0);
        assert_eq!((entry.x, entry.y), (984.0, 300.0));
        assert_eq!(cfg.general.theme, ThemeChoice::System);
    }

    #[test]
    fn copied_suites_share_the_same_skin_key() {
        assert_eq!(skin_key("Mond\\Clock"), "mond\\clock");
        assert_eq!(skin_key("Mond2\\Clock"), "mond\\clock");
        assert_eq!(skin_key("illustro\\Disk"), "illustro\\disk");
        assert_eq!(skin_key("Pacote 3\\Sub2\\Relogio"), "pacote \\sub2\\relogio", "só o nome do pacote perde o número");
    }

    #[test]
    fn old_skins_get_the_same_id_everywhere_and_rainmeter_defaults() {
        let text = r#"{ "skins": { "items": [ { "config": "Mond\\Clock", "ini": "C:\\a.ini" }, { "config": "Mond2\\Clock" } ] } }"#;
        let dir = std::env::temp_dir().join(format!("kage-core-ids-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, text).unwrap();
        let one = Config::load_from(&path).unwrap();
        let two = Config::load_from(&path).unwrap();
        assert_eq!(one.skins.items[0].id, two.skins.items[0].id, "janela e motor calculam o mesmo id");
        assert_ne!(one.skins.items[0].id, one.skins.items[1].id);
        let entry = &one.skins.items[0];
        assert_eq!((entry.alpha, entry.layer, entry.on_hover), (255, SkinLayer::Desktop, SkinHover::None));
        assert!(entry.draggable && entry.keep_on_screen && entry.save_position && entry.snap_edges && !entry.click_through);

        let id = entry.id.clone();
        Config::edit_file(&path, |c| c.skin_mut(&id).unwrap().x = 42.0).unwrap();
        assert_eq!(Config::load_from(&path).unwrap().skin(&id).unwrap().x, 42.0);
        assert_ne!(new_skin_id(), new_skin_id());
    }

    #[test]
    fn hiding_a_skin_turns_off_every_screen_and_can_be_undone() {
        let mut cfg: Config = serde_json::from_str(
            r#"{ "skins": { "items": [
                { "config": "Mond\\Weather", "active": true },
                { "config": "Mond2\\Weather", "active": true },
                { "config": "Mond\\Clock", "active": true } ] } }"#,
        )
        .unwrap();
        cfg.set_hidden("mond\\weather", true);
        assert!(cfg.is_hidden("mond\\weather"));
        assert!(!cfg.skins.items[0].active && !cfg.skins.items[1].active, "sai de todas as telas");
        assert!(cfg.skins.items[2].active, "as outras skins não mudam");
        cfg.set_hidden("mond\\weather", false);
        assert!(!cfg.is_hidden("mond\\weather"));
    }

    #[test]
    fn look_and_backdrop_round_trip() {
        let mut cfg = Config::default();
        cfg.add_file(Path::new(r"C:\v.mp4"));
        assert!(cfg.library[0].look.colors_untouched());
        cfg.library[0].look.saturation = 140.0;
        cfg.general.backdrop = Backdrop::Image(PathBuf::from(r"C:\fundo.png"));
        cfg.set_favorite("mond\\clock", true);
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back, cfg);
        assert!(!back.library[0].look.colors_untouched());
        assert!(back.is_favorite("mond\\clock"));
    }

    #[test]
    fn old_or_partial_files_still_load() {
        let cfg: Config = serde_json::from_str(r#"{ "arrangement": "duplicate", "performance": { "focus": "pause" } }"#).unwrap();
        assert_eq!(cfg.arrangement, Arrangement::Duplicate);
        assert_eq!(cfg.performance.focus, Rule::Pause);
        assert_eq!(cfg.performance.fullscreen, Rule::Pause, "o que falta vem do padrão");
    }
}
