//! Skins do Rainmeter em janelas próprias, como no Rainmeter.
//!
//! Cada skin é uma janela em camadas (`UpdateLayeredWindow`): pixel transparente não pega
//! clique. Dá para arrastar (Ctrl arrasta sempre e solta das bordas), grudar a 10 px das bordas
//! e de outras skins, manter dentro do monitor, clicar através (Ctrl, Shift ou Alt suspendem),
//! escolher a camada, a transparência e o que fazer com o mouse em cima, e abrir o menu do
//! clique direito com as opções do Rainmeter. O que muda direto na skin vai para o
//! `config.json`, e a janela do Kage relê o arquivo.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, Instant};

use kage_core::{SkinAnchor, SkinEntry, SkinHover, SkinLayer, ThemeChoice};
use kage_skins::{Action, Frame, MouseEvent, Skin};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::{D2D1_DC_INITIALIZE_MODE_COPY, ID2D1Bitmap1, ID2D1GdiInteropRenderTarget};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_DBLCLKS, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, GW_HWNDPREV,
    GWL_EXSTYLE, GetClassNameW, GetCursorPos, GetWindow, GetWindowLongPtrW, HMENU, HWND_NOTOPMOST, HWND_TOP,
    HWND_TOPMOST, IDC_ARROW, LoadCursorW, MA_NOACTIVATE, MENU_ITEM_FLAGS, MF_CHECKED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, PostMessageW, RegisterClassExW, SW_SHOWNOACTIVATE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
    SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetMenuDefaultItem, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, ULW_ALPHA, UpdateLayeredWindow, WM_CAPTURECHANGED,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NULL, WM_RBUTTONUP, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows_core::{HSTRING, Interface, PCSTR, PCWSTR, Result, w};

use crate::gpu::Gpu;
use crate::monitors::Monitor;
use crate::now_playing::MediaCommand;
use crate::skins::{self, ImageCache, Measurer, Placement, TextEngine};
use crate::sysinfo::WinSystem;
use crate::{desktop, log};

const CLASS: PCWSTR = w!("KageSkin");
/// Distância em que a skin gruda numa borda (o SNAPDISTANCE do Rainmeter).
const SNAP: i32 = 10;
const FADE: Duration = Duration::from_millis(250);
/// Conferência do mouse e das teclas, só quando alguma skin usa "passar o mouse" ou "clicar através".
const POLL: Duration = Duration::from_millis(100);
/// "Sempre visível" é reafirmado de tempos em tempos (jogos e vídeos em tela cheia passam por cima).
const TOPMOST_AGAIN: Duration = Duration::from_secs(5);
const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;

// ---------- o que a janela da skin precisa saber sozinha (lido no window proc) ----------

#[derive(Clone, Copy)]
struct Behaviour {
    draggable: bool,
    snap: bool,
    keep_on_screen: bool,
    /// Camada "Normal": vem para a frente ao ser clicada.
    raise_on_click: bool,
}

impl Behaviour {
    fn of(entry: &SkinEntry) -> Self {
        Self {
            draggable: entry.draggable,
            snap: entry.snap_edges,
            keep_on_screen: entry.keep_on_screen,
            raise_on_click: entry.layer == SkinLayer::Normal,
        }
    }
}

#[derive(Clone, Copy)]
struct Drag {
    cursor: POINT,
    origin: POINT,
    moved: bool,
}

struct Tracked {
    behaviour: Behaviour,
    rect: RECT,
    drag: Option<Drag>,
}

enum Event {
    Moved(HWND),
    /// Clique, duplo clique ou rodinha; o ponto é dentro da skin.
    Mouse(HWND, MouseEvent, POINT),
    /// Botão direito: ação da skin, se houver, ou o menu (com Ctrl, sempre o menu).
    Right(HWND, POINT, POINT),
}

fn client_point(lparam: LPARAM) -> POINT {
    POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 }
}

fn push_mouse(hwnd: HWND, event: MouseEvent, at: POINT) {
    EVENTS.with_borrow_mut(|e| e.push(Event::Mouse(hwnd, event, at)));
}

thread_local! {
    static TRACKED: RefCell<HashMap<isize, Tracked>> = RefCell::default();
    static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
    /// A área de trabalho está na frente (clique nela ou Win+D).
    static DESKTOP_IN_FRONT: Cell<bool> = const { Cell::new(false) };
    static FOREGROUND_CHANGED: Cell<bool> = const { Cell::new(true) };
}

fn key(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

fn modifiers_held() -> bool {
    held(VK_CONTROL) || held(VK_SHIFT) || held(VK_MENU)
}

fn cursor() -> POINT {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    point
}

fn tracked_rect(hwnd: HWND) -> Option<RECT> {
    TRACKED.with_borrow(|t| t.get(&key(hwnd)).map(|tr| tr.rect))
}

fn contains(rect: &RECT, p: POINT) -> bool {
    p.x >= rect.left && p.x < rect.right && p.y >= rect.top && p.y < rect.bottom
}

fn monitor_rect_at(p: POINT) -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        info.rcMonitor
    }
}

/// A borda mais perto, a até `SNAP` pixels, encostando o começo ou o fim da skin nela.
fn snap_axis(value: i32, size: i32, edges: &[i32]) -> i32 {
    let (mut best, mut distance) = (value, SNAP + 1);
    for &edge in edges {
        for candidate in [edge, edge - size] {
            let d = (value - candidate).abs();
            if d < distance {
                distance = d;
                best = candidate;
            }
        }
    }
    best
}

/// Onde a skin para quando arrastada até `wanted`: grudada nas bordas próximas e dentro do monitor.
fn settle(hwnd: HWND, wanted: POINT, size: (i32, i32), behaviour: Behaviour, loose: bool) -> POINT {
    let screen = monitor_rect_at(POINT { x: wanted.x + size.0 / 2, y: wanted.y + size.1 / 2 });
    let mut p = wanted;
    if behaviour.snap && !loose {
        let (mut xs, mut ys) = (vec![screen.left, screen.right], vec![screen.top, screen.bottom]);
        TRACKED.with_borrow(|t| {
            for (k, other) in t {
                if *k != key(hwnd) {
                    xs.extend([other.rect.left, other.rect.right]);
                    ys.extend([other.rect.top, other.rect.bottom]);
                }
            }
        });
        p.x = snap_axis(p.x, size.0, &xs);
        p.y = snap_axis(p.y, size.1, &ys);
    }
    if behaviour.keep_on_screen {
        p.x = p.x.clamp(screen.left, (screen.right - size.0).max(screen.left));
        p.y = p.y.clamp(screen.top, (screen.bottom - size.1).max(screen.top));
    }
    p
}

