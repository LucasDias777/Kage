//! Motor do Kage: toca vídeos, imagens e GIFs atrás dos ícones da área de trabalho.
//!
//! Uso normal: `kage-engine` sem argumentos lê `%APPDATA%\Kage\config.json` e recarrega quando a
//! janela do Kage avisa. O resto serve para teste e para a janela:
//!   kage-engine --thumbnail <arquivo> <saida.jpg>
//!   kage-engine --shrink <arquivo> <saida.jpg> <largura>   (fundo desfocado da janela)
//!   kage-engine <arquivo>                         o mesmo wallpaper em todos os monitores
//!   kage-engine --span <arquivo>                  um wallpaper esticado por todos os monitores
//!   kage-engine --screen 1=<arquivo> --screen 2=<arquivo>   um wallpaper por monitor
//! Opções:
//!   --mode fill|fit|stretch     --seconds N
//!   --fullscreen pause|continue --focus pause|continue      --pause-scope screen|all
//!   --battery pause|continue    --battery-saver pause|continue --remote pause|continue
//!   --app <programa.exe>=pause|ignore|release (pode repetir)
//!   --release-after <segundos>|off   (padrão 120)
//!   --skins                       desenha as skins ativas do Rainmeter.ini
//! Conferência de skin: kage-engine --render-skin <skin.ini> <saida.png>

// Na versão final o motor é programa de janela: iniciando com o Windows, não abre console. Chamado
// de um terminal, ele se liga ao console de quem chamou para as mensagens continuarem aparecendo.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod from_config;
mod gpu;
mod image;
mod layout;
mod media;
mod monitors;
mod now_playing;
mod pause;
mod skinhost;
mod skins;
mod surface;
mod sysinfo;
mod thumb;
mod tray;
mod video;
mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, RECT};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Media::MediaFoundation::{MF_VERSION, MFSTARTUP_FULL, MFShutdown, MFStartup};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, PostMessageW, QS_ALLINPUT,
    TranslateMessage,
};
use windows_core::BOOL;

use media::Source;
use pause::{AppAction, Reason, Rule, Rules, Scope};

static RUNNING: AtomicBool = AtomicBool::new(true);

const DEFAULT_RELEASE_AFTER: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitMode {
    Fill,
    Fit,
    Stretch,
}

/// Como os wallpapers se distribuem pelos monitores: Tela, Span ou Duplicar.
#[derive(Clone, Debug, PartialEq)]
enum Arrangement {
    /// O mesmo wallpaper em todos os monitores.
    Duplicate(PathBuf),
    /// Um wallpaper esticado pela tela virtual inteira.
    Span(PathBuf),
    /// Um wallpaper por monitor, pelo número dele.
    PerScreen(Vec<(usize, PathBuf)>),
}

/// De onde vêm as skins no modo de teste. No modo normal elas vêm do config.json, pelo `SkinHost`.
#[derive(Clone, Debug, PartialEq)]
enum SkinSource {
    None,
    /// As ativas no Rainmeter.ini (opção --skins).
    Rainmeter,
}

#[derive(Debug, PartialEq)]
struct Args {
    arrangement: Arrangement,
    mode: FitMode,
    seconds: Option<u64>,
    rules: Rules,
    release_after: Option<Duration>,
    skins: SkinSource,
    /// Ajustes do "Customizar" de cada arquivo.
    looks: Vec<(PathBuf, kage_core::Look)>,
}

/// O que a cena faz quando a janela do Kage manda recarregar: aplica as skins na hora e diz se
/// o wallpaper mudou (aí a cena é refeita).
type OnReload<'a> = dyn FnMut(&mut skinhost::SkinHost) -> bool + 'a;

/// Por que a cena terminou.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Reload,
    Quit,
    /// O driver de vídeo foi trocado ou reiniciado: a GPU precisa ser criada de novo.
    GpuLost,
}

/// Espera entre tentativas de criar a GPU de novo enquanto o driver de vídeo termina de instalar.
const GPU_RETRY: Duration = Duration::from_secs(2);
/// Pausa depois de um erro das skins, para não tentar de novo sem parar.
const ERROR_PAUSE: Duration = Duration::from_secs(1);

