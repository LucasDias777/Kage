//! Ícone do Kage na bandeja do sistema. Duplo clique abre a janela; o clique direito abre o menu:
//! Abrir o Kage, Fechar todos os wallpapers, Pausar wallpapers, Alterar wallpaper, Customizar
//! wallpaper, o estado da atualização, Relatar um problema e Sair. Pausado pelo menu, o ícone
//! fica cinza. O motor também procura versão nova (logo depois de abrir e a cada 12 horas) e
//! avisa por um balão quando há uma.
//!
//! Tudo roda na thread do motor, dentro do tratamento de mensagens da janela invisível dele;
//! só a consulta ao GitHub vai para outra thread e volta por mensagem.

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use kage_core::UiCommand;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    SHDefExtractIconW, ShellExecuteW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, EnumWindows, GetCursorPos, GetIconInfo, GetPropW,
    GetSystemMetrics, HICON, ICONINFO, IsIconic, MENU_ITEM_FLAGS, MF_CHECKED, MF_GRAYED, MF_STRING, PostMessageW,
    RegisterWindowMessageW, SM_CXSMICON, SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, SetMenuDefaultItem, ShowWindow,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_CLOSE, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP,
};
use windows_core::{BOOL, HSTRING, PCWSTR, w};

use crate::skinhost::{append, separator, theme_menus};
use crate::{log, monitors, watch};

/// WM_APP + 48: aviso do ícone (clique, balão).
pub const MSG_TRAY: u32 = 0x8000 + 48;
/// WM_APP + 49: a consulta ao GitHub terminou.
pub const MSG_UPDATE_CHECKED: u32 = 0x8000 + 49;
/// WM_USER + 5: clicaram no balão.
const NIN_BALLOONUSERCLICK: u32 = 0x0400 + 5;
const ICON_ID: u32 = 1;
const ISSUES_URL: &str = "https://github.com/LucasDias777/Kage/issues/new";
const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(12 * 60 * 60);

const ID_OPEN: usize = 1;
const ID_CLOSE_ALL: usize = 2;
const ID_PAUSE: usize = 3;
const ID_CHANGE: usize = 4;
const ID_CUSTOMIZE: usize = 5;
const ID_UPDATE: usize = 6;
const ID_REPORT: usize = 7;
const ID_EXIT: usize = 8;

/// Estado da procura de versão nova, como o item do menu mostra.
const CHECKING: u8 = 0;
const UP_TO_DATE: u8 = 1;
const AVAILABLE: u8 = 2;
const FAILED: u8 = 3;
const NOT_CHECKING: u8 = 4;

static OWNER: AtomicIsize = AtomicIsize::new(0);
static SHOWN: AtomicBool = AtomicBool::new(false);
static PAUSED: AtomicBool = AtomicBool::new(false);
static UPDATE: AtomicU8 = AtomicU8::new(CHECKING);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static LATEST: Mutex<String> = Mutex::new(String::new());
static ANNOUNCED: Mutex<String> = Mutex::new(String::new());

thread_local! {
    /// Ícone normal e o cinza (pausado). Só a thread do motor mexe neles.
    static ICONS: RefCell<Option<(HICON, HICON)>> = const { RefCell::new(None) };
}

/// Os wallpapers estão pausados pelo menu da bandeja (as skins continuam).
pub fn user_paused() -> bool {
    PAUSED.load(Ordering::SeqCst)
}

/// Prepara o ícone (sem mostrar) e começa a procurar versão nova em segundo plano.
pub fn init(owner: HWND) {
    OWNER.store(owner.0 as isize, Ordering::SeqCst);
    TASKBAR_CREATED.store(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }, Ordering::SeqCst);
    ICONS.with(|icons| *icons.borrow_mut() = load_icons());
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            check_for_update();
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// Mostra ou tira o ícone (opção "Ícone da bandeja do sistema").
pub fn set_visible(visible: bool) {
    if visible == SHOWN.load(Ordering::SeqCst) {
        return;
    }
    if visible {
        if notify(NIM_ADD, |_| {}) {
            SHOWN.store(true, Ordering::SeqCst);
        }
    } else {
        notify(NIM_DELETE, |_| {});
        SHOWN.store(false, Ordering::SeqCst);
    }
}

