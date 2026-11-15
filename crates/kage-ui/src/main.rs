//! Janela do Kage: biblioteca, skins e configurações.
//!
//! Ela só edita o `config.json` e avisa o motor; quem toca o wallpaper e abre as skins é o
//! `kage-engine`. Quando o motor grava algo (skin arrastada, menu do clique direito), a janela
//! relê o arquivo. Fechada, a janela não gasta nada.

#![windows_subsystem = "windows"]

mod engine;
mod frame;
mod monitors;
mod notes;
mod single;
mod startup;
mod theme;
mod update;

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use kage_core::{
    AppAction, AppRule, Arrangement, Backdrop, Config, Fit, Look, MAX_ACTIVE_SKINS, MediaKind, Rule, Scope,
    SkinAnchor, SkinEntry, SkinHover, SkinLayer, ThemeChoice, Wallpaper, skin_key,
};
use slint::{ComponentHandle, Image, ModelRc, SharedString, Timer, TimerMode, VecModel};

slint::include_modules!();

/// Uma skin instalada. Pacotes copiados para outra tela (`Mond2`) entram como a mesma skin.
#[derive(Clone)]
struct SkinGroup {
    key: String,
    name: String,
    suite: String,
    config: String,
    ini: PathBuf,
}

struct State {
    config: Config,
    monitors: Vec<monitors::Monitor>,
    /// Último monitor escolhido.
    selected: usize,
    groups: Vec<SkinGroup>,
    /// 0 todas, 1 ligadas, 2 favoritas, 3 em diante: um pacote.
    skin_filter: i32,
    /// Skin aberta no diálogo das telas ou nas configurações.
    skin_open: Option<String>,
    /// Qual das telas dela está nas configurações.
    skin_instance: usize,
    /// 0 todos, 1 vídeos, 2 imagens, 3 GIFs.
    filter: i32,
    search: String,
    /// Wallpaper clicado, esperando a escolha do monitor.
    pending: Option<String>,
    /// Wallpaper do menu do card (Sobre, Customizar, Editar).
    menu_target: Option<String>,
    /// Data do config.json na última leitura ou gravação desta janela.
    known_mtime: Option<SystemTime>,
}

type Shared = Rc<RefCell<State>>;

/// Opções de "liberar memória quando pausado", na ordem da caixa de seleção.
const RELEASE_CHOICES: [Option<u64>; 6] = [None, Some(30), Some(60), Some(120), Some(300), Some(600)];
/// Camadas e "passar o mouse" na ordem das listas da janela.
const LAYERS: [SkinLayer; 5] = [SkinLayer::Topmost, SkinLayer::Top, SkinLayer::Normal, SkinLayer::Bottom, SkinLayer::Desktop];
const HOVERS: [SkinHover; 4] = [SkinHover::None, SkinHover::Hide, SkinHover::FadeIn, SkinHover::FadeOut];

fn main() -> Result<(), slint::PlatformError> {
    // Já aberta (pelo Menu Iniciar ou pela bandeja): a janela que existe aparece com o pedido.
    let command = single::command_from_args();
    if single::hand_over(command) {
        return Ok(());
    }
    let ui = App::new()?;
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    let state: Shared = Rc::new(RefCell::new(load_state()));
    if !engine::is_running() {
        let _ = engine::start();
    }
    refresh(&ui, &state.borrow());
    spawn_thumbnails(&ui, &state.borrow());
    spawn_skin_previews(&ui, &state.borrow());
    spawn_backdrop(&ui, &state.borrow());

    let toast_timer = Rc::new(Timer::default());
    wire(&ui, &state, &toast_timer);
    wire_updates(&ui, &state, &toast_timer);

    // Atualização: limpa a troca anterior, mostra as novidades da versão recém-instalada e
    // procura versão nova alguns segundos depois de abrir.
    update::clean_leftovers();
    ui.set_release_notes(release_notes_model());
    show_news_once(&ui, &state);
    run_ui_command(&ui, &state, &toast_timer, command);
    let first_check = Timer::default();
    if state.borrow().config.general.check_updates {
        let weak = ui.as_weak();
        let timer = toast_timer.clone();
        first_check.start(TimerMode::SingleShot, Duration::from_secs(4), move || {
            if let Some(ui) = weak.upgrade() {
                start_update_check(&ui, &timer, false);
            }
        });
    }

    // A cada 2 s: estado do motor, tema do Windows e o config.json gravado pelo motor.
    let status_timer = Timer::default();
    let weak = ui.as_weak();
    let watched = state.clone();
    status_timer.start(TimerMode::Repeated, Duration::from_secs(2), move || {
        let Some(ui) = weak.upgrade() else { return };
        update_engine_status(&ui);
        let changed_on_disk = {
            let s = watched.borrow();
            config_mtime() != s.known_mtime
        };
        if changed_on_disk {
            let mut s = watched.borrow_mut();
            if let Ok(config) = Config::load_from(&kage_core::config_path()) {
                s.config = config;
            }
            s.known_mtime = config_mtime();
            drop(s);
            refresh(&ui, &watched.borrow());
        }
        ui.set_dark(theme::is_dark(watched.borrow().config.general.theme));
    });
    update_engine_status(&ui);
    // A janela do winit só existe depois que o laço de eventos começa: aí entram a sombra e o
    // aviso de maximizado (Win+seta, arrastar até o topo) para o ícone do botão acompanhar.
    let weak = ui.as_weak();
    let (single_state, single_timer) = (state.clone(), toast_timer.clone());
    let _ = slint::spawn_local(async move {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
        let Some(ui) = weak.upgrade() else { return };
        if ui.window().winit_window().await.is_err() {
            return;
        }
        frame::add_shadow(ui.window());
        ui.set_is_maximized(ui.window().is_maximized());
        let commands = (ui.as_weak(), single_state.clone(), single_timer.clone());
        single::install(ui.window(), move |command| {
            if let Some(ui) = commands.0.upgrade() {
                run_ui_command(&ui, &commands.1, &commands.2, command);
            }
        });
        let weak = ui.as_weak();
        ui.window().on_winit_window_event(move |window, event| {
            if matches!(event, WindowEvent::Resized(_))
                && let Some(ui) = weak.upgrade()
            {
                ui.set_is_maximized(window.is_maximized());
            }
            EventResult::Propagate
        });
    });
    // Fechar a janela encerra o processo (sem isso o Slint só a esconde). O motor continua.
    ui.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    ui.run()
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(kage_core::config_path()).and_then(|m| m.modified()).ok()
}

fn default_skins_path() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("Documents").join("Rainmeter").join("Skins")
}

/// Agrupa as skins instaladas pela chave: `Mond\Clock` e `Mond2\Clock` viram "Clock" do pacote "Mond".
fn skin_groups(installed: Vec<(String, PathBuf)>) -> Vec<SkinGroup> {
    let mut groups: Vec<SkinGroup> = Vec::new();
    for (config, ini) in installed {
        let key = skin_key(&config);
        let original_suite = config.split('\\').next().unwrap_or(&config).to_string();
        let copy = original_suite.ends_with(|c: char| c.is_ascii_digit());
        match groups.iter_mut().find(|g| g.key == key) {
            // Fica com o pacote original (sem número no fim) quando ele existe.
            Some(group) if !copy => {
                group.config = config.clone();
                group.ini = ini;
                group.suite = original_suite;
            }
            Some(_) => {}
            None => groups.push(SkinGroup {
                name: config.rsplit('\\').next().unwrap_or(&config).to_string(),
                suite: original_suite.trim_end_matches(|c: char| c.is_ascii_digit()).trim().to_string(),
                key,
                config,
                ini,
            }),
        }
    }
    groups.sort_by_key(|g| (g.suite.to_lowercase(), g.name.to_lowercase()));
    groups
}

/// Lê o config.json. Se ele existir e não abrir, é posto de lado (`config.ilegivel-<data>.json`)
/// antes de a janela começar do zero: gravar por cima apagaria a configuração do usuário.
fn load_config_or_set_aside() -> Config {
    let path = kage_core::config_path();
    match Config::load_from(&path) {
        Ok(config) => config,
        Err(_) => {
            let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let _ = std::fs::rename(&path, path.with_file_name(format!("config.ilegivel-{stamp}.json")));
            Config::default()
        }
    }
}

