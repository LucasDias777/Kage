//! Tema claro ou escuro: o escolhido na janela ou o do Windows.

use kage_core::ThemeChoice;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_core::w;

/// Modo de apps do Windows (Configurações > Personalização > Cores).
pub fn system_is_light() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    result.is_ok() && value != 0
}

pub fn is_dark(choice: ThemeChoice) -> bool {
    match choice {
        ThemeChoice::System => !system_is_light(),
        ThemeChoice::Light => false,
        ThemeChoice::Dark => true,
    }
}