/// Tira o ícone ao encerrar (sem isso ele fica na bandeja até alguém passar o mouse).
pub fn remove() {
    set_visible(false);
    ICONS.with(|icons| {
        if let Some((normal, grey)) = icons.borrow_mut().take() {
            unsafe {
                let _ = DestroyIcon(normal);
                let _ = DestroyIcon(grey);
            }
        }
    });
}

/// Mensagens da janela invisível que são da bandeja. Devolve verdadeiro se tratou.
pub fn handle(msg: u32, lparam: LPARAM) -> bool {
    if msg == MSG_TRAY {
        match lparam.0 as u32 {
            WM_LBUTTONDBLCLK => open_ui(UiCommand::Show),
            WM_RBUTTONUP => show_menu(),
            NIN_BALLOONUSERCLICK => open_ui(UiCommand::Updates),
            _ => {}
        }
        return true;
    }
    if msg == MSG_UPDATE_CHECKED {
        announce_update();
        return true;
    }
    let taskbar = TASKBAR_CREATED.load(Ordering::SeqCst);
    if taskbar != 0 && msg == taskbar {
        // O Explorer reiniciou: a bandeja nova não tem o ícone.
        if SHOWN.swap(false, Ordering::SeqCst) {
            set_visible(true);
        }
        return true;
    }
    false
}

fn owner() -> HWND {
    HWND(OWNER.load(Ordering::SeqCst) as *mut _)
}

/// Adiciona, muda ou tira o ícone. `extra` completa os dados (balão).
fn notify(action: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE, extra: impl FnOnce(&mut NOTIFYICONDATAW)) -> bool {
    let icon = ICONS.with(|icons| icons.borrow().map(|(normal, grey)| if user_paused() { grey } else { normal }));
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: owner(),
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_TIP | if icon.is_some() { NIF_ICON } else { Default::default() },
        uCallbackMessage: MSG_TRAY,
        hIcon: icon.unwrap_or_default(),
        ..Default::default()
    };
    let tip = if user_paused() { "Kage (pausado)" } else { "Kage" };
    copy_text(&mut data.szTip, tip);
    extra(&mut data);
    unsafe { Shell_NotifyIconW(action, &data) }.as_bool()
}

fn copy_text(target: &mut [u16], text: &str) {
    let wide: Vec<u16> = text.encode_utf16().take(target.len() - 1).collect();
    target[..wide.len()].copy_from_slice(&wide);
    target[wide.len()] = 0;
}

/// Ícone pequeno do próprio .exe, no tamanho da bandeja nesta tela, e uma cópia cinza.
fn load_icons() -> Option<(HICON, HICON)> {
    let exe = std::env::current_exe().ok()?;
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    let mut small = HICON::default();
    unsafe { SHDefExtractIconW(&HSTRING::from(exe.as_os_str()), 0, 0, None, Some(&mut small), (size << 16) | size) }
        .ok()
        .ok()?;
    if small.is_invalid() {
        return None;
    }
    let grey = greyed(small).unwrap_or(small);
    Some((small, grey))
}

/// Cópia do ícone em tons de cinza e mais apagada, para o estado pausado.
fn greyed(icon: HICON) -> Option<HICON> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let free = |b: HBITMAP| {
            if !b.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(b.0));
            }
        };
        let mut bitmap = BITMAP::default();
        let read = GetObjectW(HGDIOBJ(info.hbmColor.0), size_of::<BITMAP>() as i32, Some((&mut bitmap as *mut BITMAP).cast()));
        if read == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight <= 0 {
            free(info.hbmColor);
            free(info.hbmMask);
            return None;
        }
        let (width, height) = (bitmap.bmWidth, bitmap.bmHeight);
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = GetDC(None);
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        GetDIBits(dc, info.hbmColor, 0, height as u32, Some(pixels.as_mut_ptr().cast()), &mut header, DIB_RGB_COLORS);
        for px in pixels.as_chunks_mut::<4>().0 {
            let luma = (px[2] as u32 * 30 + px[1] as u32 * 59 + px[0] as u32 * 11) / 100;
            let grey = (luma * 3 / 4 + 40).min(255) as u8;
            px[0] = grey;
            px[1] = grey;
            px[2] = grey;
            px[3] = (px[3] as u32 * 3 / 4) as u8;
        }
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(Some(dc), &header, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, dc);
        let result = match color {
            Ok(color) if !bits.is_null() => {
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast(), pixels.len());
                let grey_info = ICONINFO { fIcon: BOOL(1), hbmMask: info.hbmMask, hbmColor: color, ..Default::default() };
                let made = CreateIconIndirect(&grey_info).ok();
                free(color);
                made
            }
            _ => None,
        };
        free(info.hbmColor);
        free(info.hbmMask);
        result
    }
}