fn load_state() -> State {
    let mut config = load_config_or_set_aside();
    let setup = kage_skins::rainmeter::default_ini().and_then(|p| kage_skins::rainmeter::read(&p).ok());
    let skins_path = setup.as_ref().map(|s| s.skins_path.clone()).unwrap_or_else(default_skins_path);
    let monitors = monitors::enumerate();
    // Primeira vez: já traz as skins ativas do Rainmeter nas mesmas posições; elas ficam
    // desligadas até o usuário ligar o desenho das skins.
    if config.skins.items.is_empty()
        && let Some(setup) = &setup
    {
        config.skins.items = setup
            .active
            .iter()
            .take(MAX_ACTIVE_SKINS)
            .map(|a| entry_from_rainmeter(a, &monitors))
            .collect();
    }
    let selected = monitors.iter().position(|m| m.primary).unwrap_or(0);
    let groups = skin_groups(kage_skins::rainmeter::installed(&skins_path));
    State {
        config,
        monitors,
        selected,
        groups,
        skin_filter: 0,
        skin_open: None,
        skin_instance: 0,
        filter: 0,
        search: String::new(),
        pending: None,
        menu_target: None,
        known_mtime: config_mtime(),
    }
}

fn entry_from_rainmeter(a: &kage_skins::rainmeter::ActiveSkin, monitors: &[monitors::Monitor]) -> SkinEntry {
    SkinEntry {
        id: kage_core::new_skin_id(),
        config: a.config.clone(),
        ini: a.ini.clone(),
        x: a.x,
        y: a.y,
        active: true,
        anchor: SkinAnchor::Free,
        monitor: monitor_at(monitors, a.x as i32, a.y as i32).map(|m| m.device.clone()),
        alpha: a.options.alpha,
        layer: SkinLayer::from_rainmeter(a.options.always_on_top),
        on_hover: SkinHover::from_rainmeter(a.options.on_hover),
        click_through: a.options.click_through,
        draggable: a.options.draggable,
        keep_on_screen: a.options.keep_on_screen,
        save_position: a.options.save_position,
        snap_edges: a.options.snap_edges,
        ..Default::default()
    }
}

fn monitor_at(monitors: &[monitors::Monitor], x: i32, y: i32) -> Option<&monitors::Monitor> {
    monitors.iter().find(|m| x >= m.rect.left && x < m.rect.right && y >= m.rect.top && y < m.rect.bottom)
}

// ---------- imagens (com cache: o refresh roda a cada ação) ----------

thread_local! {
    static IMAGES: RefCell<HashMap<PathBuf, (SystemTime, Image)>> = RefCell::default();
}

fn load_image(path: &Path) -> Option<Image> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    if let Some(image) = IMAGES.with_borrow(|c| c.get(path).filter(|(t, _)| *t == modified).map(|(_, i)| i.clone())) {
        return Some(image);
    }
    let image = Image::load_from_path(path).ok()?;
    IMAGES.with_borrow_mut(|c| c.insert(path.to_path_buf(), (modified, image.clone())));
    Some(image)
}

fn image_or_empty(path: Option<PathBuf>) -> (Image, bool) {
    match path.and_then(|p| load_image(&p)) {
        Some(image) => (image, true),
        None => (Image::default(), false),
    }
}

/// Miniatura gerada pelo motor na pasta do Kage.
fn thumbnail_path(w: &Wallpaper) -> Option<PathBuf> {
    let generated = kage_core::thumbnails_dir().join(format!("{}.jpg", w.id));
    generated.is_file().then_some(generated)
}

fn thumbnail(w: &Wallpaper) -> (Image, bool) {
    image_or_empty(thumbnail_path(w))
}

fn skin_preview_path(key: &str) -> PathBuf {
    kage_core::thumbnails_dir().join("skins").join(format!("{}.png", kage_core::wallpaper_id(Path::new(key))))
}

/// Versão minúscula (desfocada) de uma imagem, para o fundo da janela.
fn blurred_path(source: &Path) -> PathBuf {
    kage_core::thumbnails_dir().join("blur").join(format!("{}.jpg", kage_core::wallpaper_id(source)))
}

fn small_path(source: &Path) -> PathBuf {
    kage_core::thumbnails_dir().join("backdrops").join(format!("{}.jpg", kage_core::wallpaper_id(source)))
}

/// Imagem por trás do fundo atual: o wallpaper da tela (Dinâmico) ou a escolhida.
fn backdrop_source(s: &State) -> Option<PathBuf> {
    match &s.config.general.backdrop {
        Backdrop::Default => None,
        Backdrop::Dynamic => used_ids(s).first().and_then(|id| s.config.wallpaper(id)).and_then(thumbnail_path),
        Backdrop::Image(path) => Some(path.clone()),
    }
}

fn kind_label(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "VÍDEO",
        MediaKind::Image => "IMAGEM",
        MediaKind::Gif => "GIF",
    }
}

fn kind_matches(filter: i32, kind: MediaKind) -> bool {
    match filter {
        1 => kind == MediaKind::Video,
        2 => kind == MediaKind::Image,
        3 => kind == MediaKind::Gif,
        _ => true,
    }
}

/// Wallpapers que estão na tela agora.
fn used_ids(s: &State) -> Vec<String> {
    match s.config.arrangement {
        Arrangement::PerScreen => s
            .monitors
            .iter()
            .filter_map(|m| s.config.screen_wallpaper(&m.device).map(str::to_string))
            .collect(),
        _ => s.config.shared.iter().cloned().collect(),
    }
}

fn devices(s: &State) -> Vec<String> {
    s.monitors.iter().map(|m| m.device.clone()).collect()
}

fn title_of(s: &State, id: &str) -> String {
    s.config.wallpaper(id).map(|w| w.title.clone()).unwrap_or_default()
}

/// Aplica sem perguntar: no último monitor escolhido, ou em todos em Span e Duplicar.
fn apply_directly(s: &mut State, id: &str) -> String {
    let title = title_of(s, id);
    match s.config.arrangement {
        Arrangement::PerScreen => match s.monitors.get(s.selected).cloned() {
            Some(m) => {
                let all = devices(s);
                s.config.put_on_screen(&all, &m.device, id);
                format!("{title} no monitor {}", m.number)
            }
            None => "Nenhum monitor encontrado".into(),
        },
        _ => {
            s.config.shared = Some(id.to_string());
            format!("{title} em todos os monitores")
        }
    }
}

fn target_label(s: &State) -> String {
    let many = s.monitors.len() > 1;
    match s.config.arrangement {
        Arrangement::PerScreen if many && s.config.general.ask_monitor => {
            "Clique num wallpaper para escolher em qual monitor ele fica.".into()
        }
        Arrangement::PerScreen if many => match s.monitors.get(s.selected) {
            Some(m) => format!("Clique num wallpaper para aplicar no monitor {}.", m.number),
            None => String::new(),
        },
        Arrangement::PerScreen => "Clique num wallpaper para aplicar.".into(),
        Arrangement::Span => "Disposição Span: o wallpaper atravessa todos os monitores.".into(),
        Arrangement::Duplicate => "Disposição Duplicar: o mesmo wallpaper em todos os monitores.".into(),
    }
}

fn rule_index(rule: Rule) -> i32 {
    if rule == Rule::Pause { 0 } else { 1 }
}

fn rule_from(index: i32) -> Rule {
    if index == 0 { Rule::Pause } else { Rule::Continue }
}

fn release_index(secs: Option<u64>) -> i32 {
    RELEASE_CHOICES.iter().position(|c| *c == secs).unwrap_or(3) as i32
}

fn theme_index(choice: ThemeChoice) -> i32 {
    match choice {
        ThemeChoice::System => 0,
        ThemeChoice::Light => 1,
        ThemeChoice::Dark => 2,
    }
}

// ---------- skins ----------

/// Monitor de uma skin: o gravado, o que contém a posição livre, ou o principal.
fn skin_monitor<'a>(s: &'a State, e: &SkinEntry) -> Option<&'a monitors::Monitor> {
    e.monitor
        .as_deref()
        .and_then(|d| s.monitors.iter().find(|m| m.device.eq_ignore_ascii_case(d)))
        .or_else(|| if e.anchor == SkinAnchor::Free { monitor_at(&s.monitors, e.x as i32, e.y as i32) } else { None })
        .or_else(|| s.monitors.iter().find(|m| m.primary))
}

/// Índices das telas onde a skin está ligada, na ordem dos monitores.
fn instances(s: &State, key: &str) -> Vec<usize> {
    let mut list: Vec<usize> = s
        .config
        .skins
        .items
        .iter()
        .enumerate()
        .filter(|(_, e)| e.active && skin_key(&e.config) == key)
        .map(|(i, _)| i)
        .collect();
    list.sort_by_key(|&i| skin_monitor(s, &s.config.skins.items[i]).map_or(usize::MAX, |m| m.number));
    list
}

fn active_count(config: &Config) -> usize {
    config.skins.items.iter().filter(|e| e.active).count()
}

