//! Skins do Rainmeter no Kage.
//!
//! Este crate não desenha nada e não depende do Windows: lê o INI da skin (com `@include`,
//! variáveis e fórmulas), atualiza as medidas e monta um [`Frame`], a lista de textos, imagens e
//! formas com posição e estilo. O motor do Kage desenha esse frame na GPU.

mod color;
mod draw;
pub mod formula;
pub mod ini;
pub mod rainmeter;
mod skin;
pub mod strftime;
pub mod system;
mod vars;

pub use color::Color;
pub use draw::{
    Align, BackgroundFit, BackgroundImage, Effect, Frame, ImageItem, Item, LineItem, ShapeItem, ShapeKind, TextItem,
    TextStyle,
};
pub use skin::{Action, MouseEvent, Skin, TextMeasurer, parse_actions};
pub use strftime::DateTime;
pub use system::{NoSystem, NowPlaying, SystemProbe};
pub use vars::Variables;