fn exit_code(result: windows_core::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Erro: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    #[cfg(not(debug_assertions))]
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.is_empty() {
        return exit_code(run_from_config());
    }
    if matches!(raw.first().map(String::as_str), Some("--thumbnail" | "--shrink")) {
        let (Some(input), Some(output)) = (raw.get(1), raw.get(2)) else {
            eprintln!("Uso: kage-engine --thumbnail <arquivo> <saida.jpg> | --shrink <arquivo> <saida.jpg> <largura>");
            return ExitCode::from(2);
        };
        let width = raw.get(3).and_then(|w| w.parse().ok()).filter(|_| raw[0] == "--shrink");
        return exit_code(with_media_foundation(|| {
            let wic = unsafe {
                windows::Win32::System::Com::CoCreateInstance(
                    &windows::Win32::Graphics::Imaging::CLSID_WICImagingFactory,
                    None,
                    windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
                )?
            };
            match width {
                Some(width) => thumb::make_sized(&wic, Path::new(input), Path::new(output), width),
                None => thumb::make(&wic, Path::new(input), Path::new(output)),
            }
        }));
    }
    if raw.first().map(String::as_str) == Some("--render-skin") {
        let (Some(ini), Some(out)) = (raw.get(1), raw.get(2)) else {
            eprintln!("Uso: kage-engine --render-skin <skin.ini> <saida.png>");
            return ExitCode::from(2);
        };
        return match with_media_foundation(|| render_skin(Path::new(ini), Path::new(out))) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Erro: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let args = match parse_args(raw.into_iter()) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "Uso: kage-engine <arquivo> | --span <arquivo> | --screen N=<arquivo> ... [opções]\n\
                 Veja as opções no topo de src/main.rs."
            );
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Erro: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_rule(value: Option<String>, option: &str) -> Result<Rule, String> {
    match value.as_deref() {
        Some("pause") => Ok(Rule::Pause),
        Some("continue") => Ok(Rule::Continue),
        other => Err(format!("{option} aceita pause ou continue, não {other:?}")),
    }
}

fn parse_args(mut raw: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut duplicate = None;
    let mut span = None;
    let mut screens: Vec<(usize, PathBuf)> = Vec::new();
    let mut mode = FitMode::Fill;
    let mut seconds = None;
    let mut rules = Rules::default();
    let mut release_after = Some(DEFAULT_RELEASE_AFTER);
    let mut skins = SkinSource::None;

    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "--span" => span = Some(PathBuf::from(raw.next().ok_or("--span precisa de um arquivo")?)),
            "--screen" => {
                let value = raw.next().ok_or("--screen precisa de N=arquivo")?;
                let (number, file) = value.split_once('=').ok_or(format!("use N=arquivo em vez de {value}"))?;
                let number = number.trim().parse().map_err(|_| format!("monitor inválido: {number}"))?;
                screens.push((number, PathBuf::from(file)));
            }
            "--mode" => {
                mode = match raw.next().as_deref() {
                    Some("fill") => FitMode::Fill,
                    Some("fit") => FitMode::Fit,
                    Some("stretch") => FitMode::Stretch,
                    other => return Err(format!("modo inválido: {other:?}")),
                };
            }
            "--seconds" => {
                let value = raw.next().ok_or("--seconds precisa de um número")?;
                seconds = Some(value.parse().map_err(|_| format!("segundos inválidos: {value}"))?);
            }
            "--fullscreen" => rules.fullscreen = parse_rule(raw.next(), "--fullscreen")?,
            "--focus" => rules.focus = parse_rule(raw.next(), "--focus")?,
            "--battery" => rules.battery = parse_rule(raw.next(), "--battery")?,
            "--battery-saver" => rules.battery_saver = parse_rule(raw.next(), "--battery-saver")?,
            "--remote" => rules.remote_desktop = parse_rule(raw.next(), "--remote")?,
            "--pause-scope" => {
                rules.scope = match raw.next().as_deref() {
                    Some("screen") => Scope::PerScreen,
                    Some("all") => Scope::All,
                    other => return Err(format!("--pause-scope aceita screen ou all, não {other:?}")),
                };
            }
            "--app" => {
                let value = raw.next().ok_or("--app precisa de programa.exe=ação")?;
                let (exe, action) = value.split_once('=').ok_or(format!("use programa.exe=ação em vez de {value}"))?;
                let action = match action {
                    "pause" => AppAction::Pause,
                    "ignore" => AppAction::Ignore,
                    "release" => AppAction::Release,
                    other => return Err(format!("ação de app inválida: {other}")),
                };
                rules.apps.push((exe.trim().to_ascii_lowercase(), action));
            }
            "--skins" => skins = SkinSource::Rainmeter,
            "--release-after" => {
                let value = raw.next().ok_or("--release-after precisa de segundos ou off")?;
                release_after = match value.as_str() {
                    "off" => None,
                    n => Some(Duration::from_secs(n.parse().map_err(|_| format!("segundos inválidos: {n}"))?)),
                };
            }
            _ if arg.starts_with("--") => return Err(format!("opção desconhecida: {arg}")),
            _ if duplicate.is_none() => duplicate = Some(PathBuf::from(arg)),
            _ => return Err(format!("argumento a mais: {arg}")),
        }
    }

    let arrangement = match (duplicate, span, screens.is_empty()) {
        (Some(file), None, true) => Arrangement::Duplicate(file),
        (None, Some(file), true) => Arrangement::Span(file),
        (None, None, false) => Arrangement::PerScreen(screens),
        (None, None, true) => return Err("informe um arquivo".into()),
        _ => return Err("escolha só uma disposição: arquivo, --span ou --screen".into()),
    };
    for file in arrangement_files(&arrangement) {
        if !file.is_file() {
            return Err(format!("arquivo não encontrado: {}", file.display()));
        }
        if media::kind_of(file).is_none() {
            return Err(format!("formato não suportado: {}", file.display()));
        }
    }
    Ok(Args { arrangement, mode, seconds, rules, release_after, skins, looks: Vec::new() })
}

