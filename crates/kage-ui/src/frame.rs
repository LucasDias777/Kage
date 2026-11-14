//! Moldura própria da janela. Sem a barra de título do Windows, o Kage desenha a dele na mesma
//! cor da janela; aqui ficam o arrastar, os dois cliques que maximizam, as bordas de
//! redimensionar e a sombra, que a janela sem moldura perde.

use std::cell::Cell;
use std::time::{Duration, Instant};

use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use slint::winit_030::winit::window::ResizeDirection;

/// Intervalo padrão do Windows para dois cliques.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

thread_local! {
    static LAST_TITLE_PRESS: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Sombra em volta da janela, como a das janelas com moldura.
pub fn add_shadow(window: &slint::Window) {
    window.with_winit_window(|w| w.set_undecorated_shadow(true));
}

/// Aperto na barra de título: arrasta a janela (o Windows cuida de encaixar nas bordas da tela)
/// ou, se for o segundo aperto seguido, maximiza e restaura.
pub fn press_title(window: &slint::Window) {
    let now = Instant::now();
    if LAST_TITLE_PRESS.get().is_some_and(|before| now.duration_since(before) < DOUBLE_CLICK) {
        LAST_TITLE_PRESS.set(None);
        toggle_maximize(window);
        return;
    }
    LAST_TITLE_PRESS.set(Some(now));
    window.with_winit_window(|w| {
        let _ = w.drag_window();
    });
}

pub fn toggle_maximize(window: &slint::Window) {
    window.set_maximized(!window.is_maximized());
}

/// Borda apertada, na ordem da janela: 0 esquerda, 1 direita, 2 cima, 3 baixo, 4 canto superior
/// esquerdo, 5 superior direito, 6 inferior esquerdo, 7 inferior direito.
pub fn start_resize(window: &slint::Window, edge: i32) {
    let direction = match edge {
        0 => ResizeDirection::West,
        1 => ResizeDirection::East,
        2 => ResizeDirection::North,
        3 => ResizeDirection::South,
        4 => ResizeDirection::NorthWest,
        5 => ResizeDirection::NorthEast,
        6 => ResizeDirection::SouthWest,
        _ => ResizeDirection::SouthEast,
    };
    window.with_winit_window(|w| {
        let _ = w.drag_resize_window(direction);
    });
}