fn screens_label(s: &State, key: &str) -> String {
    instances(s, key)
        .iter()
        .filter_map(|&i| skin_monitor(s, &s.config.skins.items[i]).map(|m| format!("Tela {}", m.number)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn group_matches(s: &State, g: &SkinGroup) -> bool {
    if s.config.is_hidden(&g.key) {
        return false;
    }
    match s.skin_filter {
        0 => true,
        1 => !instances(s, &g.key).is_empty(),
        2 => s.config.is_favorite(&g.key),
        n => suites(s).get((n - 3) as usize).is_some_and(|suite| *suite == g.suite),
    }
}

fn suites(s: &State) -> Vec<String> {
    let mut list: Vec<String> = s.groups.iter().map(|g| g.suite.clone()).collect();
    list.dedup();
    list
}

/// A tela da skin aberta nas configurações.
fn current_instance(s: &State) -> Option<usize> {
    let key = s.skin_open.as_deref()?;
    instances(s, key).get(s.skin_instance).copied()
}

fn alpha_for_item(item: i32) -> u8 {
    if item >= 10 { 1 } else { (255 - item.clamp(0, 9) * 230 / 9) as u8 }
}

fn item_for_alpha(alpha: u8) -> i32 {
    if alpha <= 1 { 10 } else { ((255.0 - alpha as f32) * 9.0 / 230.0).round().clamp(0.0, 9.0) as i32 }
}

fn refresh(ui: &App, s: &State) {
    let cfg = &s.config;
    ui.set_dark(theme::is_dark(cfg.general.theme));
    ui.set_theme_choice(theme_index(cfg.general.theme));

    // Fundo da janela (diálogo Tema)
    let (backdrop, has_backdrop) = image_or_empty(backdrop_source(s).map(|p| blurred_path(&p)));
    ui.set_backdrop_image(backdrop);
    ui.set_has_backdrop(has_backdrop);
    let dynamic = used_ids(s).first().and_then(|id| cfg.wallpaper(id)).map(thumbnail).unwrap_or_default();
    let mut backdrops = vec![
        BackdropItem {
            title: "Padrão".into(),
            subtitle: "Cor lisa do tema".into(),
            thumb: Image::default(),
            has_thumb: false,
            removable: false,
        },
        BackdropItem {
            title: "Dinâmico".into(),
            subtitle: "Adaptar ao wallpaper da tela".into(),
            thumb: dynamic.0,
            has_thumb: dynamic.1,
            removable: false,
        },
    ];
    for path in &cfg.general.backdrops {
        let (thumb, has_thumb) = image_or_empty(Some(small_path(path)));
        backdrops.push(BackdropItem {
            title: path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default().into(),
            subtitle: "Imagem escolhida".into(),
            thumb,
            has_thumb,
            removable: true,
        });
    }
    ui.set_backdrops(ModelRc::new(VecModel::from(backdrops)));
    ui.set_backdrop_choice(match &cfg.general.backdrop {
        Backdrop::Default => 0,
        Backdrop::Dynamic => 1,
        Backdrop::Image(p) => cfg.general.backdrops.iter().position(|b| b == p).map_or(0, |i| i as i32 + 2),
    });

    // Biblioteca
    let used = used_ids(s);
    let search = s.search.to_lowercase();
    let items: Vec<WallpaperItem> = cfg
        .library
        .iter()
        .filter(|w| kind_matches(s.filter, w.kind) && (search.is_empty() || w.title.to_lowercase().contains(&search)))
        .map(|w| {
            let (thumb, has_thumb) = thumbnail(w);
            WallpaperItem {
                id: w.id.clone().into(),
                title: w.title.clone().into(),
                kind: kind_label(w.kind).into(),
                thumb,
                has_thumb,
                in_use: used.contains(&w.id),
            }
        })
        .collect();
    ui.set_wallpapers(ModelRc::new(VecModel::from(items)));
    let count = |f: i32| cfg.library.iter().filter(|w| kind_matches(f, w.kind)).count() as i32;
    ui.set_filter(s.filter);
    ui.set_count_all(count(0));
    ui.set_count_video(count(1));
    ui.set_count_image(count(2));
    ui.set_count_gif(count(3));
    ui.set_target_label(target_label(s).into());
    ui.set_ask_monitor(cfg.general.ask_monitor);

    // Monitores (no diálogo das skins, marca onde a skin aberta está ligada)
    let marked: Vec<String> = s
        .skin_open
        .as_deref()
        .map(|key| instances(s, key).iter().filter_map(|&i| skin_monitor(s, &cfg.skins.items[i]).map(|m| m.device.clone())).collect())
        .unwrap_or_default();
    let (mut left, mut top, mut right, mut bottom) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for m in &s.monitors {
        left = left.min(m.rect.left);
        top = top.min(m.rect.top);
        right = right.max(m.rect.right);
        bottom = bottom.max(m.rect.bottom);
    }
    let monitor_items: Vec<MonitorItem> = s
        .monitors
        .iter()
        .map(|m| {
            let id = match cfg.arrangement {
                Arrangement::PerScreen => cfg.screen_wallpaper(&m.device),
                _ => cfg.shared.as_deref(),
            };
            let wallpaper = id.and_then(|id| cfg.wallpaper(id));
            let (thumb, has_thumb) = wallpaper.map(thumbnail).unwrap_or_default();
            MonitorItem {
                number: m.number as i32,
                label: format!("Monitor {}", m.number).into(),
                x: m.rect.left as f32,
                y: m.rect.top as f32,
                w: (m.rect.right - m.rect.left) as f32,
                h: (m.rect.bottom - m.rect.top) as f32,
                primary: m.primary,
                wallpaper_title: wallpaper.map(|w| w.title.clone()).unwrap_or_else(|| "Sem wallpaper".into()).into(),
                thumb,
                has_thumb,
                marked: marked.iter().any(|d| d.eq_ignore_ascii_case(&m.device)),
            }
        })
        .collect();
    ui.set_monitors(ModelRc::new(VecModel::from(monitor_items)));
    let labels: Vec<SharedString> = s.monitors.iter().map(|m| format!("Monitor {}", m.number).into()).collect();
    ui.set_monitor_labels(ModelRc::new(VecModel::from(labels)));
    if !s.monitors.is_empty() {
        ui.set_map_x(left as f32);
        ui.set_map_y(top as f32);
        ui.set_map_w((right - left).max(1) as f32);
        ui.set_map_h((bottom - top).max(1) as f32);
    }
    ui.set_selected_monitor(s.selected as i32);
    ui.set_arrangement(match cfg.arrangement {
        Arrangement::PerScreen => 0,
        Arrangement::Span => 1,
        Arrangement::Duplicate => 2,
    });
    ui.set_fit(match cfg.fit {
        Fit::Fill => 0,
        Fit::Fit => 1,
        Fit::Stretch => 2,
    });

    // Desempenho
    let p = &cfg.performance;
    ui.set_fullscreen_rule(rule_index(p.fullscreen));
    ui.set_focus_rule(rule_index(p.focus));
    ui.set_battery_rule(rule_index(p.battery));
    ui.set_saver_rule(rule_index(p.battery_saver));
    ui.set_remote_rule(rule_index(p.remote_desktop));
    ui.set_scope(if p.scope == Scope::All { 1 } else { 0 });
    ui.set_release_index(release_index(p.release_after_secs));
    let rules: Vec<AppRuleItem> = p
        .apps
        .iter()
        .map(|a| AppRuleItem {
            exe: a.exe.clone().into(),
            action: match a.action {
                AppAction::Pause => 0,
                AppAction::Ignore => 1,
                AppAction::Release => 2,
            },
        })
        .collect();
    ui.set_app_rules(ModelRc::new(VecModel::from(rules)));

    refresh_skins(ui, s);

    // Geral e sistema
    ui.set_start_with_windows(startup::is_enabled());
    ui.set_check_updates(cfg.general.check_updates);
    ui.set_tray_icon(cfg.general.tray_icon);
    ui.set_data_path(kage_core::data_dir().display().to_string().into());
}

fn refresh_skins(ui: &App, s: &State) {
    let cfg = &s.config;
    ui.set_skins_enabled(cfg.skins.enabled);
    ui.set_active_skins(active_count(cfg) as i32);
    ui.set_max_skins(MAX_ACTIVE_SKINS as i32);

    // Skins tiradas da lista não entram nos contadores nem nos cards.
    let count = |f: &dyn Fn(&SkinGroup) -> bool| s.groups.iter().filter(|g| !cfg.is_hidden(&g.key) && f(g)).count() as i32;
    ui.set_hidden_skins(s.groups.iter().filter(|g| cfg.is_hidden(&g.key)).count() as i32);
    let mut filters = vec![
        NavCount { label: "Todas as skins".into(), count: count(&|_| true) },
        NavCount { label: "Ligadas".into(), count: count(&|g| !instances(s, &g.key).is_empty()) },
        NavCount { label: "Favoritas".into(), count: count(&|g| cfg.is_favorite(&g.key)) },
    ];
    for suite in suites(s) {
        filters.push(NavCount { count: count(&|g| g.suite == suite), label: suite.into() });
    }
    ui.set_skin_filters(ModelRc::new(VecModel::from(filters)));
    ui.set_skin_filter(s.skin_filter);

    let cards: Vec<SkinCard> = s
        .groups
        .iter()
        .filter(|g| group_matches(s, g))
        .map(|g| {
            let (preview, has_preview) = image_or_empty(Some(skin_preview_path(&g.key)));
            let screens = screens_label(s, &g.key);
            SkinCard {
                key: g.key.clone().into(),
                name: g.name.clone().into(),
                suite: g.suite.to_uppercase().into(),
                preview,
                has_preview,
                active: !screens.is_empty(),
                screens: screens.into(),
                favorite: cfg.is_favorite(&g.key),
            }
        })
        .collect();
    ui.set_skin_cards(ModelRc::new(VecModel::from(cards)));

    // Skin aberta: diálogo das telas e configurações por tela
    let Some(group) = s.skin_open.as_deref().and_then(|k| s.groups.iter().find(|g| g.key == k)) else { return };
    ui.set_skin_name(group.name.clone().into());
    ui.set_skin_suite(group.suite.clone().into());
    let (preview, has_preview) = image_or_empty(Some(skin_preview_path(&group.key)));
    ui.set_skin_preview(preview);
    ui.set_skin_has_preview(has_preview);
    let list = instances(s, &group.key);
    let labels: Vec<SharedString> = list
        .iter()
        .map(|&i| skin_monitor(s, &cfg.skins.items[i]).map_or("Tela".to_string(), |m| format!("Tela {}", m.number)).into())
        .collect();
    ui.set_skin_instances(ModelRc::new(VecModel::from(labels)));
    let current = s.skin_instance.min(list.len().saturating_sub(1));
    ui.set_skin_instance(current as i32);
    let Some(entry) = list.get(current).map(|&i| &cfg.skins.items[i]) else { return };
    ui.set_s_anchor(if entry.anchor == SkinAnchor::Centered { 0 } else { 1 });
    ui.set_s_height(entry.height_percent);
    ui.set_s_x(format!("{}", entry.x).into());
    ui.set_s_y(format!("{}", entry.y).into());
    let variants = kage_skins::rainmeter::variants(&entry.ini);
    let names: Vec<SharedString> =
        variants.iter().map(|v| v.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default().into()).collect();
    ui.set_s_variants(ModelRc::new(VecModel::from(names)));
    ui.set_s_variant(variants.iter().position(|v| *v == entry.ini).unwrap_or(0) as i32);
    ui.set_s_alpha(item_for_alpha(entry.alpha));
    ui.set_s_layer(LAYERS.iter().position(|l| *l == entry.layer).unwrap_or(4) as i32);
    ui.set_s_hover(HOVERS.iter().position(|h| *h == entry.on_hover).unwrap_or(0) as i32);
    ui.set_s_click(entry.click_through);
    ui.set_s_drag(entry.draggable);
    ui.set_s_keep(entry.keep_on_screen);
    ui.set_s_snap(entry.snap_edges);
    ui.set_s_save(entry.save_position);
}

fn update_engine_status(ui: &App) {
    let running = engine::is_running();
    ui.set_engine_running(running);
    let detail = if running { engine::memory_mb().map(|mb| format!("RAM {mb} MB")).unwrap_or_default() } else { String::new() };
    ui.set_engine_detail(detail.into());
}

fn show_toast(ui: &App, timer: &Timer, text: &str) {
    ui.set_toast(text.into());
    let weak = ui.as_weak();
    timer.start(TimerMode::SingleShot, Duration::from_millis(2600), move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_toast("".into());
        }
    });
}

/// Grava o config.json. Se o motor gravou algo depois da última leitura (skin arrastada), traz
/// a posição das skins do disco antes, para não desfazer o que o usuário fez na área de trabalho.
fn save(s: &mut State) -> Result<(), String> {
    if config_mtime() != s.known_mtime
        && let Ok(disk) = Config::load_from(&kage_core::config_path())
    {
        for entry in &mut s.config.skins.items {
            if let Some(d) = disk.skin(&entry.id) {
                (entry.x, entry.y, entry.anchor, entry.monitor) = (d.x, d.y, d.anchor, d.monitor.clone());
            }
        }
    }
    let result = s.config.save();
    s.known_mtime = config_mtime();
    result
}

/// Salva, avisa o motor e atualiza a tela.
fn commit(ui: &App, state: &Shared, timer: &Timer, message: &str) {
    let result = save(&mut state.borrow_mut()).and_then(|_| engine::reload());
    refresh(ui, &state.borrow());
    match result {
        Ok(()) => show_toast(ui, timer, message),
        Err(e) => show_toast(ui, timer, &format!("Não salvou: {e}")),
    }
}

/// Salva sem recarregar o motor (opções só da janela).
fn save_only(ui: &App, state: &Shared, timer: &Timer, message: &str) {
    let result = save(&mut state.borrow_mut());
    refresh(ui, &state.borrow());
    match result {
        Ok(()) => show_toast(ui, timer, message),
        Err(e) => show_toast(ui, timer, &format!("Não salvou: {e}")),
    }
}

/// Roda tarefas do motor em segundo plano (miniaturas, prévias) e atualiza a tela a cada uma.
fn spawn_jobs(ui: &App, jobs: Vec<Box<dyn FnOnce() -> bool + Send>>) {
    if jobs.is_empty() {
        return;
    }
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        for job in jobs {
            if job() {
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.invoke_refresh_library();
                    }
                });
            }
        }
    });
}

