//! Observa o Windows e tira a "foto" que o módulo `pause` avalia.
//!
//! Acorda por eventos (foco, minimizar, mostrar, esconder, área de trabalho virtual, bloqueio,
//! energia). O evento de mudança de posição fica de fora porque o Windows o dispara a cada
//! movimento do mouse; para pegar maximizar e restaurar, a janela em foco é conferida uma vez
//! por segundo, uma chamada barata.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Shell::SHQueryUserNotificationState;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, FindWindowW, GA_ROOT, GWL_EXSTYLE,
    GetAncestor, GetClassNameW, GetForegroundWindow, GetLayeredWindowAttributes,
    GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, IsZoomed, LWA_ALPHA, RegisterClassExW, SM_REMOTESESSION, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_OVERLAPPED,
};
use windows_core::{BOOL, PWSTR, Result, w};

use crate::pause::{Foreground, Snapshot, SystemState, WindowInfo};

static DIRTY: AtomicBool = AtomicBool::new(true);
static LOCKED: AtomicBool = AtomicBool::new(false);
static DISPLAY_CHANGED: AtomicBool = AtomicBool::new(false);
static RELOAD: AtomicBool = AtomicBool::new(false);
static QUIT: AtomicBool = AtomicBool::new(false);

/// A janela do Kage salvou a configuração e pediu para recarregar.
pub fn take_reload() -> bool {
    RELOAD.swap(false, Ordering::SeqCst)
}

/// A janela do Kage (ou o menu da bandeja) pediu para encerrar o motor.
pub fn quit_requested() -> bool {
    QUIT.load(Ordering::SeqCst)
}

/// O menu da bandeja mudou a configuração: recarregar como se a janela tivesse pedido.
pub fn request_reload() {
    RELOAD.store(true, Ordering::SeqCst);
}

/// "Sair" do menu da bandeja.
pub fn request_quit() {
    QUIT.store(true, Ordering::SeqCst);
}

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_SYSTEM_MOVESIZEEND: u32 = 0x000B;
const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
const EVENT_SYSTEM_MINIMIZEEND: u32 = 0x0017;
const EVENT_OBJECT_DESTROY: u32 = 0x8001;
const EVENT_OBJECT_SHOW: u32 = 0x8002;
const EVENT_OBJECT_HIDE: u32 = 0x8003;
const EVENT_OBJECT_CLOAKED: u32 = 0x8017;
const EVENT_OBJECT_UNCLOAKED: u32 = 0x8018;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;
const OBJID_WINDOW: i32 = 0;
const CHILDID_SELF: i32 = 0;

const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WM_POWERBROADCAST: u32 = 0x0218;
const WM_DISPLAYCHANGE: u32 = 0x007E;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;

/// Jogo em tela cheia exclusiva e modo apresentação, como o Windows informa.
const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;
const QUNS_PRESENTATION_MODE: i32 = 4;

/// Intervalo mínimo entre duas avaliações, para não reavaliar a cada evento de uma rajada.
const DEBOUNCE: Duration = Duration::from_millis(100);
/// Conferência leve da janela em foco (maximizar e restaurar não geram evento barato).
const FOREGROUND_PROBE: Duration = Duration::from_secs(1);

/// Janelas que são a própria área de trabalho ou a barra de tarefas: não contam como "app em foco".
const SHELL_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "Windows.UI.Core.CoreWindow",
    "MultitaskingViewFrame",
    "ForegroundStaging",
    "TaskListThumbnailWnd",
    "XamlExplorerHostIslandWindow",
];

/// Janela invisível de topo: recebe bloqueio de tela, energia, mudança de monitor e os avisos
/// da janela do Kage. Vive enquanto o motor estiver aberto.
pub struct SystemWindow {
    hwnd: HWND,
}