fn begin_drag(hwnd: HWND) {
    let ctrl = held(VK_CONTROL);
    let started = TRACKED.with_borrow_mut(|t| {
        let tr = t.get_mut(&key(hwnd))?;
        let dragging = tr.behaviour.draggable || ctrl;
        if dragging {
            tr.drag = Some(Drag { cursor: cursor(), origin: POINT { x: tr.rect.left, y: tr.rect.top }, moved: false });
        }
        Some((dragging, tr.behaviour.raise_on_click))
    });
    let Some((dragging, raise)) = started else { return };
    unsafe {
        if raise {
            let _ = SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        if dragging {
            SetCapture(hwnd);
        }
    }
}

fn continue_drag(hwnd: HWND) {
    let step = TRACKED.with_borrow_mut(|t| {
        let tr = t.get_mut(&key(hwnd))?;
        let drag = tr.drag.as_mut()?;
        let now = cursor();
        let (dx, dy) = (now.x - drag.cursor.x, now.y - drag.cursor.y);
        if !drag.moved && dx.abs() < 2 && dy.abs() < 2 {
            return None;
        }
        drag.moved = true;
        let size = (tr.rect.right - tr.rect.left, tr.rect.bottom - tr.rect.top);
        Some((POINT { x: drag.origin.x + dx, y: drag.origin.y + dy }, size, tr.behaviour))
    });
    let Some((wanted, size, behaviour)) = step else { return };
    // Ctrl ou Shift soltam a skin das bordas enquanto arrasta.
    let p = settle(hwnd, wanted, size, behaviour, held(VK_CONTROL) || held(VK_SHIFT));
    unsafe {
        let _ = SetWindowPos(hwnd, None, p.x, p.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    TRACKED.with_borrow_mut(|t| {
        if let Some(tr) = t.get_mut(&key(hwnd)) {
            tr.rect = RECT { left: p.x, top: p.y, right: p.x + size.0, bottom: p.y + size.1 };
        }
    });
}

/// Termina o arrasto. Devolve se a skin andou (aí o soltar do botão não é um clique).
fn finish_drag(hwnd: HWND) -> bool {
    let moved = TRACKED.with_borrow_mut(|t| t.get_mut(&key(hwnd)).and_then(|tr| tr.drag.take()).map(|d| d.moved));
    let Some(moved) = moved else { return false };
    unsafe {
        let _ = ReleaseCapture();
    }
    if moved {
        EVENTS.with_borrow_mut(|e| e.push(Event::Moved(hwnd)));
    }
    moved
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // A skin nunca rouba o foco de quem está digitando.
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            begin_drag(hwnd);
            push_mouse(hwnd, MouseEvent::LeftDown, client_point(lparam));
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            continue_drag(hwnd);
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            // Soltar depois de arrastar não é clique: os botões da skin só agem parados.
            if !finish_drag(hwnd) {
                push_mouse(hwnd, MouseEvent::LeftUp, client_point(lparam));
            }
            return LRESULT(0);
        }
        WM_LBUTTONDBLCLK => {
            push_mouse(hwnd, MouseEvent::LeftDoubleClick, client_point(lparam));
            return LRESULT(0);
        }
        WM_MBUTTONUP => {
            push_mouse(hwnd, MouseEvent::MiddleUp, client_point(lparam));
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            // A rodinha manda o ponto na tela; a skin quer o ponto dentro dela.
            let mut p = client_point(lparam);
            unsafe {
                let _ = ScreenToClient(hwnd, &mut p);
            }
            let delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
            push_mouse(hwnd, if delta > 0 { MouseEvent::ScrollUp } else { MouseEvent::ScrollDown }, p);
            return LRESULT(0);
        }
        WM_CAPTURECHANGED => {
            finish_drag(hwnd);
        }
        WM_RBUTTONUP => {
            let inside = client_point(lparam);
            let mut screen = inside;
            unsafe {
                let _ = ClientToScreen(hwnd, &mut screen);
            }
            EVENTS.with_borrow_mut(|e| e.push(Event::Right(hwnd, inside, screen)));
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    let mut buffer = [0u16; 32];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
    let class = String::from_utf16_lossy(&buffer[..len]);
    let desktop = class == "WorkerW" || class == "Progman";
    if desktop != DESKTOP_IN_FRONT.get() {
        DESKTOP_IN_FRONT.set(desktop);
        FOREGROUND_CHANGED.set(true);
    }
}

fn register_class() -> Result<()> {
    static REGISTER: Once = Once::new();
    let mut result = Ok(());
    REGISTER.call_once(|| unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            // Duplo clique vira mensagem própria (o LeftMouseDoubleClickAction das skins).
            style: CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            hInstance: GetModuleHandleW(None).map(Into::into).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            result = Err(windows_core::Error::from_thread());
        }
    });
    result
}

/// Pasta das skins: a do Rainmeter.ini quando a skin está nela; senão, três níveis acima do .ini
/// (Skins\Pacote\Config\arquivo.ini).
pub fn skins_root_for(ini: &Path) -> PathBuf {
    kage_skins::rainmeter::default_ini()
        .and_then(|p| kage_skins::rainmeter::read(&p).ok())
        .map(|setup| setup.skins_path)
        .filter(|root| ini.starts_with(root))
        .or_else(|| ini.parent()?.parent()?.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// O primeiro `.ini` da pasta de uma skin (o `!ToggleConfig` pode vir sem o nome do arquivo).
fn first_ini(folder: &Path) -> Option<PathBuf> {
    let mut inis: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ini")))
        .collect();
    inis.sort();
    inis.into_iter().next()
}

/// Ação que não é bang: um endereço, arquivo ou programa, aberto como no Executar do Windows.
fn open_target(target: &str) {
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    let file = HSTRING::from(target);
    unsafe {
        let _ = ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL);
    }
}

/// As skins ativas no Rainmeter.ini, com as opções de janela de lá (modo de teste `--skins`).
pub fn rainmeter_entries() -> Vec<SkinEntry> {
    let Some(setup) = kage_skins::rainmeter::default_ini().and_then(|p| kage_skins::rainmeter::read(&p).ok()) else {
        return Vec::new();
    };
    setup
        .active
        .into_iter()
        .map(|a| SkinEntry {
            id: a.config.to_lowercase(),
            config: a.config,
            ini: a.ini,
            x: a.x,
            y: a.y,
            active: true,
            alpha: a.options.alpha,
            layer: SkinLayer::from_rainmeter(a.options.always_on_top),
            on_hover: SkinHover::from_rainmeter(a.options.on_hover),
            click_through: a.options.click_through,
            draggable: a.options.draggable,
            keep_on_screen: a.options.keep_on_screen,
            save_position: a.options.save_position,
            snap_edges: a.options.snap_edges,
            ..Default::default()
        })
        .collect()
}

fn monitor_of<'a>(entry: &SkinEntry, monitors: &'a [Monitor]) -> Option<&'a Monitor> {
    entry
        .monitor
        .as_deref()
        .and_then(|d| monitors.iter().find(|m| m.device_name.eq_ignore_ascii_case(d)))
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or(monitors.first())
}