/// Gera as miniaturas que faltam (o motor faz o trabalho).
fn spawn_thumbnails(ui: &App, s: &State) {
    let jobs = s
        .config
        .library
        .iter()
        .filter(|w| thumbnail_path(w).is_none())
        .map(|w| {
            let (input, output) = (w.path.clone(), kage_core::thumbnails_dir().join(format!("{}.jpg", w.id)));
            Box::new(move || engine::make_thumbnail(&input, &output)) as Box<dyn FnOnce() -> bool + Send>
        })
        .collect();
    spawn_jobs(ui, jobs);
}

/// Prévia de cada skin, desenhada pelo motor; refeita quando o .ini muda.
fn spawn_skin_previews(ui: &App, s: &State) {
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let jobs = s
        .groups
        .iter()
        .filter(|g| {
            let out = skin_preview_path(&g.key);
            match (modified(&out), modified(&g.ini)) {
                (Some(done), Some(source)) => done < source,
                (None, _) => true,
                _ => false,
            }
        })
        .map(|g| {
            let (ini, out) = (g.ini.clone(), skin_preview_path(&g.key));
            Box::new(move || engine::render_skin(&ini, &out)) as Box<dyn FnOnce() -> bool + Send>
        })
        .collect();
    spawn_jobs(ui, jobs);
}

/// Versão desfocada do fundo atual e miniaturas das imagens de fundo, se faltarem.
fn spawn_backdrop(ui: &App, s: &State) {
    let mut jobs: Vec<Box<dyn FnOnce() -> bool + Send>> = Vec::new();
    if let Some(source) = backdrop_source(s) {
        let out = blurred_path(&source);
        if !out.is_file() {
            jobs.push(Box::new(move || engine::make_blurred(&source, &out)));
        }
    }
    for path in &s.config.general.backdrops {
        let (input, out) = (path.clone(), small_path(path));
        if !out.is_file() {
            jobs.push(Box::new(move || engine::make_thumbnail(&input, &out)));
        }
    }
    spawn_jobs(ui, jobs);
}

fn add_paths(s: &mut State, files: &[PathBuf]) -> usize {
    let before = s.config.library.len();
    for file in files {
        s.config.add_file(file);
    }
    s.config.library.len() - before
}

fn open_in_explorer(path: &Path, select: bool) {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("explorer");
    if select {
        // O Explorer só aceita o caminho entre aspas colado no "/select,".
        command.raw_arg(format!("/select,\"{}\"", path.display()));
    } else {
        command.arg(path);
    }
    let _ = command.spawn();
}

fn size_label(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} KB", b / 1024),
    }
}