impl SystemWindow {
    pub fn create() -> Result<Self> {
        let hwnd = create_message_window()?;
        unsafe {
            let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
        }
        Ok(Self { hwnd })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for SystemWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = WTSUnRegisterSessionNotification(self.hwnd);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Janela de um motor que já está rodando, se houver.
pub fn running_engine() -> Option<HWND> {
    let class = windows_core::HSTRING::from(kage_core::ENGINE_WINDOW_CLASS);
    unsafe { FindWindowW(&class, None) }.ok().filter(|h| !h.is_invalid())
}

pub struct Watcher {
    hooks: Vec<HWINEVENTHOOK>,
    own_pid: u32,
    need_exe: bool,
    last_snapshot: Option<Instant>,
    last_probe: Instant,
    probed: (isize, RECT, bool),
}

impl Watcher {
    /// `need_exe`: descobrir o executável de cada janela (só quando há regras por app).
    pub fn new(need_exe: bool) -> Result<Self> {
        let ranges = [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZEEND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ];
        let hooks = ranges
            .iter()
            .map(|&(min, max)| unsafe {
                SetWinEventHook(min, max, None, Some(on_win_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)
            })
            .filter(|h| !h.is_invalid())
            .collect();

        Ok(Self {
            hooks,
            own_pid: unsafe { GetCurrentProcessId() },
            need_exe,
            last_snapshot: None,
            last_probe: Instant::now(),
            probed: (0, RECT::default(), false),
        })
    }

    /// Se já é hora de tirar uma foto nova.
    pub fn due(&mut self, now: Instant) -> bool {
        let rested = self.last_snapshot.is_none_or(|t| now.duration_since(t) >= DEBOUNCE);
        if DIRTY.load(Ordering::SeqCst) && rested {
            return true;
        }
        if now.duration_since(self.last_probe) >= FOREGROUND_PROBE {
            self.last_probe = now;
            let probe = probe_foreground();
            if probe != self.probed {
                self.probed = probe;
                return true;
            }
        }
        false
    }

    /// Quando o laço principal deve acordar de novo por causa do observador.
    pub fn next_wake(&self, now: Instant) -> Instant {
        let probe = self.last_probe + FOREGROUND_PROBE;
        if DIRTY.load(Ordering::SeqCst) {
            let debounce = self.last_snapshot.map_or(now, |t| t + DEBOUNCE);
            debounce.min(probe)
        } else {
            probe
        }
    }

    pub fn display_changed(&self) -> bool {
        DISPLAY_CHANGED.swap(false, Ordering::SeqCst)
    }

    pub fn snapshot(&mut self) -> Snapshot {
        DIRTY.store(false, Ordering::SeqCst);
        self.last_snapshot = Some(Instant::now());

        let mut ctx = EnumContext { own_pid: self.own_pid, need_exe: self.need_exe, windows: Vec::new() };
        unsafe {
            let _ = EnumWindows(Some(collect_window), LPARAM(&mut ctx as *mut EnumContext as isize));
        }

        Snapshot { windows: ctx.windows, foreground: self.foreground(), system: system_state() }
    }

    fn foreground(&self) -> Option<Foreground> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return None;
            }
            if SHELL_CLASSES.contains(&class_name(hwnd).as_str()) {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == self.own_pid {
                return None;
            }
            let bounds = frame_bounds(hwnd);
            let monitor = monitor_rect(hwnd);
            // Num monitor sem barra de tarefas, uma janela maximizada ocupa o monitor inteiro;
            // tela cheia de verdade (jogo, F11) não fica no estado "maximizada".
            let covers_monitor = !IsZoomed(hwnd).as_bool()
                && bounds.left <= monitor.left
                && bounds.top <= monitor.top
                && bounds.right >= monitor.right
                && bounds.bottom >= monitor.bottom;
            let exclusive = SHQueryUserNotificationState()
                .map(|s| s.0 == QUNS_RUNNING_D3D_FULL_SCREEN || s.0 == QUNS_PRESENTATION_MODE)
                .unwrap_or(false);
            let exe = if self.need_exe { process_name(pid) } else { String::new() };
            Some(Foreground { window: WindowInfo { bounds, exe }, fullscreen: covers_monitor || exclusive })
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            for hook in &self.hooks {
                let _ = UnhookWinEvent(*hook);
            }
        }
        // A próxima cena começa reavaliando tudo.
        DIRTY.store(true, Ordering::SeqCst);
    }
}

struct EnumContext {
    own_pid: u32,
    need_exe: bool,
    windows: Vec<WindowInfo>,
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(data.0 as *mut EnumContext) };
    if let Some(info) = unsafe { covering_window(hwnd, ctx) } {
        ctx.windows.push(info);
    }
    BOOL(1)
}