fn placement(entry: &SkinEntry, monitors: &[Monitor]) -> Placement {
    match (entry.anchor, monitor_of(entry, monitors)) {
        (SkinAnchor::Centered, Some(m)) => {
            Placement::Centered { area: m.rect, height: entry.height_percent.clamp(0.0, 100.0) / 100.0 }
        }
        _ => Placement::Free { x: entry.x, y: entry.y },
    }
}

/// Opacidade que a skin deve ter agora, com ou sem o mouse em cima.
fn target_alpha(entry: &SkinEntry, hovered: bool) -> f32 {
    let base = entry.alpha.max(1) as f32;
    match (entry.on_hover, hovered) {
        (SkinHover::Hide | SkinHover::FadeOut, true) => 1.0,
        (SkinHover::FadeIn, true) => 255.0,
        _ => base,
    }
}

/// Transparência do menu: o item k (0 a 9) vale 255 - k * 230 / 9; "~100%" vale 1 para a skin
/// continuar recebendo o mouse (a conta do Rainmeter).
fn alpha_for_item(item: u32) -> u8 {
    if item >= 10 { 1 } else { (255 - item * 230 / 9) as u8 }
}

fn item_for_alpha(alpha: u8) -> u32 {
    if alpha <= 1 { 10 } else { ((255.0 - alpha as f32) * 9.0 / 230.0).round().clamp(0.0, 9.0) as u32 }
}

fn set_alpha(hwnd: HWND, alpha: f32) {
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha.round().clamp(0.0, 255.0) as u8,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        let _ = UpdateLayeredWindow(hwnd, None, None, None, None, None, COLORREF(0), Some(&blend), ULW_ALPHA);
    }
}

/// Menu do clique direito escuro ou claro, como o tema escolhido no Kage ("Sistema" segue o modo
/// de apps do Windows). O Windows 10 só oferece isso por funções da uxtheme sem nome, pelo número
/// (133, 135 e 136, do 1903 em diante); se não existirem, o menu fica no padrão.
pub(crate) fn theme_menus(owner: HWND) {
    type AllowDarkModeForWindow = unsafe extern "system" fn(HWND, i32) -> i32;
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    type FlushMenuThemes = unsafe extern "system" fn();
    type Farproc = unsafe extern "system" fn() -> isize;
    // 1 = segue o Windows, 2 = sempre escuro, 3 = sempre claro.
    let mode = match kage_core::Config::load_from(&kage_core::config_path()).map(|c| c.general.theme) {
        Ok(ThemeChoice::Dark) => 2,
        Ok(ThemeChoice::Light) => 3,
        _ => 1,
    };
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        let ordinal = |n: usize| GetProcAddress(uxtheme, PCSTR(n as *const u8));
        if let Some(f) = ordinal(133) {
            std::mem::transmute::<Farproc, AllowDarkModeForWindow>(f)(owner, 1);
        }
        if let Some(f) = ordinal(135) {
            std::mem::transmute::<Farproc, SetPreferredAppMode>(f)(mode);
        }
        if let Some(f) = ordinal(136) {
            std::mem::transmute::<Farproc, FlushMenuThemes>(f)();
        }
    }
}

fn set_click_through(hwnd: HWND, on: bool) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let bit = WS_EX_TRANSPARENT.0 as isize;
        let new = if on { style | bit } else { style & !bit };
        if new != style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new);
        }
    }
}

fn apply_layer(hwnd: HWND, layer: SkinLayer) {
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
    unsafe {
        let after = match layer {
            SkinLayer::Top | SkinLayer::Topmost => HWND_TOPMOST,
            SkinLayer::Normal => HWND_NOTOPMOST,
            // Com a área de trabalho na frente (Win+D), a skin "No desktop" continua à vista.
            SkinLayer::Desktop if DESKTOP_IN_FRONT.get() => HWND_TOPMOST,
            SkinLayer::Desktop | SkinLayer::Bottom => {
                let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
                // Logo acima da janela dos ícones.
                let Some(host) = desktop::icon_host() else { return };
                match GetWindow(host, GW_HWNDPREV) {
                    Ok(prev) if prev != hwnd => prev,
                    _ => return,
                }
            }
        };
        let _ = SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, flags);
    }
}