/// Liga cada evento da tela à configuração.
fn wire(ui: &App, state: &Shared, timer: &Rc<Timer>) {
    macro_rules! handler {
        ($on:ident, |$ui:ident, $state:ident, $timer:ident $(, $arg:ident : $ty:ty)*| $body:block) => {{
            let weak = ui.as_weak();
            let $state = state.clone();
            let $timer = timer.clone();
            ui.$on(move |$($arg: $ty),*| {
                let Some($ui) = weak.upgrade() else { return };
                let _ = (&$ui, &$state, &$timer);
                $body
            });
        }};
    }

    handler!(on_refresh_library, |ui, state, timer| {
        refresh(&ui, &state.borrow());
    });

    // ---------- moldura própria ----------

    handler!(on_drag_window, |ui, state, timer| {
        frame::press_title(ui.window());
    });

    handler!(on_minimize_window, |ui, state, timer| {
        ui.window().set_minimized(true);
    });

    handler!(on_toggle_maximize, |ui, state, timer| {
        frame::toggle_maximize(ui.window());
    });

    handler!(on_close_window, |ui, state, timer| {
        let _ = slint::quit_event_loop();
    });

    handler!(on_resize_window, |ui, state, timer, edge: i32| {
        frame::start_resize(ui.window(), edge);
    });

    handler!(on_open_color_settings, |ui, state, timer| {
        let _ = std::process::Command::new("explorer").arg("ms-settings:colors").spawn();
    });

    // ---------- biblioteca ----------

    handler!(on_pick_wallpaper, |ui, state, timer, id: SharedString| {
        let ask = {
            let s = state.borrow();
            s.monitors.len() > 1 && s.config.general.ask_monitor
        };
        if ask {
            let title = title_of(&state.borrow(), &id);
            state.borrow_mut().pending = Some(id.to_string());
            ui.set_pending_title(title.into());
            ui.set_dialog(3);
            return;
        }
        let message = apply_directly(&mut state.borrow_mut(), &id);
        commit(&ui, &state, &timer, &message);
        spawn_backdrop(&ui, &state.borrow());
    });

    handler!(on_apply_to_monitor, |ui, state, timer, index: i32| {
        let message = {
            let mut s = state.borrow_mut();
            let (Some(id), Some(monitor)) = (s.pending.take(), s.monitors.get(index as usize).cloned()) else { return };
            let all = devices(&s);
            s.config.put_on_screen(&all, &monitor.device, &id);
            s.selected = index as usize;
            format!("{} no monitor {}", title_of(&s, &id), monitor.number)
        };
        ui.set_dialog(0);
        commit(&ui, &state, &timer, &message);
        spawn_backdrop(&ui, &state.borrow());
    });

    handler!(on_apply_to_all, |ui, state, timer, mode: i32| {
        let message = {
            let mut s = state.borrow_mut();
            let Some(id) = s.pending.take() else { return };
            s.config.arrangement = if mode == 1 { Arrangement::Span } else { Arrangement::Duplicate };
            s.config.shared = Some(id.clone());
            let how = if mode == 1 { "atravessando todos os monitores" } else { "em todos os monitores" };
            format!("{} {how}", title_of(&s, &id))
        };
        ui.set_dialog(0);
        commit(&ui, &state, &timer, &message);
        spawn_backdrop(&ui, &state.borrow());
    });

    handler!(on_set_ask_monitor, |ui, state, timer, ask: bool| {
        state.borrow_mut().config.general.ask_monitor = ask;
        save_only(&ui, &state, &timer, if ask { "Vai perguntar o monitor" } else { "Aplica direto no último monitor" });
    });

    handler!(on_remove_wallpaper, |ui, state, timer, id: SharedString| {
        let _ = std::fs::remove_file(kage_core::thumbnails_dir().join(format!("{id}.jpg")));
        let title = {
            let mut s = state.borrow_mut();
            let title = title_of(&s, &id);
            s.config.remove(&id);
            title
        };
        commit(&ui, &state, &timer, &format!("{title} saiu da biblioteca (o arquivo continua no disco)"));
    });

    handler!(on_reveal_wallpaper, |ui, state, timer, id: SharedString| {
        match state.borrow().config.wallpaper(&id).map(|w| w.path.clone()) {
            Some(path) if path.exists() => open_in_explorer(&path, true),
            _ => show_toast(&ui, &timer, "O arquivo não está mais nesse lugar"),
        }
    });

    handler!(on_about_wallpaper, |ui, state, timer, id: SharedString| {
        let s = state.borrow();
        let Some(w) = s.config.wallpaper(&id) else { return };
        let (thumb, has_thumb) = thumbnail(w);
        ui.set_about_title(w.title.clone().into());
        ui.set_about_kind(
            match w.kind {
                MediaKind::Video => "Vídeo",
                MediaKind::Image => "Imagem",
                MediaKind::Gif => "GIF",
            }
            .into(),
        );
        let size = std::fs::metadata(&w.path).map(|m| size_label(m.len())).unwrap_or_else(|_| "arquivo não encontrado".into());
        ui.set_about_size(size.into());
        ui.set_about_path(w.path.display().to_string().into());
        ui.set_about_thumb(thumb);
        ui.set_about_has_thumb(has_thumb);
        ui.set_dialog(4);
    });

    handler!(on_preview_wallpaper, |ui, state, timer, id: SharedString| {
        match state.borrow().config.wallpaper(&id).map(|w| w.path.clone()) {
            // Abre no programa padrão do Windows para esse tipo de arquivo.
            Some(path) if path.exists() => open_in_explorer(&path, false),
            _ => show_toast(&ui, &timer, "O arquivo não está mais nesse lugar"),
        }
    });

    handler!(on_customize_wallpaper, |ui, state, timer, id: SharedString| {
        let mut s = state.borrow_mut();
        let Some(w) = s.config.wallpaper(&id).cloned() else { return };
        s.menu_target = Some(id.to_string());
        ui.set_about_title(w.title.clone().into());
        ui.set_look_speed(w.look.speed);
        ui.set_look_brightness(w.look.brightness);
        ui.set_look_contrast(w.look.contrast);
        ui.set_look_saturation(w.look.saturation);
        ui.set_look_moves(w.kind != MediaKind::Image);
        ui.set_dialog(5);
    });

    handler!(on_set_look, |ui, state, timer, field: SharedString, value: f32| {
        let value = value.round();
        let message = {
            let mut s = state.borrow_mut();
            let Some(id) = s.menu_target.clone() else { return };
            let Some(w) = s.config.library.iter_mut().find(|w| w.id == id) else { return };
            let (slot, label) = match field.as_str() {
                "speed" => (&mut w.look.speed, "Velocidade"),
                "brightness" => (&mut w.look.brightness, "Brilho"),
                "contrast" => (&mut w.look.contrast, "Contraste"),
                _ => (&mut w.look.saturation, "Saturação"),
            };
            *slot = value;
            format!("{label}: {value}%")
        };
        let look = state.borrow().menu_target.clone().and_then(|id| state.borrow().config.wallpaper(&id).map(|w| w.look));
        if let Some(look) = look {
            ui.set_look_speed(look.speed);
            ui.set_look_brightness(look.brightness);
            ui.set_look_contrast(look.contrast);
            ui.set_look_saturation(look.saturation);
        }
        commit(&ui, &state, &timer, &message);
    });

    handler!(on_reset_look, |ui, state, timer| {
        {
            let mut s = state.borrow_mut();
            let Some(id) = s.menu_target.clone() else { return };
            let Some(w) = s.config.library.iter_mut().find(|w| w.id == id) else { return };
            w.look = Look::default();
        }
        let look = Look::default();
        ui.set_look_speed(look.speed);
        ui.set_look_brightness(look.brightness);
        ui.set_look_contrast(look.contrast);
        ui.set_look_saturation(look.saturation);
        commit(&ui, &state, &timer, "Ajustes do wallpaper restaurados");
    });

    handler!(on_share_wallpaper, |ui, state, timer, id: SharedString| {
        let Some(path) = state.borrow().config.wallpaper(&id).map(|w| w.path.clone()) else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Some(target) = rfd::FileDialog::new().set_title("Salvar uma cópia do wallpaper").set_file_name(&name).save_file()
        else {
            return;
        };
        match std::fs::copy(&path, &target) {
            Ok(_) => show_toast(&ui, &timer, &format!("Cópia salva em {}", target.display())),
            Err(e) => show_toast(&ui, &timer, &format!("Não copiou: {e}")),
        }
    });

    handler!(on_edit_wallpaper, |ui, state, timer, id: SharedString| {
        let title = title_of(&state.borrow(), &id);
        state.borrow_mut().menu_target = Some(id.to_string());
        ui.set_edit_title(title.into());
        ui.set_dialog(6);
    });

    handler!(on_rename_wallpaper, |ui, state, timer, text: SharedString| {
        let title = text.trim().to_string();
        if title.is_empty() {
            show_toast(&ui, &timer, "Digite um nome");
            return;
        }
        {
            let mut s = state.borrow_mut();
            let Some(id) = s.menu_target.clone() else { return };
            let Some(w) = s.config.library.iter_mut().find(|w| w.id == id) else { return };
            w.title = title.clone();
        }
        ui.set_dialog(0);
        save_only(&ui, &state, &timer, &format!("Agora se chama {title}"));
    });

    handler!(on_add_files, |ui, state, timer| {
        let Some(files) = rfd::FileDialog::new()
            .set_title("Adicionar wallpapers")
            .add_filter("Vídeos, imagens e GIFs", kage_core::SUPPORTED_EXTENSIONS)
            .pick_files()
        else {
            return;
        };
        let added = add_paths(&mut state.borrow_mut(), &files);
        ui.set_dialog(0);
        save_only(&ui, &state, &timer, &format!("{added} wallpaper(s) na biblioteca. Clique num deles para aplicar."));
        spawn_thumbnails(&ui, &state.borrow());
    });

    handler!(on_add_folder, |ui, state, timer| {
        let Some(folder) = rfd::FileDialog::new().set_title("Escolher pasta de wallpapers").pick_folder() else { return };
        let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| kage_core::kind_of(p).is_some()).collect())
            .unwrap_or_default();
        files.sort();
        let added = add_paths(&mut state.borrow_mut(), &files);
        ui.set_dialog(0);
        save_only(&ui, &state, &timer, &format!("{added} wallpaper(s) da pasta na biblioteca"));
        spawn_thumbnails(&ui, &state.borrow());
    });

    handler!(on_set_filter, |ui, state, timer, filter: i32| {
        state.borrow_mut().filter = filter;
        refresh(&ui, &state.borrow());
    });

    handler!(on_search_changed, |ui, state, timer, text: SharedString| {
        state.borrow_mut().search = text.to_string();
        refresh(&ui, &state.borrow());
    });

    handler!(on_select_monitor, |ui, state, timer, index: i32| {
        state.borrow_mut().selected = index.max(0) as usize;
        refresh(&ui, &state.borrow());
    });

    handler!(on_set_arrangement, |ui, state, timer, index: i32| {
        let label = {
            let mut s = state.borrow_mut();
            let arrangement = match index {
                1 => Arrangement::Span,
                2 => Arrangement::Duplicate,
                _ => Arrangement::PerScreen,
            };
            // Para Span ou Duplicar sem wallpaper comum: usa o do monitor escolhido.
            if arrangement != Arrangement::PerScreen && s.config.shared.is_none() {
                let from_selected =
                    s.monitors.get(s.selected).and_then(|m| s.config.screen_wallpaper(&m.device)).map(str::to_string);
                let any = s.config.screens.iter().find_map(|a| a.wallpaper.clone());
                s.config.shared = from_selected.or(any);
            }
            // Para Tela vindo de Span ou Duplicar: cada monitor fica com o que mostrava.
            if arrangement == Arrangement::PerScreen
                && s.config.arrangement != Arrangement::PerScreen
                && let Some(shared) = s.config.shared.clone()
            {
                for device in devices(&s) {
                    s.config.assign(&device, Some(&shared));
                }
            }
            s.config.arrangement = arrangement;
            ["Disposição: Tela", "Disposição: Span", "Disposição: Duplicar"][index.clamp(0, 2) as usize]
        };
        commit(&ui, &state, &timer, label);
    });

    handler!(on_set_fit, |ui, state, timer, index: i32| {
        state.borrow_mut().config.fit = match index {
            1 => Fit::Fit,
            2 => Fit::Stretch,
            _ => Fit::Fill,
        };
        commit(&ui, &state, &timer, ["Ajuste: preencher", "Ajuste: ajustar", "Ajuste: esticar"][index.clamp(0, 2) as usize]);
    });

    handler!(on_clear_wallpapers, |ui, state, timer| {
        state.borrow_mut().config.clear_wallpapers();
        commit(&ui, &state, &timer, "Wallpapers fechados");
    });

    // ---------- desempenho ----------

    handler!(on_set_rule, |ui, state, timer, name: SharedString, index: i32| {
        {
            let p = &mut state.borrow_mut().config.performance;
            match name.as_str() {
                "fullscreen" => p.fullscreen = rule_from(index),
                "focus" => p.focus = rule_from(index),
                "battery" => p.battery = rule_from(index),
                "saver" => p.battery_saver = rule_from(index),
                "remote" => p.remote_desktop = rule_from(index),
                "scope" => p.scope = if index == 1 { Scope::All } else { Scope::PerScreen },
                _ => return,
            }
        }
        commit(&ui, &state, &timer, "Regra salva");
    });

    handler!(on_set_release, |ui, state, timer, index: i32| {
        state.borrow_mut().config.performance.release_after_secs =
            RELEASE_CHOICES.get(index.max(0) as usize).copied().flatten();
        commit(&ui, &state, &timer, "Liberação de memória salva");
    });

    handler!(on_add_app_rule, |ui, state, timer, exe: SharedString, action: i32| {
        let mut exe = exe.trim().to_ascii_lowercase();
        if exe.is_empty() {
            show_toast(&ui, &timer, "Digite o nome do programa, como obs64.exe");
            return;
        }
        if !exe.ends_with(".exe") {
            exe.push_str(".exe");
        }
        let action = match action {
            1 => AppAction::Ignore,
            2 => AppAction::Release,
            _ => AppAction::Pause,
        };
        {
            let apps = &mut state.borrow_mut().config.performance.apps;
            apps.retain(|a| a.exe != exe);
            apps.push(AppRule { exe: exe.clone(), action });
        }
        commit(&ui, &state, &timer, &format!("Regra para {exe} salva"));
    });

    handler!(on_remove_app_rule, |ui, state, timer, index: i32| {
        {
            let apps = &mut state.borrow_mut().config.performance.apps;
            if (index as usize) < apps.len() {
                apps.remove(index as usize);
            }
        }
        commit(&ui, &state, &timer, "Regra removida");
    });

    // ---------- skins ----------

    handler!(on_toggle_skins, |ui, state, timer, enabled: bool| {
        state.borrow_mut().config.skins.enabled = enabled;
        commit(&ui, &state, &timer, if enabled { "Skins na tela" } else { "Skins escondidas" });
    });

    handler!(on_set_skin_filter, |ui, state, timer, filter: i32| {
        state.borrow_mut().skin_filter = filter;
        refresh(&ui, &state.borrow());
    });

    handler!(on_open_skin, |ui, state, timer, key: SharedString| {
        {
            let mut s = state.borrow_mut();
            s.skin_open = Some(key.to_string());
            s.skin_instance = 0;
        }
        refresh(&ui, &state.borrow());
        ui.set_dialog(8);
    });

    handler!(on_open_skin_settings, |ui, state, timer, key: SharedString| {
        {
            let mut s = state.borrow_mut();
            s.skin_open = Some(key.to_string());
            s.skin_instance = 0;
        }
        refresh(&ui, &state.borrow());
        ui.set_skin_detail_open(true);
    });

    handler!(on_toggle_skin_screen, |ui, state, timer, index: i32| {
        let message = {
            let mut s = state.borrow_mut();
            let (Some(key), Some(monitor)) = (s.skin_open.clone(), s.monitors.get(index as usize).cloned()) else { return };
            let Some(group) = s.groups.iter().find(|g| g.key == key).cloned() else { return };
            let on_this = |s: &State, e: &SkinEntry| {
                skin_key(&e.config) == key && skin_monitor(s, e).is_some_and(|m| m.device == monitor.device)
            };
            let active_here = s.config.skins.items.iter().position(|e| e.active && on_this(&s, e));
            match active_here {
                Some(i) => {
                    s.config.skins.items[i].active = false;
                    format!("{} desligada na tela {}", group.name, monitor.number)
                }
                None if active_count(&s.config) >= MAX_ACTIVE_SKINS => {
                    drop(s);
                    show_toast(&ui, &timer, &format!("Já há {MAX_ACTIVE_SKINS} skins ligadas: desligue uma para ligar outra"));
                    return;
                }
                None => {
                    // Volta com as opções que tinha nessa tela, se já esteve lá.
                    match s.config.skins.items.iter().position(|e| !e.active && on_this(&s, e)) {
                        Some(i) => s.config.skins.items[i].active = true,
                        None => s.config.skins.items.push(SkinEntry {
                            id: kage_core::new_skin_id(),
                            config: group.config.clone(),
                            ini: group.ini.clone(),
                            active: true,
                            anchor: SkinAnchor::Centered,
                            monitor: Some(monitor.device.clone()),
                            height_percent: 30.0,
                            ..Default::default()
                        }),
                    }
                    s.config.skins.enabled = true;
                    format!("{} ligada na tela {}", group.name, monitor.number)
                }
            }
        };
        commit(&ui, &state, &timer, &message);
    });

    handler!(on_select_skin_instance, |ui, state, timer, index: i32| {
        state.borrow_mut().skin_instance = index.max(0) as usize;
        refresh(&ui, &state.borrow());
    });

    handler!(on_set_skin_option, |ui, state, timer, name: SharedString, value: i32| {
        let message = {
            let mut s = state.borrow_mut();
            let Some(index) = current_instance(&s) else { return };
            let monitor_rect = skin_monitor(&s, &s.config.skins.items[index]).map(|m| m.rect);
            let e = &mut s.config.skins.items[index];
            match name.as_str() {
                "anchor" => {
                    e.anchor = if value == 0 { SkinAnchor::Centered } else { SkinAnchor::Free };
                    // Livre sem posição ainda: perto do canto do monitor dela.
                    if e.anchor == SkinAnchor::Free
                        && e.x == 0.0
                        && e.y == 0.0
                        && let Some(r) = monitor_rect
                    {
                        (e.x, e.y) = ((r.left + 100) as f32, (r.top + 100) as f32);
                    }
                    "Posição salva".to_string()
                }
                "variant" => {
                    let Some(v) = kage_skins::rainmeter::variants(&e.ini).get(value.max(0) as usize).cloned() else { return };
                    e.ini = v;
                    "Variante trocada".to_string()
                }
                "alpha" => {
                    e.alpha = alpha_for_item(value);
                    "Transparência salva".to_string()
                }
                "layer" => {
                    e.layer = LAYERS[value.clamp(0, 4) as usize];
                    "Camada salva".to_string()
                }
                "hover" => {
                    e.on_hover = HOVERS[value.clamp(0, 3) as usize];
                    "Salvo".to_string()
                }
                "click" => {
                    e.click_through = value != 0;
                    "Salvo".to_string()
                }
                "drag" => {
                    e.draggable = value != 0;
                    "Salvo".to_string()
                }
                "keep" => {
                    e.keep_on_screen = value != 0;
                    "Salvo".to_string()
                }
                "snap" => {
                    e.snap_edges = value != 0;
                    "Salvo".to_string()
                }
                "save" => {
                    e.save_position = value != 0;
                    "Salvo".to_string()
                }
                "off" => {
                    e.active = false;
                    s.skin_instance = 0;
                    "Skin desligada nesta tela".to_string()
                }
                _ => return,
            }
        };
        commit(&ui, &state, &timer, &message);
    });

    handler!(on_set_skin_height, |ui, state, timer, percent: f32| {
        let percent = percent.clamp(5.0, 95.0).round();
        {
            let mut s = state.borrow_mut();
            let Some(index) = current_instance(&s) else { return };
            let e = &mut s.config.skins.items[index];
            e.height_percent = percent;
            e.anchor = SkinAnchor::Centered;
        }
        commit(&ui, &state, &timer, &format!("Skin a {percent}% da altura"));
    });

    handler!(on_set_skin_xy, |ui, state, timer, x: SharedString, y: SharedString| {
        let parse = |v: &str| v.trim().replace(',', ".").parse::<f32>().ok();
        let (Some(x), Some(y)) = (parse(&x), parse(&y)) else {
            show_toast(&ui, &timer, "Use números, como 984 e 300");
            return;
        };
        {
            let mut s = state.borrow_mut();
            let Some(index) = current_instance(&s) else { return };
            let e = &mut s.config.skins.items[index];
            (e.x, e.y, e.anchor) = (x, y, SkinAnchor::Free);
        }
        commit(&ui, &state, &timer, "Posição salva");
    });

    handler!(on_skin_action, |ui, state, timer, key: SharedString, action: SharedString| {
        let key = if key.is_empty() { state.borrow().skin_open.clone().unwrap_or_default() } else { key.to_string() };
        let (group, current_ini) = {
            let s = state.borrow();
            let Some(group) = s.groups.iter().find(|g| g.key == key).cloned() else { return };
            let ini = current_instance(&s).filter(|_| s.skin_open.as_deref() == Some(&key)).map(|i| s.config.skins.items[i].ini.clone());
            (group, ini)
        };
        let ini = current_ini.unwrap_or_else(|| group.ini.clone());
        match action.as_str() {
            "edit" => {
                let _ = std::process::Command::new("notepad").arg(&ini).spawn();
            }
            "folder" => {
                if let Some(folder) = ini.parent() {
                    open_in_explorer(folder, false);
                }
            }
            "favorite" => {
                let now = {
                    let mut s = state.borrow_mut();
                    let favorite = !s.config.is_favorite(&key);
                    s.config.set_favorite(&key, favorite);
                    favorite
                };
                save_only(&ui, &state, &timer, if now { "Nos favoritos" } else { "Fora dos favoritos" });
            }
            "off" => {
                {
                    let mut s = state.borrow_mut();
                    for e in s.config.skins.items.iter_mut().filter(|e| skin_key(&e.config) == key) {
                        e.active = false;
                    }
                }
                commit(&ui, &state, &timer, &format!("{} desligada em todas as telas", group.name));
            }
            "hide" => {
                {
                    let mut s = state.borrow_mut();
                    s.config.set_hidden(&key, true);
                    if s.skin_open.as_deref() == Some(&key) {
                        s.skin_open = None;
                    }
                }
                ui.set_skin_detail_open(false);
                commit(&ui, &state, &timer, &format!("{} saiu da lista (os arquivos continuam na pasta)", group.name));
            }
            _ => {}
        }
    });

    handler!(on_show_hidden_skins, |ui, state, timer| {
        let count = {
            let mut s = state.borrow_mut();
            let count = s.config.skins.hidden.len();
            s.config.skins.hidden.clear();
            count
        };
        save_only(&ui, &state, &timer, &format!("{count} skin(s) de volta à lista"));
    });

    // ---------- geral e sistema ----------

    handler!(on_set_theme, |ui, state, timer, index: i32| {
        state.borrow_mut().config.general.theme = match index {
            1 => ThemeChoice::Light,
            2 => ThemeChoice::Dark,
            _ => ThemeChoice::System,
        };
        save_only(&ui, &state, &timer, ["Tema do sistema", "Tema claro", "Tema escuro"][index.clamp(0, 2) as usize]);
    });

    handler!(on_set_backdrop, |ui, state, timer, index: i32| {
        {
            let mut s = state.borrow_mut();
            s.config.general.backdrop = match index {
                0 => Backdrop::Default,
                1 => Backdrop::Dynamic,
                n => match s.config.general.backdrops.get((n - 2) as usize) {
                    Some(path) => Backdrop::Image(path.clone()),
                    None => return,
                },
            };
        }
        save_only(&ui, &state, &timer, "Fundo da janela salvo");
        spawn_backdrop(&ui, &state.borrow());
    });

    handler!(on_add_backdrop, |ui, state, timer| {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Imagem de fundo do Kage")
            .add_filter("Imagens", &["png", "jpg", "jpeg", "bmp", "webp", "gif"])
            .pick_file()
        else {
            return;
        };
        {
            let general = &mut state.borrow_mut().config.general;
            general.backdrops.retain(|b| *b != path);
            general.backdrops.push(path.clone());
            general.backdrop = Backdrop::Image(path);
        }
        save_only(&ui, &state, &timer, "Imagem de fundo escolhida");
        spawn_backdrop(&ui, &state.borrow());
    });

    handler!(on_remove_backdrop, |ui, state, timer, index: i32| {
        {
            let general = &mut state.borrow_mut().config.general;
            let at = (index - 2).max(0) as usize;
            if at >= general.backdrops.len() {
                return;
            }
            let removed = general.backdrops.remove(at);
            if general.backdrop == Backdrop::Image(removed) {
                general.backdrop = Backdrop::Default;
            }
        }
        save_only(&ui, &state, &timer, "Imagem tirada da lista");
    });

    handler!(on_set_start_with_windows, |ui, state, timer, enabled: bool| {
        match startup::set(enabled, &engine::exe_path()) {
            Ok(()) => {
                state.borrow_mut().config.general.start_with_windows = enabled;
                save_only(&ui, &state, &timer, if enabled { "O Kage vai abrir com o Windows" } else { "O Kage não abre mais com o Windows" });
            }
            Err(e) => show_toast(&ui, &timer, &e),
        }
        ui.set_start_with_windows(startup::is_enabled());
    });

    handler!(on_start_engine, |ui, state, timer| {
        match engine::start() {
            Ok(()) => show_toast(&ui, &timer, "Motor iniciado"),
            Err(e) => show_toast(&ui, &timer, &e),
        }
    });

    handler!(on_stop_engine, |ui, state, timer| {
        engine::stop();
        show_toast(&ui, &timer, "Motor parado");
    });

    handler!(on_open_data_folder, |ui, state, timer| {
        let dir = kage_core::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        open_in_explorer(&dir, false);
    });

    handler!(on_open_log, |ui, state, timer| {
        let log = kage_core::engine_log_path();
        if log.is_file() {
            let _ = std::process::Command::new("notepad").arg(log).spawn();
        } else {
            show_toast(&ui, &timer, "O motor ainda não registrou nada");
        }
    });
}

