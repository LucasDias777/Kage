//! "Iniciar com o Windows": a entrada `Kage` em HKCU\...\Run aponta para o motor.
//! Só muda quando o usuário liga ou desliga a opção na janela.

use std::path::Path;

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_core::{HSTRING, w};

const RUN_KEY: windows_core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: windows_core::PCWSTR = w!("Kage");

pub fn is_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, RRF_RT_REG_SZ, None, None, None) }.is_ok()
}

pub fn set(enabled: bool, engine: &Path) -> Result<(), String> {
    let result = unsafe {
        if enabled {
            let command = HSTRING::from(format!("\"{}\"", engine.display()));
            let bytes = (command.len() + 1) * 2;
            RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, REG_SZ.0, Some(command.as_ptr().cast()), bytes as u32)
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE)
        }
    };
    if result.is_ok() || !enabled { Ok(()) } else { Err(format!("o Windows recusou: {result:?}")) }
}
