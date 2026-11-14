//! O que a skin pede para desenhar, já com posição final em pixels da janela da skin.

use std::path::PathBuf;

use crate::Color;

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: f32,
    pub height: f32,
    /// Fundo da janela da skin (BackgroundMode=2 + SolidColor).
    pub background: Option<Color>,
    /// Imagem de fundo (Background=, BackgroundMode 0, 3 ou 4).
    pub background_image: Option<BackgroundImage>,
    pub items: Vec<Item>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundFit {
    /// BackgroundMode=0: tamanho natural.
    Natural,
    /// BackgroundMode=3: esticada até a janela; as margens não esticam (9 partes).
    Scaled,
    /// BackgroundMode=4: repetida lado a lado.
    Tiled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundImage {
    pub path: PathBuf,
    pub fit: BackgroundFit,
    /// BackgroundMargins: esquerda, cima, direita, baixo.
    pub margins: (f32, f32, f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Text(TextItem),
    Image(ImageItem),
    Shape(ShapeItem),
    Line(LineItem),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub face: String,
    /// Tamanho em pixels (o Rainmeter usa pontos: FontSize x 4/3).
    pub size_px: f32,
    pub bold: bool,
    pub italic: bool,
    /// InlineSetting=CharacterSpacing em pixels: antes, depois e avanço mínimo de cada caractere.
    pub spacing: Option<(f32, f32, f32)>,
    /// Pasta `@Resources\Fonts` da skin, de onde vêm as fontes próprias dela.
    pub fonts_dir: Option<PathBuf>,
    /// `AccurateText=0` (o padrão): o Rainmeter imita a medida do GDI, com margem de
    /// tamanho/6 dos dois lados e um espaçamento mínimo entre caracteres.
    pub gdi_emulation: bool,
}

/// Alinhamento dentro do retângulo do medidor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Shadow,
    Border,
}

/// Texto desenhado dentro do retângulo do medidor, com o alinhamento feito pelo DirectWrite,
/// como no Rainmeter.
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    pub text: String,
    pub style: TextStyle,
    pub color: Color,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub h_align: Align,
    pub v_align: Align,
    pub antialias: bool,
    pub effect: Option<(Effect, Color)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageItem {
    pub path: PathBuf,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// 0 a 255.
    pub alpha: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    Rectangle { radius: f32 },
    Ellipse,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapeItem {
    pub kind: ShapeKind,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub fill: Option<Color>,
    pub stroke: Option<(Color, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineItem {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub color: Color,
    pub width: f32,
}