/// Consulta o GitHub (em outra thread) e avisa o motor pela janela invisível.
fn check_for_update() {
    let config = kage_core::Config::load_from(&kage_core::config_path()).unwrap_or_default();
    if !config.general.check_updates {
        UPDATE.store(NOT_CHECKING, Ordering::SeqCst);
        return;
    }
    UPDATE.store(CHECKING, Ordering::SeqCst);
    let current = kage_core::release::current_version();
    let state = match kage_core::release::latest() {
        Ok(Some(release)) if kage_core::release::is_newer(&release.version, &current) => {
            *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = release.version;
            AVAILABLE
        }
        Ok(_) => UP_TO_DATE,
        Err(e) => {
            log(&format!("Atualização: não deu para procurar ({e})"));
            FAILED
        }
    };
    UPDATE.store(state, Ordering::SeqCst);
    unsafe {
        let _ = PostMessageW(Some(owner()), MSG_UPDATE_CHECKED, WPARAM(0), LPARAM(0));
    }
}

/// Versão nova achada: balão uma vez por versão (com o ícone à vista).
fn announce_update() {
    if UPDATE.load(Ordering::SeqCst) != AVAILABLE {
        return;
    }
    let version = LATEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut announced = ANNOUNCED.lock().unwrap_or_else(|e| e.into_inner());
    if *announced == version {
        return;
    }
    log(&format!("Atualização: versão {version} disponível"));
    if SHOWN.load(Ordering::SeqCst) {
        notify(NIM_MODIFY, |data| {
            data.uFlags |= NIF_INFO;
            data.dwInfoFlags = NIIF_INFO;
            copy_text(&mut data.szInfoTitle, &format!("Kage {version} disponível"));
            copy_text(&mut data.szInfo, "Clique para ver o que muda e atualizar.");
        });
    }
    *announced = version;
}

fn update_item() -> (MENU_ITEM_FLAGS, String) {
    match UPDATE.load(Ordering::SeqCst) {
        UP_TO_DATE => (MF_STRING | MF_GRAYED, "O Kage está atualizado".into()),
        AVAILABLE => {
            let version = LATEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
            (MF_STRING, format!("Atualização disponível: versão {version}"))
        }
        FAILED => (MF_STRING, "Não deu para procurar atualização".into()),
        NOT_CHECKING => (MF_STRING, "Procurar atualização".into()),
        _ => (MF_STRING | MF_GRAYED, "Procurando atualização...".into()),
    }
}

fn show_menu() {
    let config = kage_core::Config::load_from(&kage_core::config_path()).unwrap_or_default();
    let has_wallpaper = config.current_wallpaper().is_some();
    let enabled = |on: bool| if on { MF_STRING } else { MF_STRING | MF_GRAYED };
    let owner = owner();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        append(menu, MF_STRING, ID_OPEN, "Abrir o Kage");
        let _ = SetMenuDefaultItem(menu, ID_OPEN as u32, 0);
        separator(menu);
        append(menu, enabled(has_wallpaper), ID_CLOSE_ALL, "Fechar todos os wallpapers");
        append(menu, if user_paused() { MF_STRING | MF_CHECKED } else { MF_STRING }, ID_PAUSE, "Pausar wallpapers");
        append(menu, enabled(!config.library.is_empty()), ID_CHANGE, "Alterar wallpaper");
        append(menu, enabled(has_wallpaper), ID_CUSTOMIZE, "Customizar wallpaper");
        separator(menu);
        let (flags, text) = update_item();
        append(menu, flags, ID_UPDATE, &text);
        separator(menu);
        append(menu, MF_STRING, ID_REPORT, "Relatar um problema");
        separator(menu);
        append(menu, MF_STRING, ID_EXIT, "Sair");
        theme_menus(owner);
        // Sem a janela do motor em primeiro plano, o menu não fecha ao clicar fora.
        let _ = SetForegroundWindow(owner);
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        let command = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, at.x, at.y, owner, None).0 as usize;
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        run(command);
    }
}

