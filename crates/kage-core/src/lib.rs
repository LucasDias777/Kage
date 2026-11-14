//! Configuração e biblioteca do Kage, compartilhadas pela janela (`kage`) e pelo motor
//! (`kage-engine`).
//!
//! A janela edita e salva o `config.json`; o motor lê. Depois de salvar, a janela manda a
//! mensagem [`MSG_RELOAD`] para a janela invisível do motor ([`ENGINE_WINDOW_CLASS`]), e o motor
//! recarrega na hora, sem ficar conferindo o arquivo.

mod config;

pub use config::*;

use std::path::PathBuf;

/// Classe da janela invisível do motor, que recebe os avisos da janela do Kage.
pub const ENGINE_WINDOW_CLASS: &str = "KageSystemEvents";
/// WM_APP + 16: recarregar a configuração.
pub const MSG_RELOAD: u32 = 0x8000 + 16;
/// WM_APP + 17: encerrar o motor.
pub const MSG_QUIT: u32 = 0x8000 + 17;

/// `%APPDATA%\Kage`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Kage")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn thumbnails_dir() -> PathBuf {
    data_dir().join("thumbnails")
}

pub fn engine_log_path() -> PathBuf {
    data_dir().join("engine.log")
}
