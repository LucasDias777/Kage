//! Configuração e biblioteca do Kage, compartilhadas pela janela (`kage`) e pelo motor
//! (`kage-engine`).
//!
//! A janela edita e salva o `config.json`; o motor lê. Depois de salvar, a janela manda a
//! mensagem [`MSG_RELOAD`] para a janela invisível do motor ([`ENGINE_WINDOW_CLASS`]), e o motor
//! recarrega na hora, sem ficar conferindo o arquivo.

mod config;
pub mod release;

pub use config::*;

use std::path::PathBuf;

/// Classe da janela invisível do motor, que recebe os avisos da janela do Kage.
pub const ENGINE_WINDOW_CLASS: &str = "KageSystemEvents";
/// WM_APP + 16: recarregar a configuração.
pub const MSG_RELOAD: u32 = 0x8000 + 16;
/// WM_APP + 17: encerrar o motor.
pub const MSG_QUIT: u32 = 0x8000 + 17;

/// Propriedade (SetPropW) que marca a janela do Kage, para o motor e uma segunda cópia a acharem.
pub const UI_WINDOW_PROP: &str = "KageJanela";
/// Mensagem registrada (RegisterWindowMessageW) que pede à janela aberta para mostrar uma tela.
pub const UI_COMMAND_MESSAGE: &str = "KageJanelaComando";

/// Telas que a bandeja (ou uma segunda abertura) pede à janela. Na linha de comando:
/// `kage.exe --abrir atualizacoes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiCommand {
    Show = 0,
    Updates = 1,
    Customize = 2,
    News = 3,
}

impl UiCommand {
    pub fn arg(self) -> &'static str {
        match self {
            Self::Show => "inicio",
            Self::Updates => "atualizacoes",
            Self::Customize => "customizar",
            Self::News => "novidades",
        }
    }

    pub fn from_arg(arg: &str) -> Option<Self> {
        [Self::Show, Self::Updates, Self::Customize, Self::News].into_iter().find(|c| c.arg() == arg)
    }

    pub fn from_code(code: usize) -> Option<Self> {
        [Self::Show, Self::Updates, Self::Customize, Self::News].into_iter().find(|c| *c as usize == code)
    }
}

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