fn run(command: usize) {
    match command {
        ID_OPEN => open_ui(UiCommand::Show),
        ID_CLOSE_ALL => edit_config("Bandeja: wallpapers fechados", |c| {
            c.clear_wallpapers();
            true
        }),
        ID_PAUSE => {
            let paused = !PAUSED.fetch_xor(true, Ordering::SeqCst);
            log(if paused { "Bandeja: wallpapers pausados" } else { "Bandeja: wallpapers de volta" });
            if SHOWN.load(Ordering::SeqCst) {
                notify(NIM_MODIFY, |_| {});
            }
        }
        ID_CHANGE => {
            let monitors = monitors::enumerate();
            let pairs = monitors::id_pairs(&monitors);
            let devices: Vec<String> = monitors.into_iter().map(|m| m.id).collect();
            let seed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
            edit_config("Bandeja: wallpaper sorteado", |c| {
                c.adopt_monitor_ids(&pairs);
                c.random_wallpapers(&devices, seed)
            });
        }
        ID_CUSTOMIZE => open_ui(UiCommand::Customize),
        ID_UPDATE => {
            if UPDATE.load(Ordering::SeqCst) == NOT_CHECKING {
                UPDATE.store(CHECKING, Ordering::SeqCst);
            }
            open_ui(UiCommand::Updates);
        }
        ID_REPORT => open_url(ISSUES_URL),
        ID_EXIT => {
            if let Some(ui) = ui_window() {
                unsafe {
                    let _ = PostMessageW(Some(ui), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            log("Bandeja: Sair");
            watch::request_quit();
        }
        _ => {}
    }
}

/// Muda o config.json como a janela faria e recarrega o motor.
fn edit_config(message: &str, change: impl FnOnce(&mut kage_core::Config) -> bool) {
    let mut changed = false;
    match kage_core::Config::edit_file(&kage_core::config_path(), |c| changed = change(c)) {
        Ok(()) if changed => {
            log(message);
            watch::request_reload();
        }
        Ok(()) => {}
        Err(e) => log(&format!("Bandeja: não salvou ({e})")),
    }
}

/// A janela do Kage aberta, achada pela marca que ela põe em si mesma.
fn ui_window() -> Option<HWND> {
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
        let found = unsafe { &mut *(data.0 as *mut Option<HWND>) };
        if !unsafe { GetPropW(hwnd, &HSTRING::from(kage_core::UI_WINDOW_PROP)) }.is_invalid() {
            *found = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut found: Option<HWND> = None;
    unsafe {
        let _ = EnumWindows(Some(find), LPARAM(&mut found as *mut Option<HWND> as isize));
    }
    found
}

/// Traz a janela do Kage para a frente (ou abre) já na tela pedida.
pub fn open_ui(command: UiCommand) {
    if let Some(ui) = ui_window() {
        unsafe {
            if IsIconic(ui).as_bool() {
                let _ = ShowWindow(ui, SW_RESTORE);
            }
            let _ = SetForegroundWindow(ui);
            if command != UiCommand::Show {
                let message = RegisterWindowMessageW(&HSTRING::from(kage_core::UI_COMMAND_MESSAGE));
                let _ = PostMessageW(Some(ui), message, WPARAM(command as usize), LPARAM(0));
            }
        }
        return;
    }
    let exe = std::env::current_exe().unwrap_or_default().with_file_name("kage.exe");
    let mut process = std::process::Command::new(&exe);
    if command != UiCommand::Show {
        process.args(["--abrir", command.arg()]);
    }
    if let Err(e) = process.spawn() {
        log(&format!("Bandeja: não abriu a janela ({}): {e}", exe.display()));
    }
}

fn open_url(url: &str) {
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_text_is_cut_to_fit_and_ends_with_zero() {
        let mut buffer = [1u16; 8];
        copy_text(&mut buffer, "Kage (pausado)");
        assert_eq!(String::from_utf16_lossy(&buffer[..7]), "Kage (p");
        assert_eq!(buffer[7], 0);
    }
}
