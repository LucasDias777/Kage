//! Monitores conectados, para o mapa da página Monitores.

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows_core::BOOL;

#[derive(Clone, Debug)]
pub struct Monitor {
    pub number: usize,
    /// `\\.\DISPLAY1`: é por esse nome que a configuração guarda o wallpaper de cada monitor.
    pub device: String,
    pub rect: RECT,
    pub primary: bool,
}

pub fn enumerate() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut list as *mut Vec<Monitor> as isize));
    }
    list.sort_by_key(|m| m.device.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u32>().unwrap_or(u32::MAX));
    for (i, m) in list.iter_mut().enumerate() {
        m.number = i + 1;
    }
    list
}

unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) }.as_bool() {
        let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        list.push(Monitor {
            number: 0,
            device: String::from_utf16_lossy(&info.szDevice[..len]),
            rect: info.monitorInfo.rcMonitor,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    BOOL(1)
}
