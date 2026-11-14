//! Decide, por monitor, se o wallpaper toca ou pausa.
//!
//! Esta parte é lógica pura sobre uma "foto" das janelas e do sistema; quem tira a foto é o
//! módulo `watch`. Assim as regras são testáveis sem o Windows.

use windows::Win32::Foundation::RECT;

/// O que fazer quando a condição acontece (aba "Desempenho" das configurações).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Pause,
    Continue,
}

/// Regra por aplicativo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppAction {
    /// Pausa todos os wallpapers enquanto o app tem janela visível.
    Pause,
    /// O app nunca pausa o wallpaper (nem por foco, tela cheia ou por cobrir a tela).
    Ignore,
    /// Pausa e libera a memória na hora, enquanto o app tem janela visível.
    Release,
}

/// Pausar só o monitor afetado ou todos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    PerScreen,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub fullscreen: Rule,
    pub focus: Rule,
    pub battery: Rule,
    pub battery_saver: Rule,
    pub remote_desktop: Rule,
    pub scope: Scope,
    /// Nome do executável em minúsculas, sem caminho (ex.: "code.exe").
    pub apps: Vec<(String, AppAction)>,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            fullscreen: Rule::Pause,
            focus: Rule::Continue,
            battery: Rule::Pause,
            battery_saver: Rule::Pause,
            remote_desktop: Rule::Pause,
            scope: Scope::PerScreen,
            apps: Vec::new(),
        }
    }
}

