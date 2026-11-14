//! Conversa com o motor (`kage-engine.exe`, na mesma pasta desta janela).

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId, PostMessageW};
use windows_core::HSTRING;

/// O motor roda sem janela de console.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_default().with_file_name("kage-engine.exe")
}

fn window() -> Option<HWND> {
    let class = HSTRING::from(kage_core::ENGINE_WINDOW_CLASS);
    unsafe { FindWindowW(&class, None) }.ok().filter(|h| !h.is_invalid())
}

pub fn is_running() -> bool {
    window().is_some()
}

pub fn start() -> Result<(), String> {
    Command::new(exe_path())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("não abriu o motor ({}): {e}", exe_path().display()))
}

/// Avisa o motor que a configuração mudou; se ele não estiver aberto, abre.
pub fn reload() -> Result<(), String> {
    match window() {
        Some(hwnd) => unsafe { PostMessageW(Some(hwnd), kage_core::MSG_RELOAD, WPARAM(0), LPARAM(0)) }
            .map_err(|e| e.to_string()),
        None => start(),
    }
}

pub fn stop() {
    if let Some(hwnd) = window() {
        let _ = unsafe { PostMessageW(Some(hwnd), kage_core::MSG_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

/// RAM privada do motor em MB, para o rodapé.
pub fn memory_mb() -> Option<u64> {
    let hwnd = window()?;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
        let ok = K32GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        );
        let _ = CloseHandle(process);
        ok.as_bool().then_some(counters.PrivateUsage as u64 / (1024 * 1024))
    }
}

fn run(args: &[&std::ffi::OsStr]) -> bool {
    Command::new(exe_path()).args(args).creation_flags(CREATE_NO_WINDOW).status().is_ok_and(|s| s.success())
}

/// Miniatura pelo próprio motor (ele já tem o decodificador de vídeo).
pub fn make_thumbnail(input: &Path, output: &Path) -> bool {
    run(&["--thumbnail".as_ref(), input.as_os_str(), output.as_os_str()])
}

/// Imagem minúscula que a janela aumenta e fica desfocada (fundo "Dinâmico").
pub fn make_blurred(input: &Path, output: &Path) -> bool {
    run(&["--shrink".as_ref(), input.as_os_str(), output.as_os_str(), "48".as_ref()])
}

/// Prévia da skin desenhada pelo motor, num fundo escuro.
pub fn render_skin(ini: &Path, output: &Path) -> bool {
    if let Some(dir) = output.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    run(&["--render-skin".as_ref(), ini.as_os_str(), output.as_os_str()])
}