// ---------- atualização e novidades ----------

thread_local! {
    /// Última release achada no GitHub, esperando o "Atualizar agora".
    static LATEST: RefCell<Option<update::Release>> = const { RefCell::new(None) };
    /// Relógio do aviso rápido, para o resultado da procura (que volta de outra thread) usar.
    static TOAST_TIMER: RefCell<Option<Rc<Timer>>> = const { RefCell::new(None) };
}

fn release_notes_model() -> ModelRc<ReleaseNote> {
    let notes: Vec<ReleaseNote> = notes::NOTES
        .iter()
        .map(|n| ReleaseNote {
            version: n.version.into(),
            date: n.date.into(),
            title: n.title.into(),
            items: ModelRc::new(VecModel::from(n.items.iter().map(|i| SharedString::from(*i)).collect::<Vec<_>>())),
        })
        .collect();
    ModelRc::new(VecModel::from(notes))
}

/// Primeira abertura de uma versão nova: abre "Novidades" uma vez. Na primeira instalação não
/// abre (não há "de onde" ter vindo), só anota a versão.
fn show_news_once(ui: &App, state: &Shared) {
    let current = update::current_version();
    let current = current.as_str();
    let seen = state.borrow().config.general.seen_version.clone();
    if seen == current {
        return;
    }
    if !seen.is_empty() && notes::NOTES.iter().any(|n| n.version == current) {
        ui.set_dialog(11);
    }
    let mut s = state.borrow_mut();
    s.config.general.seen_version = current.to_string();
    let _ = save(&mut s);
}