fn arrangement_files(arrangement: &Arrangement) -> Vec<&Path> {
    match arrangement {
        Arrangement::Duplicate(file) | Arrangement::Span(file) => vec![file.as_path()],
        Arrangement::PerScreen(list) => list.iter().map(|(_, file)| file.as_path()).collect(),
    }
}

fn run(args: &Args) -> windows_core::Result<()> {
    with_media_foundation(|| {
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(on_ctrl_c), true);
        }
        let _window = watch::SystemWindow::create()?;
        let gpu = gpu::Gpu::new()?;
        let mut host = skinhost::SkinHost::new(false)?;
        if args.skins == SkinSource::Rainmeter {
            host.apply(&skinhost::rainmeter_entries(), &[], &monitors::enumerate());
        }
        play(args, &gpu, &mut host, &mut |_| false).map(|_| ())
    })
}

fn load_config() -> kage_core::Config {
    kage_core::Config::load_from(&kage_core::config_path()).unwrap_or_else(|e| {
        log(&e);
        kage_core::Config::default()
    })
}

/// As skins que o motor deve abrir (nenhuma com o desenho de skins desligado).
fn skin_list(config: &kage_core::Config) -> Vec<kage_core::SkinEntry> {
    if config.skins.enabled { config.skins.items.clone() } else { Vec::new() }
}

/// Modo normal: lê a configuração, toca, e aplica o que a janela do Kage mudar. As skins vivem
/// fora da cena do wallpaper: mudar só skin não reinicia o vídeo, e sem wallpaper elas continuam.
fn run_from_config() -> windows_core::Result<()> {
    if let Some(engine) = watch::running_engine() {
        unsafe {
            let _ = PostMessageW(Some(engine), kage_core::MSG_RELOAD, WPARAM(0), LPARAM(0));
        }
        println!("O motor do Kage já está rodando; pedi para ele recarregar a configuração.");
        return Ok(());
    }
    with_media_foundation(|| {
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(on_ctrl_c), true);
        }
        let window = watch::SystemWindow::create()?;
        log(&format!("Motor do Kage iniciado. Configuração: {}", kage_core::config_path().display()));
        let mut gpu = gpu::Gpu::new()?;
        let mut host = skinhost::SkinHost::new(true)?;
        let mut config = load_config();
        tray::init(window.hwnd());
        tray::set_visible(config.general.tray_icon);
        loop {
            if gpu.is_lost() {
                match recreate_gpu(&mut host) {
                    Some(fresh) => gpu = fresh,
                    None => break,
                }
            }
            let monitors = monitors::enumerate();
            if adopt_monitor_ids(&config, &monitors) {
                config = load_config();
            }
            host.apply(&skin_list(&config), &config.skins.favorites, &monitors);
            let scene = from_config::args(&config, &monitors);
            let mut fresh = None;
            let outcome = {
                let mut on_reload = |host: &mut skinhost::SkinHost| {
                    let next = load_config();
                    tray::set_visible(next.general.tray_icon);
                    let monitors = monitors::enumerate();
                    host.apply(&skin_list(&next), &next.skins.favorites, &monitors);
                    let rebuild = from_config::args(&next, &monitors) != scene;
                    fresh = Some(next);
                    rebuild
                };
                match &scene {
                    Some(args) => match play(args, &gpu, &mut host, &mut on_reload) {
                        Ok(outcome) => outcome,
                        Err(_) if gpu.is_lost() => Outcome::GpuLost,
                        Err(e) => {
                            log(&format!("Erro: {e}"));
                            idle(&gpu, &mut host, &mut on_reload)
                        }
                    },
                    None => {
                        log(&format!("Nenhum wallpaper escolhido; {} skin(s) na tela.", host.len()));
                        idle(&gpu, &mut host, &mut on_reload)
                    }
                }
            };
            if let Some(next) = fresh {
                config = next;
            }
            match outcome {
                Outcome::Quit => break,
                Outcome::Reload => log("Configuração mudou; refazendo a cena do wallpaper."),
                Outcome::GpuLost => {}
            }
        }
        tray::remove();
        log("Motor do Kage encerrado.");
        Ok(())
    })
}

