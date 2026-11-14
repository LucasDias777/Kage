//! Camada da área de trabalho atrás dos ícones (WorkerW).
//!
//! O Explorer cria um WorkerW entre o papel de parede e os ícones quando recebe a
//! mensagem 0x052C no Progman. Janelas filhas desse WorkerW aparecem atrás dos ícones.

use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::MapWindowPoints;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GWL_STYLE, GetWindowLongPtrW, SMTO_NORMAL,
    SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SendMessageTimeoutW, SetParent, SetWindowLongPtrW,
    SetWindowPos, SystemParametersInfoW, WS_CHILD, WS_POPUP, WS_VISIBLE,
};
use windows_core::{BOOL, Error, Result, w};

const SPAWN_WORKERW: u32 = 0x052C;

pub fn find_workerw() -> Result<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None)?;
        let mut _reply = 0usize;
        SendMessageTimeoutW(
            progman,
            SPAWN_WORKERW,
            WPARAM(0xD),
            LPARAM(0x1),
            SMTO_NORMAL,
            1000,
            Some(&mut _reply),
        );

        let mut found = HWND::default();
        let _ = EnumWindows(Some(find_sibling_workerw), LPARAM(&mut found as *mut HWND as isize));

        // No Windows 11 24H2 o WorkerW passou a ficar dentro do próprio Progman.
        if found.is_invalid()
            && let Ok(inner) = FindWindowExW(Some(progman), None, w!("WorkerW"), None)
        {
            found = inner;
        }
        if found.is_invalid() {
            return Err(Error::new(E_FAIL, "WorkerW não encontrado"));
        }
        Ok(found)
    }
}

/// Janela de topo com os ícones da área de trabalho (Progman ou o WorkerW com SHELLDLL_DefView).
/// As skins "No desktop" ficam logo acima dela.
pub fn icon_host() -> Option<HWND> {
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
        let has_icons = unsafe { FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None) }
            .map(|h| !h.is_invalid())
            .unwrap_or(false);
        if has_icons {
            unsafe { *(data.0 as *mut HWND) = hwnd };
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut found = HWND::default();
    unsafe {
        let _ = EnumWindows(Some(find), LPARAM(&mut found as *mut HWND as isize));
    }
    if found.is_invalid() { unsafe { FindWindowW(w!("Progman"), None).ok() } } else { Some(found) }
}

/// Acha a janela que contém os ícones (SHELLDLL_DefView); o WorkerW certo é o irmão seguinte.
unsafe extern "system" fn find_sibling_workerw(hwnd: HWND, data: LPARAM) -> BOOL {
    unsafe {
        let has_icons = FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None)
            .map(|h| !h.is_invalid())
            .unwrap_or(false);
        if has_icons && let Ok(workerw) = FindWindowExW(None, Some(hwnd), w!("WorkerW"), None) {
            *(data.0 as *mut HWND) = workerw;
            return BOOL(0);
        }
    }
    BOOL(1)
}

/// Prende a janela no WorkerW e a posiciona sobre o retângulo do monitor (coordenadas de tela).
pub fn attach(hwnd: HWND, workerw: HWND, screen_rect: RECT) -> Result<()> {
    unsafe {
        SetParent(hwnd, Some(workerw))?;
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let style = (style & !(WS_POPUP.0 as isize)) | (WS_CHILD.0 | WS_VISIBLE.0) as isize;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style);

        // O WorkerW cobre a tela virtual inteira; o monitor da esquerda tem X negativo.
        let mut corners = [
            POINT { x: screen_rect.left, y: screen_rect.top },
            POINT { x: screen_rect.right, y: screen_rect.bottom },
        ];
        MapWindowPoints(None, Some(workerw), &mut corners);
        SetWindowPos(
            hwnd,
            None,
            corners[0].x,
            corners[0].y,
            corners[1].x - corners[0].x,
            corners[1].y - corners[0].y,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
        )
    }
}

/// Redesenha o papel de parede estático ao sair, para não deixar o último quadro preso na tela.
pub fn refresh_wallpaper() {
    let mut path = [0u16; 260];
    unsafe {
        let got = SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            path.len() as u32,
            Some(path.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        // Caminho vazio apagaria o papel de parede; nesse caso não mexe.
        if got.is_ok() && path[0] != 0 {
            let _ = SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(path.as_mut_ptr().cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
        }
    }
}