/// Procura a última release em segundo plano. Sozinha (`interactive` falso), só avisa quando
/// há versão nova e fica quieta se a internet falhar.
fn start_update_check(ui: &App, timer: &Rc<Timer>, interactive: bool) {
    if matches!(ui.get_update_state(), 1 | 4 | 5) {
        return;
    }
    let before = ui.get_update_state();
    ui.set_update_state(1);
    ui.set_update_detail("".into());
    TOAST_TIMER.with(|t| *t.borrow_mut() = Some(timer.clone()));
    let weak = ui.as_weak();
    let current = update::current_version();
    std::thread::spawn(move || {
        let result = update::latest();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let now = time_label();
            match result {
                Ok(Some(release)) if update::is_newer(&release.version, &current) => {
                    ui.set_update_state(3);
                    ui.set_update_version(release.version.clone().into());
                    ui.set_update_notes(update::plain_notes(&release.notes).into());
                    ui.set_update_detail(
                        format!("Você tem a {current}. A troca leva poucos segundos e mantém wallpapers, skins e configurações.").into(),
                    );
                    if !interactive && let Some(timer) = TOAST_TIMER.with(|t| t.borrow().clone()) {
                        show_toast(&ui, &timer, &format!("Versão {} disponível em Mais opções › Atualizações", release.version));
                    }
                    LATEST.with(|l| *l.borrow_mut() = Some(release));
                }
                Ok(_) => {
                    ui.set_update_state(2);
                    ui.set_update_detail(format!("Você tem a versão {current}, a mais recente. Procurado às {now}.").into());
                }
                Err(e) if interactive => {
                    ui.set_update_state(6);
                    ui.set_update_detail(format!("Não deu para procurar: {e}.").into());
                }
                Err(_) => {
                    ui.set_update_state(before);
                }
            }
        });
    });
}