/// Cria a GPU de novo depois que o driver de vídeo foi trocado ou reiniciado. Enquanto o driver
/// termina de instalar a criação pode falhar: tenta de novo a cada `GPU_RETRY`, atendendo as
/// mensagens. `None` quando pedem para encerrar no meio.
fn recreate_gpu(host: &mut skinhost::SkinHost) -> Option<gpu::Gpu> {
    log("A placa de vídeo reiniciou (driver trocado ou recuperado); refazendo a GPU, o wallpaper e as skins.");
    host.release_gpu();
    let mut last_error = None;
    loop {
        pump_messages();
        if !RUNNING.load(Ordering::SeqCst) || watch::quit_requested() {
            return None;
        }
        match gpu::Gpu::new() {
            Ok(fresh) if !fresh.is_lost() => {
                log("GPU pronta de novo.");
                return Some(fresh);
            }
            Ok(_) => {}
            Err(e) => {
                let message = format!("A GPU ainda não voltou: {e}");
                if last_error.as_ref() != Some(&message) {
                    log(&message);
                    last_error = Some(message);
                }
            }
        }
        unsafe {
            MsgWaitForMultipleObjects(None, false, GPU_RETRY.as_millis() as u32, QS_ALLINPUT);
        }
    }
}

/// Grava no config a identificação estável dos monitores no lugar do nome do Windows
/// (`\\.\DISPLAY1`), que muda numa troca de driver. Verdadeiro quando gravou.
fn adopt_monitor_ids(config: &kage_core::Config, monitors: &[monitors::Monitor]) -> bool {
    let pairs = monitors::id_pairs(monitors);
    if !config.clone().adopt_monitor_ids(&pairs) {
        return false;
    }
    match kage_core::Config::edit_file(&kage_core::config_path(), |c| {
        c.adopt_monitor_ids(&pairs);
    }) {
        Ok(_) => {
            log("Monitores agora guardados pelo modelo da tela, que não muda com o driver de vídeo.");
            true
        }
        Err(e) => {
            log(&format!("Não gravou a identificação dos monitores: {e}"));
            false
        }
    }
}

/// Sem wallpaper: só as skins, até a janela do Kage mandar recarregar ou encerrar.
fn idle(gpu: &gpu::Gpu, host: &mut skinhost::SkinHost, on_reload: &mut OnReload) -> Outcome {
    let mut last_error: Option<String> = None;
    loop {
        pump_messages();
        if !RUNNING.load(Ordering::SeqCst) || watch::quit_requested() {
            return Outcome::Quit;
        }
        if watch::take_reload() && on_reload(host) {
            return Outcome::Reload;
        }
        let now = Instant::now();
        let failed = match host.tick(gpu, now, &[]) {
            Ok(()) => {
                last_error = None;
                false
            }
            Err(_) if gpu.is_lost() => return Outcome::GpuLost,
            Err(e) => {
                // O mesmo erro uma vez só no log, e uma pausa antes de tentar de novo: a skin que
                // falhou continua pendente e, sem a pausa, o laço giraria sem parar.
                let message = format!("Skins: {e}");
                if last_error.as_ref() != Some(&message) {
                    log(&message);
                    last_error = Some(message);
                }
                true
            }
        };
        let wake = if failed { now + ERROR_PAUSE } else { host.next_wake(now, &[]).min(now + Duration::from_secs(1)) };
        let timeout = wake.saturating_duration_since(Instant::now()).as_millis() as u32;
        unsafe {
            MsgWaitForMultipleObjects(None, false, timeout, QS_ALLINPUT);
        }
    }
}

fn with_media_foundation(work: impl FnOnce() -> windows_core::Result<()>) -> windows_core::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
    }
    let result = work();
    unsafe {
        let _ = MFShutdown();
        CoUninitialize();
    }
    result
}

fn render_skin(ini: &Path, out: &Path) -> windows_core::Result<()> {
    let gpu = gpu::Gpu::new()?;
    let mut skin =
        kage_skins::Skin::load(ini, &skinhost::skins_root_for(ini)).map_err(|e| windows_core::Error::new(E_FAIL, e))?;
    let (w, h) = skins::render_to_png(&gpu, &mut skin, out)?;
    println!("Skin {} desenhada em {w}x{h}: {}", skin.config, out.display());
    Ok(())
}

