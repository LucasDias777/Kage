//! Uma janela do Kage por vez. A janela se marca (`SetPropW` com [`kage_core::UI_WINDOW_PROP`])
//! e recebe, por uma mensagem registrada, os pedidos do menu da bandeja e de uma segunda abertura
//! do `kage.exe`: aparecer, abrir Atualizações, Customizar ou Novidades.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use kage_core::UiCommand;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetPropW, IsIconic, PostMessageW, RegisterWindowMessageW, SW_RESTORE, SetForegroundWindow,
    SetPropW, ShowWindow,
};
use windows_core::{BOOL, HSTRING};

type OnCommand = Box<dyn Fn(UiCommand)>;

thread_local! {
    static COMMAND_MESSAGE: Cell<u32> = const { Cell::new(0) };
    static ON_COMMAND: RefCell<Option<OnCommand>> = RefCell::new(None);
}

fn command_message() -> u32 {
    unsafe { RegisterWindowMessageW(&HSTRING::from(kage_core::UI_COMMAND_MESSAGE)) }
}

/// A tela pedida na linha de comando (`--abrir atualizacoes`); sem pedido, só abrir.
pub fn command_from_args() -> UiCommand {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--abrir")
        .and_then(|i| args.get(i + 1))
        .and_then(|name| UiCommand::from_arg(name))
        .unwrap_or(UiCommand::Show)
}

/// A janela do Kage que já está aberta, se houver.
fn existing_window() -> Option<HWND> {
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
        let found = unsafe { &mut *(data.0 as *mut Option<HWND>) };
        if !unsafe { GetPropW(hwnd, &HSTRING::from(kage_core::UI_WINDOW_PROP)) }.is_invalid() {
            *found = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut found: Option<HWND> = None;
    unsafe {
        let _ = EnumWindows(Some(find), LPARAM(&mut found as *mut Option<HWND> as isize));
    }
    found
}

/// Se já há uma janela aberta, traz ela para a frente com o pedido e devolve verdadeiro: esta
/// cópia pode sair sem abrir outra.
pub fn hand_over(command: UiCommand) -> bool {
    let Some(hwnd) = existing_window() else { return false };
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
        if command != UiCommand::Show {
            let _ = PostMessageW(Some(hwnd), command_message(), WPARAM(command as usize), LPARAM(0));
        }
    }
    true
}

/// Marca a janela e passa a atender os pedidos. Chamar depois que a janela do winit existe.
pub fn install(window: &slint::Window, on_command: impl Fn(UiCommand) + 'static) {
    let Some(hwnd) = window.with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
        _ => None,
    }).flatten() else {
        return;
    };
    COMMAND_MESSAGE.set(command_message());
    ON_COMMAND.with(|c| *c.borrow_mut() = Some(Box::new(on_command)));
    unsafe {
        let _ = SetPropW(hwnd, &HSTRING::from(kage_core::UI_WINDOW_PROP), Some(HANDLE(std::ptr::dangling_mut())));
        let _ = SetWindowSubclass(hwnd, Some(subclass), 1, 0);
    }
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> LRESULT {
    let wanted = COMMAND_MESSAGE.get();
    if wanted != 0 && msg == wanted {
        if let Some(command) = UiCommand::from_code(wparam.0) {
            // Fora do tratamento da mensagem, para a tela não ser mexida no meio de um evento.
            slint::Timer::single_shot(Duration::ZERO, move || {
                ON_COMMAND.with(|c| {
                    if let Some(run) = c.borrow().as_ref() {
                        run(command);
                    }
                });
            });
        }
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
