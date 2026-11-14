//! Lista os monitores conectados, em pixels físicos.

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows_core::BOOL;

#[derive(Clone, Debug)]
pub struct Monitor {
    /// Número exibido ao usuário, a partir de 1, na ordem do Windows (\\.\DISPLAY1, 2...).
    pub number: usize,
    pub device_name: String,
    pub rect: RECT,
    pub primary: bool,
}

impl Monitor {
    pub fn width(&self) -> u32 {
        (self.rect.right - self.rect.left) as u32
    }

    pub fn height(&self) -> u32 {
        (self.rect.bottom - self.rect.top) as u32
    }
}

pub fn enumerate() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut list as *mut Vec<Monitor> as isize),
        );
    }
    list.sort_by_key(|m| display_index(&m.device_name));
    for (i, m) in list.iter_mut().enumerate() {
        m.number = i + 1;
    }
    list
}

unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) };
    if ok.as_bool() {
        let name_len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        list.push(Monitor {
            number: 0,
            device_name: String::from_utf16_lossy(&info.szDevice[..name_len]),
            rect: info.monitorInfo.rcMonitor,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    BOOL(1)
}

/// Extrai o número de "\\.\DISPLAY2" para ordenar como o Windows mostra.
fn display_index(device_name: &str) -> u32 {
    device_name
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .unwrap_or(u32::MAX)
}