/// A janela, se ela pode esconder o wallpaper.
unsafe fn covering_window(hwnd: HWND, ctx: &EnumContext) -> Option<WindowInfo> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        // Janelas "escondidas" pelo DWM: outra área de trabalho virtual, apps UWP suspensos.
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), 4);
        if cloaked != 0 {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == ctx.own_pid {
            return None;
        }
        let class = class_name(hwnd);
        if class == "Progman" || class == "WorkerW" {
            return None;
        }
        // Camadas transparentes de clique (overlays) não escondem nada.
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TRANSPARENT.0 != 0 {
            return None;
        }
        if ex_style & WS_EX_LAYERED.0 != 0 {
            let (mut alpha, mut flags) = (255u8, Default::default());
            if GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)).is_ok()
                && flags & LWA_ALPHA == LWA_ALPHA
                && alpha == 0
            {
                return None;
            }
        }
        let bounds = frame_bounds(hwnd);
        if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
            return None;
        }
        let exe = if ctx.need_exe { process_name(pid) } else { String::new() };
        Some(WindowInfo { bounds, exe })
    }
}

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != OBJID_WINDOW || id_child != CHILDID_SELF || hwnd.is_invalid() {
        return;
    }
    // Mostrar vale só para janelas de topo; esconder e destruir chegam quando a janela pode
    // já não existir, então contam sempre.
    if event == EVENT_OBJECT_SHOW && unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return;
    }
    DIRTY.store(true, Ordering::SeqCst);
}

fn create_message_window() -> Result<HWND> {
    let class_name = windows_core::HSTRING::from(kage_core::ENGINE_WINDOW_CLASS);
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(system_events_proc),
            hInstance: instance.into(),
            lpszClassName: windows_core::PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&class);
        // Janela de topo invisível: só janelas de topo recebem os avisos de energia e de tela.
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            &class_name,
            w!("Kage"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

unsafe extern "system" fn system_events_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if crate::tray::handle(msg, lparam) {
        return LRESULT(0);
    }
    match msg {
        WM_WTSSESSION_CHANGE => {
            match wparam.0 {
                WTS_SESSION_LOCK => LOCKED.store(true, Ordering::SeqCst),
                WTS_SESSION_UNLOCK => LOCKED.store(false, Ordering::SeqCst),
                _ => {}
            }
            DIRTY.store(true, Ordering::SeqCst);
        }
        WM_POWERBROADCAST => {
            DIRTY.store(true, Ordering::SeqCst);
            return LRESULT(1);
        }
        WM_DISPLAYCHANGE => {
            DISPLAY_CHANGED.store(true, Ordering::SeqCst);
            DIRTY.store(true, Ordering::SeqCst);
        }
        kage_core::MSG_RELOAD => RELOAD.store(true, Ordering::SeqCst),
        kage_core::MSG_QUIT => QUIT.store(true, Ordering::SeqCst),
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn system_state() -> SystemState {
    let mut power = SYSTEM_POWER_STATUS::default();
    let has_power = unsafe { GetSystemPowerStatus(&mut power) }.is_ok();
    SystemState {
        locked: LOCKED.load(Ordering::SeqCst),
        remote_session: unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0,
        // ACLineStatus 0 = na bateria; 255 = desconhecido (desktop sem bateria).
        on_battery: has_power && power.ACLineStatus == 0,
        battery_saver: has_power && power.SystemStatusFlag == 1,
    }
}

/// (janela, limites, maximizada) da janela em foco, para notar maximizar e restaurar.
fn probe_foreground() -> (isize, RECT, bool) {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return (0, RECT::default(), false);
        }
        (hwnd.0 as isize, frame_bounds(hwnd), IsZoomed(hwnd).as_bool())
    }
}

/// Limites visíveis (sem as bordas invisíveis de redimensionamento do Windows 10).
fn frame_bounds(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        let ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        );
        if ok.is_err() {
            let _ = GetWindowRect(hwnd, &mut rect);
        }
    }
    rect
}

fn monitor_rect(hwnd: HWND) -> RECT {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        info.rcMonitor
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..len])
}

/// Nome do executável em minúsculas, sem caminho ("code.exe").
fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buffer = [0u16; 512];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len);
        let _ = windows::Win32::Foundation::CloseHandle(process);
        if ok.is_err() {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        path.rsplit('\\').next().unwrap_or_default().to_ascii_lowercase()
    }
}