/// Copia a imagem desenhada para a janela da skin (só funciona entre o BeginDraw e o EndDraw).
fn present(gpu: &Gpu, hwnd: HWND, pos: POINT, size: (u32, u32), alpha: f32) -> Result<()> {
    unsafe {
        let interop: ID2D1GdiInteropRenderTarget = gpu.d2d.cast()?;
        let hdc = interop.GetDC(D2D1_DC_INITIALIZE_MODE_COPY)?;
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha.round().clamp(0.0, 255.0) as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let source = POINT::default();
        let extent = SIZE { cx: size.0 as i32, cy: size.1 as i32 };
        let result = UpdateLayeredWindow(
            hwnd,
            None,
            Some(&pos),
            Some(&extent),
            Some(hdc),
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        interop.ReleaseDC(None)?;
        result
    }
}

// ---------- cada skin numa tela ----------

struct Instance {
    entry: SkinEntry,
    skin: Skin,
    hwnd: HWND,
    frame: Option<Frame>,
    bitmap: Option<(ID2D1Bitmap1, (u32, u32))>,
    next_update: Instant,
    shown: bool,
    alpha: f32,
    /// De, para, desde quando.
    fade: Option<(f32, f32, Instant)>,
    hovered: bool,
    transparent: bool,
    last_topmost: Instant,
}

impl Instance {
    fn create(entry: SkinEntry) -> std::result::Result<Self, String> {
        let skin = Skin::load(&entry.ini, &skins_root_for(&entry.ini))?;
        register_class().map_err(|e| e.to_string())?;
        let mut ex_style = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        if entry.click_through {
            ex_style |= WS_EX_TRANSPARENT;
        }
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            CreateWindowExW(
                ex_style,
                CLASS,
                &HSTRING::from(entry.config.as_str()),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| e.to_string())?
        };
        TRACKED.with_borrow_mut(|t| {
            t.insert(key(hwnd), Tracked { behaviour: Behaviour::of(&entry), rect: RECT::default(), drag: None });
        });
        for warning in &skin.warnings {
            log(&format!("  aviso: {warning}"));
        }
        Ok(Self {
            alpha: target_alpha(&entry, false),
            transparent: entry.click_through,
            entry,
            skin,
            hwnd,
            frame: None,
            bitmap: None,
            next_update: Instant::now(),
            shown: false,
            fade: None,
            hovered: false,
            last_topmost: Instant::now(),
        })
    }

    fn rect(&self) -> RECT {
        tracked_rect(self.hwnd).unwrap_or_default()
    }

    /// Faz a skin redesenhar já, com a posição e as opções novas.
    fn invalidate(&mut self) {
        self.frame = None;
        self.next_update = Instant::now();
    }

    /// Aplica opções novas sem recriar a janela (a skin só recarrega se mudou o `.ini`).
    fn update(&mut self, entry: SkinEntry) {
        if entry.ini != self.entry.ini {
            match Skin::load(&entry.ini, &skins_root_for(&entry.ini)) {
                Ok(skin) => self.skin = skin,
                Err(e) => log(&format!("Skin {} não carregou: {e}", entry.config)),
            }
        }
        self.entry = entry;
        TRACKED.with_borrow_mut(|t| {
            if let Some(tr) = t.get_mut(&key(self.hwnd)) {
                tr.behaviour = Behaviour::of(&self.entry);
            }
        });
        self.transparent = self.entry.click_through;
        set_click_through(self.hwnd, self.transparent);
        let alpha = target_alpha(&self.entry, self.hovered);
        if alpha != self.alpha {
            log(&format!("Skin {}: transparência {}% (opacidade {} de 255)", self.entry.config, item_for_alpha(self.entry.alpha) * 10, alpha));
        }
        self.alpha = alpha;
        self.fade = None;
        if self.shown {
            apply_layer(self.hwnd, self.entry.layer);
            // A opacidade vale já, sem esperar o próximo desenho: com o monitor coberto (a janela do
            // Kage na frente, por exemplo) a skin não redesenha, e a mudança ficava para depois.
            set_alpha(self.hwnd, self.alpha);
        }
        self.invalidate();
    }

    /// Leva a janela para a posição da configuração já. O desenho também posiciona, mas com o
    /// monitor coberto a skin não redesenha, e a posição nova (Centralizada, X e Y) ficava para depois.
    fn place_now(&mut self, monitors: &[Monitor]) {
        let Some(size) = self.bitmap.as_ref().map(|(_, s)| *s) else { return };
        let (x, y) = skins::place_skin(placement(&self.entry, monitors), (size.0 as f32, size.1 as f32));
        let pos = POINT { x: x as i32, y: y as i32 };
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, pos.x, pos.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        let hwnd = self.hwnd;
        TRACKED.with_borrow_mut(|t| {
            if let Some(tr) = t.get_mut(&key(hwnd)) {
                tr.rect = RECT { left: pos.x, top: pos.y, right: pos.x + size.0 as i32, bottom: pos.y + size.1 as i32 };
            }
        });
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        TRACKED.with_borrow_mut(|t| t.remove(&key(self.hwnd)));
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// ---------- o conjunto ----------

pub struct SkinHost {
    text: TextEngine,
    images: ImageCache,
    system: WinSystem,
    instances: Vec<Instance>,
    monitors: Vec<Monitor>,
    favorites: Vec<String>,
    /// Gravar no config.json o que muda direto na skin (desligado no modo de teste).
    persist: bool,
    hook: HWINEVENTHOOK,
    last_poll: Instant,
    /// Avisos de ação não suportada já escritos no log (a rodinha repetiria o mesmo a cada passo).
    logged: std::collections::HashSet<String>,
}

const CMD_KAGE: u32 = 2;
const CMD_EDIT: u32 = 3;
const CMD_RELOAD: u32 = 4;
const CMD_OFF: u32 = 5;
const CMD_CLICK_THROUGH: u32 = 10;
const CMD_DRAGGABLE: u32 = 11;
const CMD_KEEP_ON_SCREEN: u32 = 12;
const CMD_SAVE_POSITION: u32 = 13;
const CMD_SNAP: u32 = 14;
const CMD_FAVORITE: u32 = 15;
const CMD_MONITOR: u32 = 200;
const CMD_LAYER: u32 = 300;
const CMD_ALPHA: u32 = 400;
const CMD_HOVER: u32 = 500;

const LAYERS: [(SkinLayer, &str); 5] = [
    (SkinLayer::Topmost, "Sempre visível"),
    (SkinLayer::Top, "Acima"),
    (SkinLayer::Normal, "Normal"),
    (SkinLayer::Bottom, "Abaixo"),
    (SkinLayer::Desktop, "No desktop"),
];
const HOVERS: [(SkinHover, &str); 4] = [
    (SkinHover::None, "Não fazer nada"),
    (SkinHover::Hide, "Esconder"),
    (SkinHover::FadeIn, "Aparecer"),
    (SkinHover::FadeOut, "Desaparecer"),
];

fn checked(on: bool) -> MENU_ITEM_FLAGS {
    if on { MF_STRING | MF_CHECKED } else { MF_STRING }
}

pub(crate) fn append(menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str) {
    unsafe {
        let _ = AppendMenuW(menu, flags, id, &HSTRING::from(text));
    }
}

pub(crate) fn separator(menu: HMENU) {
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    }
}

impl SkinHost {
    pub fn new(persist: bool) -> Result<Self> {
        register_class()?;
        // Fora de contexto e sem os eventos do próprio motor.
        let hook = unsafe {
            SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, 0x0002)
        };
        let system = WinSystem::default();
        system.prime();
        Ok(Self {
            text: TextEngine::new()?,
            images: ImageCache::default(),
            system,
            instances: Vec::new(),
            monitors: Vec::new(),
            favorites: Vec::new(),
            persist,
            hook,
            last_poll: Instant::now(),
            logged: Default::default(),
        })
    }

    fn log_once(&mut self, message: String) {
        if self.logged.insert(message.clone()) {
            log(&message);
        }
    }

    fn index_of(&self, hwnd: HWND) -> Option<usize> {
        self.instances.iter().position(|i| i.hwnd == hwnd)
    }

    /// Executa a ação de mouse de uma skin, passo a passo. O que é só da skin (variáveis, opções)
    /// ela mesma faz; a música, ligar outra skin, recarregar e abrir programas ficam aqui.
    fn run_actions(&mut self, index: usize, text: &str) {
        let hwnd = self.instances[index].hwnd;
        for action in kage_skins::parse_actions(text) {
            // Uma ação anterior pode ter desligado a própria skin.
            let Some(index) = self.index_of(hwnd) else { return };
            match action {
                Action::Run(target) => open_target(&target),
                Action::Bang { name, args } => {
                    if self.instances[index].skin.apply_bang(&name, &args) {
                        continue;
                    }
                    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
                    match name.as_str() {
                        "commandmeasure" => self.measure_command(index, &arg(0), &arg(1)),
                        // !PluginBang "Medida Comando": o nome da medida vem junto.
                        "pluginbang" => {
                            let full = arg(0);
                            let (measure, command) = full.split_once(' ').unwrap_or((full.as_str(), ""));
                            self.measure_command(index, measure, command.trim());
                        }
                        "toggleconfig" | "activateconfig" | "deactivateconfig" => {
                            self.config_bang(index, &name, &arg(0), &arg(1));
                        }
                        "refresh" => {
                            let i = &mut self.instances[index];
                            match Skin::load(&i.entry.ini, &skins_root_for(&i.entry.ini)) {
                                Ok(skin) => i.skin = skin,
                                Err(e) => log(&format!("Skin {} não recarregou: {e}", i.entry.config)),
                            }
                        }
                        // Os arquivos da skin são do usuário: o Kage não reescreve.
                        "writekeyvalue" => {}
                        other => {
                            let config = self.instances[index].entry.config.clone();
                            self.log_once(format!("Skin {config}: a ação !{other} ainda não é suportada"));
                        }
                    }
                }
            }
        }
        if let Some(index) = self.index_of(hwnd) {
            self.instances[index].invalidate();
        }
    }

    /// `!CommandMeasure` numa medida do "tocando agora": os botões do player.
    fn measure_command(&mut self, index: usize, measure: &str, command: &str) {
        if !self.instances[index].skin.is_now_playing(measure) {
            let config = self.instances[index].entry.config.clone();
            self.log_once(format!("Skin {config}: comando na medida {measure} ainda não é suportado"));
            return;
        }
        let lower = command.trim().to_ascii_lowercase();
        let (verb, value) = lower.split_once(' ').unwrap_or((lower.as_str(), ""));
        let command = match verb {
            "playpause" => MediaCommand::PlayPause,
            "play" => MediaCommand::Play,
            "pause" => MediaCommand::Pause,
            "stop" => MediaCommand::Stop,
            "next" => MediaCommand::Next,
            "previous" => MediaCommand::Previous,
            // SetPosition N: N por cento da música; +N e -N andam a partir de onde está.
            "setposition" => {
                let Some(now) = self.system.media().snapshot().filter(|n| n.duration > 0.0) else { return };
                let value = value.trim();
                let amount: f64 = value.trim_start_matches(['+', '-']).parse().unwrap_or(0.0);
                let here = now.position / now.duration * 100.0;
                let percent = match value.chars().next() {
                    Some('+') => here + amount,
                    Some('-') => here - amount,
                    _ => amount,
                };
                MediaCommand::SetPosition(now.duration * percent.clamp(0.0, 100.0) / 100.0)
            }
            _ => {
                let config = self.instances[index].entry.config.clone();
                self.log_once(format!("Skin {config}: comando \"{command}\" do player ainda não é suportado"));
                return;
            }
        };
        self.system.media().command(command);
    }

    /// `!ToggleConfig`, `!ActivateConfig` e `!DeactivateConfig`: liga ou desliga outra skin. Ligada
    /// por aqui, ela aparece logo abaixo de quem pediu, na mesma tela e na mesma camada.
    fn config_bang(&mut self, index: usize, bang: &str, config: &str, file: &str) {
        if config.is_empty() {
            return;
        }
        let active = self.instances.iter().any(|i| i.entry.config.eq_ignore_ascii_case(config));
        let turn_on = match bang {
            "activateconfig" => true,
            "deactivateconfig" => false,
            _ => !active,
        };
        if turn_on == active {
            return;
        }
        if turn_on && self.instances.len() >= kage_core::MAX_ACTIVE_SKINS {
            log(&format!("Skin {config} não ligou: o limite é de {} skins ligadas", kage_core::MAX_ACTIVE_SKINS));
            return;
        }
        let source = &self.instances[index];
        let rect = source.rect();
        let folder = skins_root_for(&source.entry.ini).join(config);
        let ini = if file.is_empty() { first_ini(&folder).unwrap_or_else(|| folder.join("skin.ini")) } else { folder.join(file) };
        let monitor = source.entry.monitor.clone().or_else(|| monitor_of(&source.entry, &self.monitors).map(|m| m.device_name.clone()));
        let (layer, source_config) = (source.entry.layer, source.entry.config.clone());
        let (x, y) = (rect.left as f32, (rect.bottom + 8) as f32);
        let result = kage_core::Config::edit_file(&kage_core::config_path(), |c| {
            if !turn_on {
                for e in c.skins.items.iter_mut().filter(|e| e.config.eq_ignore_ascii_case(config)) {
                    e.active = false;
                }
                return;
            }
            let fresh = SkinEntry {
                id: kage_core::new_skin_id(),
                config: config.to_string(),
                ini: ini.clone(),
                layer,
                ..Default::default()
            };
            let entry = match c.skins.items.iter().position(|e| e.config.eq_ignore_ascii_case(config)) {
                Some(at) => &mut c.skins.items[at],
                None => {
                    c.skins.items.push(fresh);
                    c.skins.items.last_mut().expect("acabou de entrar")
                }
            };
            (entry.active, entry.anchor, entry.monitor, entry.x, entry.y) = (true, SkinAnchor::Free, monitor.clone(), x, y);
        });
        if let Err(e) = result {
            log(&format!("Skin {config} não {}: {e}", if turn_on { "ligou" } else { "desligou" }));
            return;
        }
        log(&format!("Skin {config} {} pelo clique em {source_config}", if turn_on { "ligada" } else { "desligada" }));
        let saved = kage_core::Config::load();
        let entries = if saved.skins.enabled { saved.skins.items.clone() } else { Vec::new() };
        let monitors = self.monitors.clone();
        self.apply(&entries, &saved.skins.favorites, &monitors);
    }

    /// Põe na tela exatamente as skins ligadas (no máximo `MAX_ACTIVE_SKINS`). As que já estão
    /// abertas só recebem as opções novas; as desligadas fecham.
    pub fn apply(&mut self, entries: &[SkinEntry], favorites: &[String], monitors: &[Monitor]) {
        self.monitors = monitors.to_vec();
        self.favorites = favorites.to_vec();
        let wanted: Vec<&SkinEntry> = entries.iter().filter(|e| e.active).take(kage_core::MAX_ACTIVE_SKINS).collect();
        let skipped = entries.iter().filter(|e| e.active).count().saturating_sub(wanted.len());
        if skipped > 0 {
            log(&format!("Limite de {} skins ligadas: {skipped} ficaram de fora", kage_core::MAX_ACTIVE_SKINS));
        }
        self.instances.retain(|i| wanted.iter().any(|w| w.id == i.entry.id));
        for entry in wanted {
            match self.instances.iter_mut().find(|i| i.entry.id == entry.id) {
                Some(instance) => {
                    if instance.entry != *entry {
                        instance.update(entry.clone());
                        instance.place_now(&self.monitors);
                    }
                }
                None => match Instance::create(entry.clone()) {
                    Ok(instance) => {
                        log(&format!("Skin {} numa janela própria ({:?})", entry.config, entry.layer));
                        self.instances.push(instance);
                    }
                    Err(e) => log(&format!("Skin {} não carregou: {e}", entry.config)),
                },
            }
        }
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    fn needs_poll(&self) -> bool {
        self.instances.iter().any(|i| i.entry.on_hover != SkinHover::None || i.entry.click_through)
    }

    /// Próximo momento em que alguma skin precisa de atenção.
    pub fn next_wake(&self, now: Instant, hidden: &[RECT]) -> Instant {
        let mut wake = now + Duration::from_secs(3600);
        for i in &self.instances {
            if !is_hidden(&i.rect(), hidden) || !i.shown {
                wake = wake.min(i.next_update);
            }
            if i.fade.is_some() {
                wake = wake.min(now + Duration::from_millis(16));
            }
            if i.entry.layer == SkinLayer::Topmost {
                wake = wake.min(i.last_topmost + TOPMOST_AGAIN);
            }
        }
        if self.needs_poll() {
            wake = wake.min(self.last_poll + POLL);
        }
        wake
    }

    /// Eventos das janelas, camadas, mouse em cima, esmaecimento e atualização das skins.
    /// `hidden`: monitores cobertos por tela cheia, onde não adianta redesenhar.
    pub fn tick(&mut self, gpu: &Gpu, now: Instant, hidden: &[RECT]) -> Result<()> {
        for event in EVENTS.with_borrow_mut(std::mem::take) {
            match event {
                Event::Moved(hwnd) => self.moved(hwnd),
                Event::Mouse(hwnd, event, at) => {
                    let Some(index) = self.index_of(hwnd) else { continue };
                    if let Some(action) = self.instances[index].skin.mouse_action(event, at.x as f32, at.y as f32) {
                        self.run_actions(index, &action);
                    }
                }
                // Botão direito: a ação da skin, se ela tiver; com Ctrl, ou sem ação, o menu.
                Event::Right(hwnd, inside, screen) => {
                    let Some(index) = self.index_of(hwnd) else { continue };
                    let action = (!held(VK_CONTROL))
                        .then(|| self.instances[index].skin.mouse_action(MouseEvent::RightUp, inside.x as f32, inside.y as f32))
                        .flatten();
                    match action {
                        Some(action) => self.run_actions(index, &action),
                        None => self.menu(index, screen),
                    }
                }
            }
        }

        if FOREGROUND_CHANGED.replace(false) {
            for i in self.instances.iter().filter(|i| i.shown) {
                apply_layer(i.hwnd, i.entry.layer);
            }
        }
        for i in self.instances.iter_mut().filter(|i| i.shown && i.entry.layer == SkinLayer::Topmost) {
            if now >= i.last_topmost + TOPMOST_AGAIN {
                apply_layer(i.hwnd, i.entry.layer);
                i.last_topmost = now;
            }
        }

        if self.needs_poll() && now >= self.last_poll + POLL {
            self.last_poll = now;
            let modifiers = modifiers_held();
            let at = cursor();
            for i in &mut self.instances {
                if i.entry.click_through {
                    // Segurando Ctrl, Shift ou Alt a skin volta a receber o mouse.
                    let want = !modifiers;
                    if want != i.transparent {
                        i.transparent = want;
                        set_click_through(i.hwnd, want);
                    }
                }
                if i.entry.on_hover != SkinHover::None {
                    let hiding = matches!(i.entry.on_hover, SkinHover::Hide | SkinHover::FadeOut);
                    let hovered = contains(&i.rect(), at) && !(hiding && modifiers);
                    if hovered != i.hovered {
                        i.hovered = hovered;
                        let to = target_alpha(&i.entry, hovered);
                        if i.entry.on_hover == SkinHover::Hide {
                            i.alpha = to;
                            i.fade = None;
                            set_alpha(i.hwnd, to);
                        } else {
                            i.fade = Some((i.alpha, to, now));
                        }
                    }
                }
            }
        }

        for i in &mut self.instances {
            if let Some((from, to, since)) = i.fade {
                let t = (now.duration_since(since).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
                i.alpha = from + (to - from) * t;
                set_alpha(i.hwnd, i.alpha);
                if t >= 1.0 {
                    i.fade = None;
                }
            }
        }

        let monitors = &self.monitors;
        for i in &mut self.instances {
            if now < i.next_update || (i.shown && is_hidden(&i.rect(), hidden)) {
                continue;
            }
            refresh(i, &self.text, &mut self.images, &self.system, monitors, gpu, now)?;
        }
        Ok(())
    }

    /// O usuário soltou a skin num lugar novo.
    fn moved(&mut self, hwnd: HWND) {
        let Some(index) = self.instances.iter().position(|i| i.hwnd == hwnd) else { return };
        let Some(rect) = tracked_rect(hwnd) else { return };
        let center = POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
        let device = self.monitors.iter().find(|m| contains(&m.rect, center)).map(|m| m.device_name.clone());
        let i = &mut self.instances[index];
        i.entry.anchor = SkinAnchor::Free;
        i.entry.x = rect.left as f32;
        i.entry.y = rect.top as f32;
        if device.is_some() {
            i.entry.monitor = device;
        }
        log(&format!("Skin {} arrastada para ({}, {})", i.entry.config, rect.left, rect.top));
        if i.entry.save_position {
            self.save(index);
        }
    }

    /// Grava a skin e os favoritos no config.json, sem apagar o resto do que está lá.
    fn save(&self, index: usize) {
        if !self.persist {
            return;
        }
        let entry = self.instances[index].entry.clone();
        let favorites = self.favorites.clone();
        let result = kage_core::Config::edit_file(&kage_core::config_path(), |c| {
            match c.skin_mut(&entry.id) {
                Some(e) => *e = entry.clone(),
                None => c.skins.items.push(entry.clone()),
            }
            c.skins.favorites = favorites;
        });
        if let Err(e) = result {
            log(&format!("Não gravou a skin {}: {e}", entry.config));
        }
    }

    /// O menu do clique direito, com as opções do Rainmeter.
    fn menu(&mut self, index: usize, at: POINT) {
        let i = &self.instances[index];
        let hwnd = i.hwnd;
        let e = &i.entry;
        let favorite = self.favorites.contains(&kage_core::skin_key(&e.config));
        let current_monitor = monitor_of(e, &self.monitors).map(|m| m.number);
        // Título com o nome da skin (sem a pasta do pacote) e o monitor onde ela está.
        let name = e.config.rsplit('\\').next().unwrap_or(&e.config);
        let title = match current_monitor {
            Some(n) => format!("{name}  ·  Monitor {n}"),
            None => name.to_string(),
        };
        let command = unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            append(menu, MF_STRING, CMD_KAGE as usize, &title);
            let _ = SetMenuDefaultItem(menu, CMD_KAGE, 0);
            separator(menu);

            if let Ok(settings) = CreatePopupMenu() {
                if let Ok(position) = CreatePopupMenu() {
                    if let Ok(screens) = CreatePopupMenu() {
                        for (n, m) in self.monitors.iter().enumerate() {
                            let label = format!("Monitor {} ({}x{})", m.number, m.width(), m.height());
                            append(screens, checked(current_monitor == Some(m.number)), (CMD_MONITOR + n as u32) as usize, &label);
                        }
                        append(position, MF_POPUP, screens.0 as usize, "Monitores");
                    }
                    separator(position);
                    for (n, (layer, label)) in LAYERS.iter().enumerate() {
                        append(position, checked(e.layer == *layer), (CMD_LAYER + n as u32) as usize, label);
                    }
                    append(settings, MF_POPUP, position.0 as usize, "Posição");
                }
                separator(settings);
                if let Ok(alpha) = CreatePopupMenu() {
                    let current = item_for_alpha(e.alpha);
                    for n in 0..=10u32 {
                        let label = if n == 10 { "~100%".to_string() } else { format!("{}%", n * 10) };
                        append(alpha, checked(current == n), (CMD_ALPHA + n) as usize, &label);
                    }
                    append(settings, MF_POPUP, alpha.0 as usize, "Transparência");
                }
                if let Ok(hover) = CreatePopupMenu() {
                    for (n, (mode, label)) in HOVERS.iter().enumerate() {
                        append(hover, checked(e.on_hover == *mode), (CMD_HOVER + n as u32) as usize, label);
                    }
                    append(settings, MF_POPUP, hover.0 as usize, "Ao passar o mouse");
                }
                separator(settings);
                append(settings, checked(e.click_through), CMD_CLICK_THROUGH as usize, "Clicar através da skin");
                append(settings, checked(e.draggable), CMD_DRAGGABLE as usize, "Arrastável");
                append(settings, checked(e.keep_on_screen), CMD_KEEP_ON_SCREEN as usize, "Manter na tela");
                append(settings, checked(e.save_position), CMD_SAVE_POSITION as usize, "Salvar posição");
                append(settings, checked(e.snap_edges), CMD_SNAP as usize, "Grudar nas bordas");
                append(settings, checked(favorite), CMD_FAVORITE as usize, "Favoritar");
                append(menu, MF_POPUP, settings.0 as usize, "Configurações");
            }
            separator(menu);
            append(menu, MF_STRING, CMD_KAGE as usize, "Abrir no Kage");
            append(menu, MF_STRING, CMD_EDIT as usize, "Editar skin");
            append(menu, MF_STRING, CMD_RELOAD as usize, "Recarregar skin");
            separator(menu);
            append(menu, MF_STRING, CMD_OFF as usize, "Desligar skin");

            theme_menus(hwnd);
            // Sem trazer a skin para a frente, o menu não fecha ao clicar fora.
            let _ = SetForegroundWindow(hwnd);
            let command = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, at.x, at.y, hwnd, None).0 as u32;
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            command
        };
        if command != 0 {
            self.run(index, command);
        }
    }

    fn run(&mut self, index: usize, command: u32) {
        let monitors = self.monitors.clone();
        let i = &mut self.instances[index];
        let mut entry = i.entry.clone();
        match command {
            CMD_KAGE => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe.with_file_name("kage.exe")).spawn();
                }
                return;
            }
            CMD_EDIT => {
                let _ = std::process::Command::new("notepad").arg(&entry.ini).spawn();
                return;
            }
            CMD_RELOAD => {
                match Skin::load(&entry.ini, &skins_root_for(&entry.ini)) {
                    Ok(skin) => i.skin = skin,
                    Err(e) => log(&format!("Skin {} não recarregou: {e}", entry.config)),
                }
                i.invalidate();
                return;
            }
            CMD_OFF => {
                entry.active = false;
                i.entry = entry;
                log(&format!("Skin {} desligada pelo menu", i.entry.config));
                self.save(index);
                self.instances.remove(index);
                return;
            }
            CMD_CLICK_THROUGH => entry.click_through = !entry.click_through,
            CMD_DRAGGABLE => entry.draggable = !entry.draggable,
            CMD_KEEP_ON_SCREEN => entry.keep_on_screen = !entry.keep_on_screen,
            CMD_SAVE_POSITION => entry.save_position = !entry.save_position,
            CMD_SNAP => entry.snap_edges = !entry.snap_edges,
            CMD_FAVORITE => {
                let key = kage_core::skin_key(&entry.config);
                if self.favorites.contains(&key) {
                    self.favorites.retain(|f| *f != key);
                } else {
                    self.favorites.push(key);
                }
                self.save(index);
                return;
            }
            c if (CMD_MONITOR..CMD_LAYER).contains(&c) => {
                let Some(target) = monitors.get((c - CMD_MONITOR) as usize) else { return };
                if entry.anchor == SkinAnchor::Free {
                    // Mesmo lugar relativo no outro monitor.
                    let from = monitor_of(&entry, &monitors).map(|m| m.rect).unwrap_or(target.rect);
                    let rect = i.rect();
                    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
                    let x = target.rect.left + (entry.x as i32 - from.left);
                    let y = target.rect.top + (entry.y as i32 - from.top);
                    entry.x = x.clamp(target.rect.left, (target.rect.right - w).max(target.rect.left)) as f32;
                    entry.y = y.clamp(target.rect.top, (target.rect.bottom - h).max(target.rect.top)) as f32;
                }
                entry.monitor = Some(target.device_name.clone());
            }
            c if (CMD_LAYER..CMD_ALPHA).contains(&c) => match LAYERS.get((c - CMD_LAYER) as usize) {
                Some((layer, _)) => entry.layer = *layer,
                None => return,
            },
            c if (CMD_ALPHA..CMD_HOVER).contains(&c) => entry.alpha = alpha_for_item(c - CMD_ALPHA),
            c if (CMD_HOVER..CMD_HOVER + 4).contains(&c) => entry.on_hover = HOVERS[(c - CMD_HOVER) as usize].0,
            _ => return,
        }
        i.update(entry);
        self.save(index);
    }
}