/// Baixa, confere e instala a versão achada; no fim reabre o motor e a janela já na nova.
fn start_update_install(ui: &App) {
    let Some(release) = LATEST.with(|l| l.borrow().clone()) else { return };
    let current = update::current_version();
    let failed = |ui: &App, text: String| {
        ui.set_update_state(6);
        ui.set_update_detail(text.into());
    };
    let Some(dir) = update::install_dir() else {
        return failed(ui, "Não achou a pasta do Kage.".into());
    };
    if update::is_dev_build(&dir) {
        return failed(ui, "Esta cópia foi compilada do código (pasta target): atualize pelo código-fonte.".into());
    }
    ui.set_update_state(4);
    ui.set_update_progress(0.0);
    ui.set_update_detail("Baixando do GitHub e conferindo o SHA-256 de cada arquivo.".into());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        // Só repassa à janela quando o número inteiro da porcentagem muda.
        let shown = Arc::new(AtomicI32::new(-1));
        let progress = {
            let weak = weak.clone();
            move |done: u64, total: u64| {
                let fraction = if total > 0 { (done as f32 / total as f32).min(1.0) } else { 0.0 };
                let percent = (fraction * 100.0) as i32;
                if shown.swap(percent, Ordering::Relaxed) == percent {
                    return;
                }
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_update_progress(fraction);
                    }
                });
            }
        };
        let result = update::download(&release, &dir, progress).and_then(|files| update::install(&dir, &files));
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match result {
                Ok(()) => {
                    ui.set_update_state(5);
                    ui.set_update_detail("Reabrindo o Kage na versão nova.".into());
                    restart_after_update(&dir);
                }
                Err(e) => {
                    ui.set_update_state(6);
                    ui.set_update_detail(format!("{e}. O Kage continua na versão {current}.").into());
                }
            }
        });
    });
}

/// Reabre o motor (se estava rodando) e a janela com os .exe novos.
fn restart_after_update(dir: &Path) {
    if engine::is_running() {
        engine::stop();
        for _ in 0..50 {
            if !engine::is_running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = engine::start();
    }
    let _ = std::process::Command::new(dir.join("kage.exe")).spawn();
    let _ = slint::quit_event_loop();
}

fn time_label() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!("{:02}:{:02}", t.wHour, t.wMinute)
}

/// Pedido da bandeja (ou da linha de comando): qual tela abrir.
fn run_ui_command(ui: &App, state: &Shared, timer: &Rc<Timer>, command: kage_core::UiCommand) {
    use kage_core::UiCommand;
    match command {
        UiCommand::Show => {}
        UiCommand::Updates => {
            ui.set_dialog(10);
            start_update_check(ui, timer, true);
        }
        UiCommand::Customize => {
            // A bandeja pode ter trocado o wallpaper agora há pouco: lê do disco.
            let on_screen = Config::load_from(&kage_core::config_path())
                .ok()
                .and_then(|c| c.current_wallpaper().map(str::to_string))
                .filter(|id| state.borrow().config.wallpaper(id).is_some());
            match on_screen {
                Some(id) => ui.invoke_customize_wallpaper(id.into()),
                None => show_toast(ui, timer, "Nenhum wallpaper na tela para customizar"),
            }
        }
        UiCommand::News => ui.set_dialog(11),
    }
}

fn wire_updates(ui: &App, state: &Shared, timer: &Rc<Timer>) {
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let timer = timer.clone();
        ui.on_set_tray_icon(move |enabled| {
            let Some(ui) = weak.upgrade() else { return };
            state.borrow_mut().config.general.tray_icon = enabled;
            commit(&ui, &state, &timer, if enabled { "Ícone na bandeja do sistema" } else { "Ícone fora da bandeja; o Kage continua rodando" });
        });
    }
    {
        let weak = ui.as_weak();
        let timer = timer.clone();
        ui.on_check_update(move || {
            if let Some(ui) = weak.upgrade() {
                start_update_check(&ui, &timer, true);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_apply_update(move || {
            if let Some(ui) = weak.upgrade() {
                start_update_install(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let timer = timer.clone();
        ui.on_set_check_updates(move |enabled| {
            let Some(ui) = weak.upgrade() else { return };
            state.borrow_mut().config.general.check_updates = enabled;
            save_only(&ui, &state, &timer, if enabled { "O Kage vai procurar versão nova ao abrir" } else { "O Kage não procura mais versão nova sozinho" });
        });
    }
    ui.on_open_releases(|| {
        let _ = std::process::Command::new("explorer").arg(format!("https://github.com/{}/releases", update::REPO)).spawn();
    });
}
