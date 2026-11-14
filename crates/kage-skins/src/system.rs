//! Valores do sistema para as medidas CPU, PhysicalMemory, SwapMemory, FreeDiskSpace, Net* e a
//! música tocando. Quem lê o Windows é o motor; aqui fica só a interface e o formato de número
//! do Rainmeter.

use std::path::PathBuf;

/// O que está tocando agora (os plugins NowPlaying e WebNowPlaying do Rainmeter).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowPlaying {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// Posição e duração em segundos.
    pub position: f64,
    pub duration: f64,
    /// 0 parado, 1 tocando, 2 pausado (o STATE do NowPlaying).
    pub state: u8,
    /// Capa num arquivo, quando o player manda uma.
    pub cover: Option<PathBuf>,
}

pub trait SystemProbe {
    /// Música tocando no Windows; `None` quando nada toca (ou não há quem leia).
    fn now_playing(&self) -> Option<NowPlaying> {
        None
    }
    /// Uso total do processador, 0 a 100.
    fn cpu_percent(&self) -> f64;
    /// Memória física (usada, total) em bytes.
    fn memory(&self) -> (f64, f64);
    /// Memória comprometida, RAM mais arquivo de paginação (usada, total) em bytes.
    fn swap(&self) -> (f64, f64);
    /// Disco (livre, total) em bytes. `drive` no formato do Rainmeter: "C:".
    fn disk(&self, drive: &str) -> Option<(f64, f64)>;
    /// Rede (entrada, saída) em bytes por segundo, somando as interfaces físicas.
    fn network(&self) -> (f64, f64);
}

/// Sem sistema: tudo zero (testes e conferência de skin sem medir nada).
pub struct NoSystem;

impl SystemProbe for NoSystem {
    fn cpu_percent(&self) -> f64 {
        0.0
    }
    fn memory(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
    fn swap(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
    fn disk(&self, _drive: &str) -> Option<(f64, f64)> {
        None
    }
    fn network(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
}

/// Como o medidor mostra um número (opções do medidor String).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumberFormat {
    pub decimals: usize,
    /// 0 = sem; 1 = base 1024; 2 = base 1000.
    pub autoscale: u8,
    pub percentual: bool,
    pub scale: f64,
}

/// Número como o Rainmeter escreve: percentual do intervalo, ou dividido por `Scale`, e com
/// AutoScale as letras k, M, G, T depois de um espaço.
pub fn format_value(value: f64, min: f64, max: f64, f: &NumberFormat) -> String {
    let decimals = f.decimals;
    if f.percentual {
        let percent = if max > min { (value - min) / (max - min) * 100.0 } else { 0.0 };
        return format!("{percent:.decimals$}");
    }
    let mut v = value / if f.scale == 0.0 { 1.0 } else { f.scale };
    if f.autoscale == 0 {
        return format!("{v:.decimals$}");
    }
    let base = if f.autoscale == 2 { 1000.0 } else { 1024.0 };
    const SUFFIX: [&str; 5] = ["", "k", "M", "G", "T"];
    let mut i = 0;
    while v.abs() >= base && i < SUFFIX.len() - 1 {
        v /= base;
        i += 1;
    }
    format!("{v:.decimals$} {}", SUFFIX[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(decimals: usize, autoscale: u8, percentual: bool) -> NumberFormat {
        NumberFormat { decimals, autoscale, percentual, scale: 1.0 }
    }

    #[test]
    fn autoscale_uses_1024_and_suffixes() {
        let gib = 1024.0 * 1024.0 * 1024.0;
        assert_eq!(format_value(8.5 * gib, 0.0, 0.0, &fmt(1, 1, false)), "8.5 G");
        assert_eq!(format_value(1500.0, 0.0, 0.0, &fmt(1, 2, false)), "1.5 k");
        assert_eq!(format_value(512.0, 0.0, 0.0, &fmt(0, 1, false)), "512 ");
    }

    #[test]
    fn percentual_uses_min_and_max() {
        assert_eq!(format_value(4.0, 0.0, 16.0, &fmt(0, 0, true)), "25");
        assert_eq!(format_value(3.0, 0.0, 0.0, &fmt(0, 0, true)), "0", "sem intervalo, 0%");
        assert_eq!(format_value(2.0, 0.0, 8.0, &fmt(1, 0, true)), "25.0");
    }

    #[test]
    fn scale_divides_before_formatting() {
        let f = NumberFormat { decimals: 2, autoscale: 0, percentual: false, scale: 1000.0 };
        assert_eq!(format_value(1234.0, 0.0, 0.0, &f), "1.23");
    }
}