impl Drop for SkinHost {
    fn drop(&mut self) {
        self.instances.clear();
        if !self.hook.is_invalid() {
            unsafe {
                let _ = UnhookWinEvent(self.hook);
            }
        }
    }
}

/// A skin está toda dentro de monitores cobertos (tela cheia): não precisa redesenhar.
fn is_hidden(rect: &RECT, hidden: &[RECT]) -> bool {
    let center = POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
    hidden.iter().any(|h| contains(h, center))
}

fn refresh(
    i: &mut Instance,
    text: &TextEngine,
    images: &mut ImageCache,
    system: &WinSystem,
    monitors: &[Monitor],
    gpu: &Gpu,
    now: Instant,
) -> Result<()> {
    let frame = i.skin.update(&skins::local_datetime(), &Measurer { text, gpu }, system);
    if i.frame.as_ref() != Some(&frame) {
        let size = (frame.width.ceil().max(1.0) as u32, frame.height.ceil().max(1.0) as u32);
        if i.bitmap.as_ref().is_none_or(|(_, s)| *s != size) {
            i.bitmap = Some((skins::layered_bitmap(gpu, size.0, size.1)?, size));
        }
        let (x, y) = skins::place_skin(placement(&i.entry, monitors), (size.0 as f32, size.1 as f32));
        let pos = POINT { x: x as i32, y: y as i32 };
        let bitmap = i.bitmap.as_ref().map(|(b, _)| b.clone()).expect("imagem criada acima");
        let (hwnd, alpha) = (i.hwnd, i.alpha);
        skins::render_layered(text, gpu, &bitmap, &frame, images, &|| present(gpu, hwnd, pos, size, alpha))?;
        TRACKED.with_borrow_mut(|t| {
            if let Some(tr) = t.get_mut(&key(hwnd)) {
                tr.rect = RECT { left: pos.x, top: pos.y, right: pos.x + size.0 as i32, bottom: pos.y + size.1 as i32 };
            }
        });
        i.frame = Some(frame);
        if !i.shown {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            apply_layer(hwnd, i.entry.layer);
            i.shown = true;
        }
    }
    i.next_update = match i.skin.update_ms {
        Some(ms) => now + Duration::from_millis(ms.max(16)),
        None => now + Duration::from_secs(60 * 60 * 24 * 365),
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_sticks_to_the_nearest_edge_within_ten_pixels() {
        // Encosta o começo da skin na borda 0 e o fim na borda 1920.
        assert_eq!(snap_axis(7, 300, &[0, 1920]), 0);
        assert_eq!(snap_axis(1615, 300, &[0, 1920]), 1620);
        assert_eq!(snap_axis(500, 300, &[0, 1920]), 500, "longe das bordas fica onde está");
    }

    #[test]
    fn transparency_items_follow_rainmeter() {
        assert_eq!(alpha_for_item(0), 255);
        assert_eq!(alpha_for_item(9), 25);
        assert_eq!(alpha_for_item(10), 1);
        for item in 0..=10 {
            assert_eq!(item_for_alpha(alpha_for_item(item)), item);
        }
    }

    #[test]
    fn hover_targets() {
        let mut entry = SkinEntry { alpha: 200, ..Default::default() };
        assert_eq!(target_alpha(&entry, true), 200.0);
        entry.on_hover = SkinHover::FadeIn;
        assert_eq!((target_alpha(&entry, false), target_alpha(&entry, true)), (200.0, 255.0));
        entry.on_hover = SkinHover::Hide;
        assert_eq!(target_alpha(&entry, true), 1.0, "escondida, mas ainda com o mouse");
    }
}