/// Motivos de pausa em que o monitor nem aparece: não adianta atualizar skin nele.
fn hides(reason: &Reason) -> bool {
    matches!(reason, Reason::Covered | Reason::Fullscreen | Reason::Locked)
}

fn play(
    args: &Args,
    gpu: &gpu::Gpu,
    host: &mut skinhost::SkinHost,
    on_reload: &mut OnReload,
) -> windows_core::Result<Outcome> {
    let all = monitors::enumerate();
    for m in &all {
        log(&format!(
            "Monitor {}: {} ({}) {}x{}{}",
            m.number,
            m.device_name,
            m.id,
            m.width(),
            m.height(),
            if m.primary { " (principal)" } else { "" }
        ));
    }

    // Cada arquivo abre uma fonte só, mesmo que apareça em vários monitores.
    let assignments: Vec<(&monitors::Monitor, &Path)> = match &args.arrangement {
        Arrangement::Duplicate(file) | Arrangement::Span(file) => {
            all.iter().map(|m| (m, file.as_path())).collect()
        }
        Arrangement::PerScreen(list) => {
            let mut out = Vec::new();
            for (number, file) in list {
                let monitor = all.iter().find(|m| m.number == *number).ok_or_else(|| {
                    windows_core::Error::new(E_INVALIDARG, format!("o monitor {number} não existe"))
                })?;
                out.push((monitor, file.as_path()));
            }
            out
        }
    };

    let workerw = desktop::find_workerw()?;

    let mut sources: Vec<Source> = Vec::new();
    let mut source_files: Vec<PathBuf> = Vec::new();
    let mut index_of: HashMap<PathBuf, usize> = HashMap::new();
    let mut surfaces = Vec::new();
    for (monitor, file) in &assignments {
        let key = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        let index = match index_of.get(&key) {
            Some(&i) => i,
            None => {
                sources.push(Source::open(gpu, file)?);
                source_files.push(file.to_path_buf());
                index_of.insert(key, sources.len() - 1);
                sources.len() - 1
            }
        };
        surfaces.push(surface::Surface::new(gpu, workerw, monitor, index)?);
    }

    // "Customizar": velocidade na fonte, cores na tela.
    let look_of = |file: &Path| args.looks.iter().find(|(p, _)| p == file).map(|(_, l)| *l).unwrap_or_default();
    for (source, file) in sources.iter_mut().zip(&source_files) {
        let look = look_of(file);
        if look != kage_core::Look::default() {
            log(&format!(
                "Customizar {}: velocidade {}%, brilho {}%, contraste {}%, saturação {}%",
                file.display(),
                look.speed,
                look.brightness,
                look.contrast,
                look.saturation
            ));
        }
        match source {
            Source::Video(video) => video.rate = (look.speed / 100.0).clamp(0.25, 4.0) as f64,
            Source::Gif(gif) => gif.speed = look.speed / 100.0,
            Source::Still(_) => {}
        }
    }
    for s in &mut surfaces {
        s.set_colors(gpu, &look_of(&source_files[s.source]))?;
    }

    wait_until_ready(&sources)?;

    // Encaixe de cada tela; no Span o canvas é a tela virtual inteira.
    let span_canvas = layout::bounding(surfaces.iter().map(|s| s.monitor.rect));
    for s in &mut surfaces {
        let size = sources[s.source].size().unwrap_or((1920, 1080));
        let canvas = match args.arrangement {
            Arrangement::Span(_) => span_canvas,
            _ => s.monitor.rect,
        };
        s.placement = layout::clip(&layout::place(size, canvas, args.mode), s.monitor.rect);
    }

    for (i, source) in sources.iter().enumerate() {
        let (w, h) = source.size().unwrap_or((0, 0));
        let kind = match source {
            Source::Video(_) => "vídeo",
            Source::Still(_) => "imagem",
            Source::Gif(_) => "GIF",
        };
        let screens: Vec<String> =
            surfaces.iter().filter(|s| s.source == i).map(|s| s.monitor.number.to_string()).collect();
        log(&format!("Fonte {}: {kind} {w}x{h} no(s) monitor(es) {}", i + 1, screens.join(", ")));
    }
    log(&format!(
        "Disposição {}, modo {:?}, liberar memória {}",
        arrangement_name(&args.arrangement),
        args.mode,
        match args.release_after {
            Some(d) => format!("após {} s pausado", d.as_secs()),
            None => "nunca".into(),
        }
    ));

    let monitor_rects: Vec<RECT> = surfaces.iter().map(|s| s.monitor.rect).collect();
    let mut watcher = watch::Watcher::new(!args.rules.apps.is_empty())?;
    let mut reasons: Vec<Option<Reason>> = vec![None; surfaces.len()];
    let mut drawn_once = vec![false; surfaces.len()];
    let mut paused_since: Vec<Option<Instant>> = vec![None; sources.len()];
    let mut release_now = false;
    let mut first_decision = true;
    let mut trims = TrimSchedule::default();

    let started = Instant::now();
    let deadline = args.seconds.map(|s| started + Duration::from_secs(s));
    let mut presented = 0u64;
    let mut outcome = Outcome::Quit;

    while RUNNING.load(Ordering::SeqCst) {
        pump_messages();
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d) || watch::quit_requested() {
            break;
        }
        if watch::take_reload() {
            if on_reload(host) {
                outcome = Outcome::Reload;
                break;
            }
            log("Configuração mudou sem mexer no wallpaper; a cena continua.");
        }
        if watcher.display_changed() {
            // Monitor ligado, desligado ou com resolução nova: monta as telas de novo.
            log("Os monitores mudaram; reorganizando as telas.");
            outcome = Outcome::Reload;
            break;
        }

        // 1. Decide quem toca, a partir da foto mais recente das janelas.
        if watcher.due(now) {
            let decision = pause::evaluate(&monitor_rects, &watcher.snapshot(), &args.rules);
            for (j, reason) in decision.per_monitor.into_iter().enumerate() {
                if first_decision || reason != reasons[j] {
                    let number = surfaces[j].monitor.number;
                    match &reason {
                        Some(r) => log(&format!("Monitor {number}: pausado ({})", r.describe())),
                        None => log(&format!("Monitor {number}: tocando")),
                    }
                }
                reasons[j] = reason;
            }
            release_now = decision.release_now;
            first_decision = false;
        }

        // 2. Aplica nas fontes: uma fonte toca se algum monitor dela está tocando.
        for (i, source) in sources.iter_mut().enumerate() {
            // "Pausar wallpapers" da bandeja vale para todas as telas, até tirar a marca.
            let wanted = !release_now
                && !tray::user_paused()
                && surfaces.iter().enumerate().any(|(j, s)| s.source == i && reasons[j].is_none());
            match source {
                Source::Video(video) => {
                    let was_released = video.is_released();
                    video.set_playing(gpu, wanted)?;
                    if wanted && was_released {
                        log(&format!("Fonte {}: reabrindo do ponto {:.1} s", i + 1, video.resume_position()));
                    }
                    if video.is_playing() {
                        paused_since[i] = None;
                    } else {
                        let since = *paused_since[i].get_or_insert(now);
                        let expired = args.release_after.is_some_and(|d| now.duration_since(since) >= d);
                        if release_now || expired {
                            let before = (gpu.video_memory_mb(), private_memory_mb());
                            if video.release() {
                                log(&format!(
                                    "Fonte {}: decodificador liberado (posição {:.1} s guardada)",
                                    i + 1,
                                    video.resume_position()
                                ));
                                trims.schedule(now, before);
                            }
                        }
                    }
                }
                Source::Gif(gif) => gif.set_playing(wanted, now),
                Source::Still(_) => {}
            }
        }

        // O desligamento do decodificador termina em outra thread; a limpeza da GPU só
        // devolve a memória depois disso, por isso roda mais de uma vez.
        if let Some(before) = trims.run_due(now, gpu) {
            log(&format!(
                "Memória devolvida: VRAM {} → {} MB, RAM privada {} → {} MB",
                before.0,
                gpu.video_memory_mb(),
                before.1,
                private_memory_mb()
            ));
        }

        // 3. Quadros novos.
        let mut changed = vec![false; sources.len()];
        for (i, source) in sources.iter_mut().enumerate() {
            changed[i] = match source {
                Source::Video(video) => video.tick()?,
                Source::Gif(gif) => gif.advance(now)?,
                Source::Still(_) => false,
            };
        }

        // 4. Skins, cada uma na sua janela; nos monitores cobertos por tela cheia elas não redesenham.
        let hidden: Vec<RECT> = surfaces
            .iter()
            .enumerate()
            .filter(|(j, _)| reasons[*j].as_ref().is_some_and(hides))
            .map(|(_, s)| s.monitor.rect)
            .collect();
        host.tick(gpu, now, &hidden)?;

        // 5. Desenha o conteúdo novo. Imagem e GIF aparecem pelo menos uma vez, mesmo pausados.
        for (j, s) in surfaces.iter_mut().enumerate() {
            let content_changed = changed[s.source] && reasons[j].is_none();
            if drawn_once[j] && !content_changed {
                continue;
            }
            let drew = match &sources[s.source] {
                Source::Video(video) => match video.player() {
                    Some(player) if content_changed => {
                        s.draw_video(gpu, player)?;
                        true
                    }
                    _ => false,
                },
                Source::Still(img) => {
                    s.draw_bitmap(gpu, Some(&img.bitmap), img.size)?;
                    true
                }
                Source::Gif(gif) => {
                    s.draw_bitmap(gpu, Some(&gif.bitmap), gif.size)?;
                    true
                }
            };
            if drew {
                drawn_once[j] = true;
                presented += 1;
            }
        }

        // 6. Dorme até a próxima coisa que importa.
        let video_playing = sources.iter().any(|s| matches!(s, Source::Video(v) if v.is_playing()));
        if video_playing {
            unsafe {
                let _ = DwmFlush();
            }
        } else {
            let mut wake = watcher.next_wake(now);
            for (i, source) in sources.iter().enumerate() {
                match source {
                    Source::Gif(gif) if gif.playing => wake = wake.min(gif.next_due()),
                    Source::Video(v) if !v.is_released() => {
                        if let (Some(since), Some(after)) = (paused_since[i], args.release_after) {
                            wake = wake.min(since + after);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(d) = deadline {
                wake = wake.min(d);
            }
            if let Some(t) = trims.next() {
                wake = wake.min(t);
            }
            wake = wake.min(host.next_wake(now, &hidden));
            let timeout = wake.saturating_duration_since(Instant::now()).as_millis() as u32;
            unsafe {
                MsgWaitForMultipleObjects(None, false, timeout, QS_ALLINPUT);
            }
        }
    }

    let elapsed = started.elapsed().as_secs_f64();
    log(&format!(
        "Fim: {presented} quadros apresentados em {elapsed:.1} s ({:.1} por segundo, somando as telas)",
        presented as f64 / elapsed.max(0.001)
    ));
    drop(watcher);
    drop(surfaces);
    drop(sources);
    // A GPU continua para as skins e a próxima cena; a memória desta volta para o driver.
    gpu.trim();
    desktop::refresh_wallpaper();
    Ok(outcome)
}

fn arrangement_name(arrangement: &Arrangement) -> &'static str {
    match arrangement {
        Arrangement::Duplicate(_) => "Duplicar",
        Arrangement::Span(_) => "Span",
        Arrangement::PerScreen(_) => "Tela",
    }
}

/// Espera os vídeos lerem o cabeçalho para saber o tamanho de cada um.
fn wait_until_ready(sources: &[Source]) -> windows_core::Result<()> {
    let opened = Instant::now();
    while !sources.iter().all(Source::is_ready) {
        for source in sources {
            if let Source::Video(video) = source
                && let Some(player) = video.player()
                && let Some(hr) = player.error()
            {
                return Err(windows_core::Error::new(hr, "o Media Engine não abriu o vídeo"));
            }
        }
        if opened.elapsed() > Duration::from_secs(10) {
            for source in sources {
                if let Source::Video(video) = source
                    && let Some(player) = video.player()
                {
                    let (ready, network, code) = player.debug_state();
                    eprintln!("Estado do Media Engine: ready={ready} network={network} erro={code}");
                }
            }
            return Err(windows_core::Error::new(E_FAIL, "o vídeo não carregou em 10 s"));
        }
        pump_messages();
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Limpezas da GPU agendadas depois de soltar um decodificador.
#[derive(Default)]
struct TrimSchedule {
    pending: Vec<Instant>,
    /// (VRAM, RAM privada) antes da liberação, para o registro final.
    before: Option<(u64, u64)>,
}

impl TrimSchedule {
    const DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];

    fn schedule(&mut self, now: Instant, before: (u64, u64)) {
        self.pending = Self::DELAYS.iter().map(|d| now + *d).collect();
        self.before.get_or_insert(before);
    }

    fn next(&self) -> Option<Instant> {
        self.pending.first().copied()
    }

    /// Roda a limpeza que venceu. Na última, devolve a medida de antes para o registro.
    fn run_due(&mut self, now: Instant, gpu: &gpu::Gpu) -> Option<(u64, u64)> {
        if self.pending.first().is_none_or(|t| now < *t) {
            return None;
        }
        self.pending.remove(0);
        gpu.trim();
        if self.pending.is_empty() { self.before.take() } else { None }
    }
}

/// Memória privada (comprometida) do processo, em MB.
fn private_memory_mb() -> u64 {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
    };
    if ok.as_bool() { counters.PrivateUsage as u64 / (1024 * 1024) } else { 0 }
}

/// Registro no console e em `%APPDATA%\Kage\engine.log` (o motor aberto pela janela não tem console).
fn log(message: &str) {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};

    static FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
    let t = unsafe { GetLocalTime() };
    let line = format!("[{:02}:{:02}:{:02}] {message}", t.wHour, t.wMinute, t.wSecond);
    println!("{line}");
    let file = FILE.get_or_init(|| {
        let path = kage_core::engine_log_path();
        let _ = std::fs::create_dir_all(kage_core::data_dir());
        // Recomeça o arquivo quando passa de 1 MB.
        let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(!too_big)
            .truncate(too_big)
            .open(path)
            .ok();
        Mutex::new(file)
    });
    if let Ok(mut guard) = file.lock()
        && let Some(f) = guard.as_mut()
    {
        let _ = writeln!(f, "{line}");
    }
}

fn pump_messages() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn on_ctrl_c(_: u32) -> BOOL {
    RUNNING.store(false, Ordering::SeqCst);
    BOOL(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    /// Cria arquivos vazios com as extensões pedidas numa pasta temporária do teste.
    fn temp_files(names: &[&str]) -> Vec<String> {
        let dir = std::env::temp_dir().join(format!("kage-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        names
            .iter()
            .map(|n| {
                let path = dir.join(n);
                std::fs::write(&path, b"").unwrap();
                path.to_string_lossy().to_string()
            })
            .collect()
    }

    #[test]
    fn single_file_means_duplicate() {
        let f = temp_files(&["a.mp4"]);
        let parsed = parse_args(args(&[&f[0], "--seconds", "5"])).unwrap();
        assert_eq!(parsed.arrangement, Arrangement::Duplicate(PathBuf::from(&f[0])));
        assert_eq!(parsed.seconds, Some(5));
        assert_eq!(parsed.rules, Rules::default());
        assert_eq!(parsed.release_after, Some(DEFAULT_RELEASE_AFTER));
    }

    #[test]
    fn span_and_mode() {
        let f = temp_files(&["b.mp4"]);
        let parsed = parse_args(args(&["--span", &f[0], "--mode", "fit"])).unwrap();
        assert_eq!(parsed.arrangement, Arrangement::Span(PathBuf::from(&f[0])));
        assert_eq!(parsed.mode, FitMode::Fit);
    }

    #[test]
    fn one_file_per_screen() {
        let f = temp_files(&["c.mp4", "d.png"]);
        let one = format!("1={}", f[0]);
        let two = format!("2={}", f[1]);
        let parsed = parse_args(args(&["--screen", &one, "--screen", &two])).unwrap();
        assert_eq!(
            parsed.arrangement,
            Arrangement::PerScreen(vec![(1, PathBuf::from(&f[0])), (2, PathBuf::from(&f[1]))])
        );
    }

    #[test]
    fn pause_options() {
        let f = temp_files(&["g.mp4"]);
        let parsed = parse_args(args(&[
            &f[0],
            "--focus",
            "pause",
            "--fullscreen",
            "continue",
            "--pause-scope",
            "all",
            "--app",
            "OBS64.exe=release",
            "--app",
            "code.exe=ignore",
            "--release-after",
            "off",
        ]))
        .unwrap();
        assert_eq!(parsed.rules.focus, Rule::Pause);
        assert_eq!(parsed.rules.fullscreen, Rule::Continue);
        assert_eq!(parsed.rules.scope, Scope::All);
        assert_eq!(
            parsed.rules.apps,
            vec![("obs64.exe".to_string(), AppAction::Release), ("code.exe".to_string(), AppAction::Ignore)]
        );
        assert_eq!(parsed.release_after, None);
    }

    #[test]
    fn rejects_mixed_arrangements_and_bad_input() {
        let f = temp_files(&["e.mp4", "f.txt"]);
        assert!(parse_args(args(&[])).is_err());
        assert!(parse_args(args(&[&f[0], "--span", &f[0]])).is_err());
        assert!(parse_args(args(&[&f[0], "--mode", "zoom"])).is_err());
        assert!(parse_args(args(&[&f[1]])).is_err(), "extensão .txt não é mídia");
        assert!(parse_args(args(&["--screen", "1"])).is_err(), "falta o =arquivo");
        assert!(parse_args(args(&[&f[0], "--focus", "talvez"])).is_err());
        assert!(parse_args(args(&[&f[0], "--app", "x.exe=kill"])).is_err());
    }
}