impl Rules {
    fn app_action(&self, exe: &str) -> Option<AppAction> {
        self.apps.iter().find(|(name, _)| name.eq_ignore_ascii_case(exe)).map(|(_, a)| *a)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemState {
    pub locked: bool,
    pub remote_session: bool,
    pub on_battery: bool,
    pub battery_saver: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    /// Limites visíveis da janela, em coordenadas de tela.
    pub bounds: RECT,
    /// Executável em minúsculas; vazio quando não foi preciso descobrir.
    pub exe: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Foreground {
    pub window: WindowInfo,
    /// Cobre o monitor inteiro, ou o Windows avisou de jogo/apresentação em tela cheia.
    pub fullscreen: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Janelas visíveis que podem cobrir o wallpaper (sem a área de trabalho).
    pub windows: Vec<WindowInfo>,
    /// Janela em foco, quando não é a área de trabalho nem a barra de tarefas.
    pub foreground: Option<Foreground>,
    pub system: SystemState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    Locked,
    RemoteDesktop,
    Battery,
    BatterySaver,
    App(String),
    Fullscreen,
    Focus,
    Covered,
}

impl Reason {
    pub fn describe(&self) -> String {
        match self {
            Self::Locked => "tela bloqueada".into(),
            Self::RemoteDesktop => "área de trabalho remota".into(),
            Self::Battery => "na bateria".into(),
            Self::BatterySaver => "economia de bateria".into(),
            Self::App(exe) => format!("regra do app {exe}"),
            Self::Fullscreen => "app em tela cheia".into(),
            Self::Focus => "app em foco".into(),
            Self::Covered => "coberto por janelas".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// Motivo da pausa de cada monitor, na mesma ordem recebida; `None` = toca.
    pub per_monitor: Vec<Option<Reason>>,
    /// Algum app com regra "liberar" está aberto.
    pub release_now: bool,
}

/// Fração mínima do monitor que precisa estar à mostra para o wallpaper continuar tocando.
const VISIBLE_THRESHOLD: f64 = 0.01;

pub fn evaluate(monitors: &[RECT], snap: &Snapshot, rules: &Rules) -> Decision {
    let n = monitors.len();
    let mut per_monitor: Vec<Option<Reason>> = vec![None; n];
    let mut release_now = false;

    // Motivos do sistema valem para todos os monitores.
    let sys = snap.system;
    let global = if sys.locked {
        Some(Reason::Locked)
    } else if sys.remote_session && rules.remote_desktop == Rule::Pause {
        Some(Reason::RemoteDesktop)
    } else if sys.battery_saver && rules.battery_saver == Rule::Pause {
        Some(Reason::BatterySaver)
    } else if sys.on_battery && rules.battery == Rule::Pause {
        Some(Reason::Battery)
    } else {
        None
    };
    if let Some(reason) = global {
        return Decision { per_monitor: vec![Some(reason); n], release_now };
    }

    // Regras por aplicativo: qualquer janela visível do app conta.
    for w in &snap.windows {
        match rules.app_action(&w.exe) {
            Some(AppAction::Pause) => return Decision { per_monitor: vec![Some(Reason::App(w.exe.clone())); n], release_now },
            Some(AppAction::Release) => {
                release_now = true;
                return Decision { per_monitor: vec![Some(Reason::App(w.exe.clone())); n], release_now };
            }
            _ => {}
        }
    }

    // Janela em foco: tela cheia ou foco, no monitor dela ou em todos.
    if let Some(fg) = &snap.foreground
        && rules.app_action(&fg.window.exe) != Some(AppAction::Ignore)
        && let Some(index) = monitor_of(monitors, &fg.window.bounds)
    {
        let reason = if fg.fullscreen && rules.fullscreen == Rule::Pause {
            Some(Reason::Fullscreen)
        } else if rules.focus == Rule::Pause {
            Some(Reason::Focus)
        } else {
            None
        };
        if let Some(reason) = reason {
            match rules.scope {
                Scope::PerScreen => per_monitor[index] = Some(reason),
                Scope::All => per_monitor.iter_mut().for_each(|m| *m = Some(reason.clone())),
            }
        }
    }

    // Monitor escondido atrás de janelas: tocar seria desperdício.
    for (i, monitor) in monitors.iter().enumerate() {
        if per_monitor[i].is_some() {
            continue;
        }
        let covers: Vec<RECT> = snap
            .windows
            .iter()
            .filter(|w| rules.app_action(&w.exe) != Some(AppAction::Ignore))
            .map(|w| w.bounds)
            .collect();
        let total = area(monitor) as f64;
        if total > 0.0 && (uncovered_area(*monitor, &covers) as f64) / total < VISIBLE_THRESHOLD {
            per_monitor[i] = Some(Reason::Covered);
        }
    }

    Decision { per_monitor, release_now }
}

/// Monitor que contém a maior parte do retângulo.
pub fn monitor_of(monitors: &[RECT], r: &RECT) -> Option<usize> {
    monitors
        .iter()
        .enumerate()
        .map(|(i, m)| (i, area(&intersect(m, r))))
        .filter(|(_, a)| *a > 0)
        .max_by_key(|(_, a)| *a)
        .map(|(i, _)| i)
}

/// Área do monitor que nenhuma das janelas cobre.
pub fn uncovered_area(monitor: RECT, covers: &[RECT]) -> i64 {
    let mut pieces = vec![monitor];
    for c in covers {
        pieces = pieces.into_iter().flat_map(|p| subtract(p, c)).collect();
        if pieces.is_empty() {
            return 0;
        }
    }
    pieces.iter().map(area).sum()
}

/// `a` menos `b`, em até quatro retângulos sem sobreposição.
fn subtract(a: RECT, b: &RECT) -> Vec<RECT> {
    let i = intersect(&a, b);
    if area(&i) == 0 {
        return vec![a];
    }
    let mut out = Vec::with_capacity(4);
    if a.top < i.top {
        out.push(RECT { left: a.left, top: a.top, right: a.right, bottom: i.top });
    }
    if i.bottom < a.bottom {
        out.push(RECT { left: a.left, top: i.bottom, right: a.right, bottom: a.bottom });
    }
    if a.left < i.left {
        out.push(RECT { left: a.left, top: i.top, right: i.left, bottom: i.bottom });
    }
    if i.right < a.right {
        out.push(RECT { left: i.right, top: i.top, right: a.right, bottom: i.bottom });
    }
    out
}

fn intersect(a: &RECT, b: &RECT) -> RECT {
    RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    }
}

fn area(r: &RECT) -> i64 {
    let w = (r.right - r.left).max(0) as i64;
    let h = (r.bottom - r.top).max(0) as i64;
    w * h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    // Dois monitores: principal 1440p e 1080p à esquerda.
    fn monitors() -> Vec<RECT> {
        vec![rect(0, 0, 2560, 1440), rect(-1920, 0, 0, 1080)]
    }

    fn window(bounds: RECT, exe: &str) -> WindowInfo {
        WindowInfo { bounds, exe: exe.into() }
    }

    fn maximized_with_taskbar() -> Vec<WindowInfo> {
        vec![window(rect(0, 0, 2560, 1392), "code.exe"), window(rect(0, 1392, 2560, 1440), "explorer.exe")]
    }

    #[test]
    fn empty_desktop_plays_everywhere() {
        let d = evaluate(&monitors(), &Snapshot::default(), &Rules::default());
        assert_eq!(d.per_monitor, vec![None, None]);
    }

    #[test]
    fn maximized_window_plus_taskbar_covers_only_its_monitor() {
        let snap = Snapshot { windows: maximized_with_taskbar(), ..Default::default() };
        let d = evaluate(&monitors(), &snap, &Rules::default());
        assert_eq!(d.per_monitor, vec![Some(Reason::Covered), None]);
    }

    #[test]
    fn small_window_does_not_pause() {
        let snap = Snapshot { windows: vec![window(rect(100, 100, 900, 700), "notepad.exe")], ..Default::default() };
        let d = evaluate(&monitors(), &snap, &Rules::default());
        assert_eq!(d.per_monitor, vec![None, None]);
    }

    #[test]
    fn fullscreen_pauses_its_monitor_or_all() {
        let game = window(rect(0, 0, 2560, 1440), "game.exe");
        let snap = Snapshot {
            windows: vec![game.clone()],
            foreground: Some(Foreground { window: game, fullscreen: true }),
            ..Default::default()
        };
        let d = evaluate(&monitors(), &snap, &Rules::default());
        assert_eq!(d.per_monitor, vec![Some(Reason::Fullscreen), None]);

        let all = Rules { scope: Scope::All, ..Rules::default() };
        let d = evaluate(&monitors(), &snap, &all);
        assert_eq!(d.per_monitor, vec![Some(Reason::Fullscreen), Some(Reason::Fullscreen)]);
    }

    #[test]
    fn focus_rule_pauses_monitor_of_focused_window() {
        let w = window(rect(-1800, 100, -1000, 600), "notepad.exe");
        let snap = Snapshot {
            windows: vec![w.clone()],
            foreground: Some(Foreground { window: w, fullscreen: false }),
            ..Default::default()
        };
        assert_eq!(evaluate(&monitors(), &snap, &Rules::default()).per_monitor, vec![None, None]);
        let rules = Rules { focus: Rule::Pause, ..Rules::default() };
        assert_eq!(evaluate(&monitors(), &snap, &rules).per_monitor, vec![None, Some(Reason::Focus)]);
    }

    #[test]
    fn ignored_app_never_pauses() {
        let snap = Snapshot { windows: maximized_with_taskbar(), ..Default::default() };
        let rules = Rules { apps: vec![("code.exe".into(), AppAction::Ignore)], ..Rules::default() };
        // Sem o VS Code, só a barra de tarefas cobre: o monitor continua visível.
        assert_eq!(evaluate(&monitors(), &snap, &rules).per_monitor, vec![None, None]);
    }

    #[test]
    fn app_rules_pause_or_release_everything() {
        let snap = Snapshot { windows: vec![window(rect(10, 10, 50, 50), "obs64.exe")], ..Default::default() };
        let pause = Rules { apps: vec![("OBS64.EXE".into(), AppAction::Pause)], ..Rules::default() };
        let d = evaluate(&monitors(), &snap, &pause);
        assert!(d.per_monitor.iter().all(|r| r == &Some(Reason::App("obs64.exe".into()))));
        assert!(!d.release_now);

        let release = Rules { apps: vec![("obs64.exe".into(), AppAction::Release)], ..Rules::default() };
        assert!(evaluate(&monitors(), &snap, &release).release_now);
    }

    #[test]
    fn system_state_overrides_everything() {
        let snap = Snapshot { system: SystemState { locked: true, ..Default::default() }, ..Default::default() };
        assert_eq!(evaluate(&monitors(), &snap, &Rules::default()).per_monitor, vec![Some(Reason::Locked); 2]);

        let battery = Snapshot { system: SystemState { on_battery: true, ..Default::default() }, ..Default::default() };
        let keep = Rules { battery: Rule::Continue, ..Rules::default() };
        assert_eq!(evaluate(&monitors(), &battery, &keep).per_monitor, vec![None, None]);
    }

    #[test]
    fn uncovered_area_handles_overlaps() {
        let m = rect(0, 0, 100, 100);
        assert_eq!(uncovered_area(m, &[]), 10_000);
        assert_eq!(uncovered_area(m, &[rect(0, 0, 50, 100)]), 5_000);
        // Duas janelas sobrepostas não contam a sobreposição duas vezes.
        assert_eq!(uncovered_area(m, &[rect(0, 0, 60, 100), rect(40, 0, 100, 50)]), 2_000);
        assert_eq!(uncovered_area(m, &[rect(-10, -10, 200, 200)]), 0);
    }

    #[test]
    fn monitor_of_picks_largest_overlap() {
        // Janela atravessando a divisa, com mais área no monitor da esquerda.
        assert_eq!(monitor_of(&monitors(), &rect(-900, 0, 100, 500)), Some(1));
        assert_eq!(monitor_of(&monitors(), &rect(5000, 0, 5100, 100)), None);
    }
}
