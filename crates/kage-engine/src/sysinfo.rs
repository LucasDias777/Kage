//! Valores do sistema para as skins, lidos do Windows: CPU, memória, disco, rede e a música
//! tocando (em `now_playing`).
//!
//! CPU e rede são diferenças entre duas leituras. Várias medidas pedindo o mesmo valor no mesmo
//! instante receberiam um intervalo de quase zero; por isso a leitura vale por 250 ms.

use std::cell::Cell;
use std::time::{Duration, Instant};

use kage_skins::{NowPlaying, SystemProbe};

use crate::now_playing::MediaSource;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;
use windows_core::HSTRING;

const REUSE: Duration = Duration::from_millis(250);
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;

#[derive(Default)]
pub struct WinSystem {
    /// Última leitura da CPU: (quando, ocioso, total) e o percentual calculado.
    cpu_sample: Cell<Option<(Instant, u64, u64)>>,
    cpu_value: Cell<f64>,
    /// Última leitura da rede: (quando, bytes recebidos, bytes enviados) e a taxa calculada.
    net_sample: Cell<Option<(Instant, u64, u64)>>,
    net_value: Cell<(f64, f64)>,
    /// A música tocando no Windows (plugin NowPlaying).
    media: MediaSource,
}

impl WinSystem {
    /// Primeira leitura de CPU e rede, para a próxima já ter com o que comparar.
    pub fn prime(&self) {
        self.cpu_percent();
        self.network();
    }

    pub fn media(&self) -> &MediaSource {
        &self.media
    }
}

fn filetime(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

impl SystemProbe for WinSystem {
    fn now_playing(&self) -> Option<NowPlaying> {
        self.media.snapshot()
    }

    fn cpu_percent(&self) -> f64 {
        let now = Instant::now();
        if let Some((at, _, _)) = self.cpu_sample.get()
            && now.duration_since(at) < REUSE
        {
            return self.cpu_value.get();
        }
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_err() {
            return self.cpu_value.get();
        }
        // O tempo de kernel já inclui o ocioso.
        let (idle, total) = (filetime(idle), filetime(kernel) + filetime(user));
        if let Some((_, prev_idle, prev_total)) = self.cpu_sample.get()
            && total > prev_total
        {
            let busy = 1.0 - (idle - prev_idle) as f64 / (total - prev_total) as f64;
            self.cpu_value.set((busy * 100.0).clamp(0.0, 100.0));
        }
        self.cpu_sample.set(Some((now, idle, total)));
        self.cpu_value.get()
    }

    fn memory(&self) -> (f64, f64) {
        let status = memory_status();
        let total = status.ullTotalPhys as f64;
        (total - status.ullAvailPhys as f64, total)
    }

    fn swap(&self) -> (f64, f64) {
        let status = memory_status();
        let total = status.ullTotalPageFile as f64;
        (total - status.ullAvailPageFile as f64, total)
    }

    fn disk(&self, drive: &str) -> Option<(f64, f64)> {
        let root = HSTRING::from(format!("{}\\", drive.trim_end_matches('\\')));
        let (mut total, mut free) = (0u64, 0u64);
        unsafe { GetDiskFreeSpaceExW(&root, None, Some(&mut total), Some(&mut free)) }.ok()?;
        Some((free as f64, total as f64))
    }

    fn network(&self) -> (f64, f64) {
        let now = Instant::now();
        if let Some((at, _, _)) = self.net_sample.get()
            && now.duration_since(at) < REUSE
        {
            return self.net_value.get();
        }
        let Some((received, sent)) = interface_octets() else { return self.net_value.get() };
        if let Some((at, prev_received, prev_sent)) = self.net_sample.get() {
            let seconds = now.duration_since(at).as_secs_f64().max(0.001);
            self.net_value.set((
                received.saturating_sub(prev_received) as f64 / seconds,
                sent.saturating_sub(prev_sent) as f64 / seconds,
            ));
        }
        self.net_sample.set(Some((now, received, sent)));
        self.net_value.get()
    }
}

fn memory_status() -> MEMORYSTATUSEX {
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    let _ = unsafe { GlobalMemoryStatusEx(&mut status) };
    status
}

/// Bytes recebidos e enviados somando as placas físicas (sem loopback nem filtros virtuais).
fn interface_octets() -> Option<(u64, u64)> {
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return None;
        }
        let count = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts(std::ptr::addr_of!((*table).Table) as *const MIB_IF_ROW2, count);
        let (mut received, mut sent) = (0u64, 0u64);
        for row in rows {
            let flags = row.InterfaceAndOperStatusFlags._bitfield;
            let hardware = flags & 0b01 != 0;
            let filter = flags & 0b10 != 0;
            if hardware && !filter && row.Type != IF_TYPE_SOFTWARE_LOOPBACK {
                received += row.InOctets;
                sent += row.OutOctets;
            }
        }
        FreeMibTable(table as *const _);
        Some((received, sent))
    }
}
