//! Skins do Rainmeter no motor: mede e desenha o `Frame` do kage-skins com DirectWrite e Direct2D.
//!
//! Cada skin vira uma imagem com fundo transparente, refeita só quando o que ela mostra muda
//! (no relógio, uma vez por minuto), e vai para a janela dela (ver `skinhost`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kage_skins::{
    Align, BackgroundFit, BackgroundImage, Color, DateTime, Effect, Frame, Item, ShapeKind, Skin,
    TextMeasurer, TextStyle,
};
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_BITMAP_OPTIONS_GDI_COMPATIBLE,
    D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_ELLIPSE,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_MAP_OPTIONS_READ, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_ALIASED, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1Bitmap1, ID2D1Image,
    ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_FAR,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory,
    IDWriteFactory5, IDWriteFontCollection, IDWriteFontCollection1, IDWriteTextFormat,
    IDWriteTextLayout, IDWriteTextLayout1,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICBitmapFrameEncode,
    WICBitmapEncoderNoCache, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows_core::{BOOL, HSTRING, Interface, Result, w};
use windows_numerics::Vector2;

use crate::gpu::Gpu;
use crate::image::StillImage;

/// DirectWrite com as fontes de cada skin (`@Resources\Fonts`) carregadas sem instalar no Windows.
pub struct TextEngine {
    factory: IDWriteFactory5,
    collections: RefCell<HashMap<PathBuf, Option<IDWriteFontCollection1>>>,
}

impl TextEngine {
    pub fn new() -> Result<Self> {
        Ok(Self { factory: unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? }, collections: RefCell::default() })
    }

    fn collection(&self, dir: &Path) -> Option<IDWriteFontCollection1> {
        if let Some(cached) = self.collections.borrow().get(dir) {
            return cached.clone();
        }
        let loaded = self.load_collection(dir).ok().flatten();
        self.collections.borrow_mut().insert(dir.to_path_buf(), loaded.clone());
        loaded
    }

    fn load_collection(&self, dir: &Path) -> Result<Option<IDWriteFontCollection1>> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Ok(None) };
        unsafe {
            let builder = self.factory.CreateFontSetBuilder()?;
            let mut any = false;
            for path in entries.flatten().map(|e| e.path()) {
                let is_font = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"));
                if is_font {
                    let file = self.factory.CreateFontFileReference(&HSTRING::from(path.as_os_str()), None)?;
                    builder.AddFontFile(&file)?;
                    any = true;
                }
            }
            if !any {
                return Ok(None);
            }
            let set = builder.CreateFontSet()?;
            Ok(Some(self.factory.CreateFontCollectionFromFontSet(&set)?))
        }
    }

    fn format(&self, style: &TextStyle) -> Result<IDWriteTextFormat> {
        let family = HSTRING::from(style.face.as_str());
        // Fonte da skin, se ela tiver; senão, as fontes instaladas no Windows.
        let private = style
            .fonts_dir
            .as_deref()
            .and_then(|d| self.collection(d))
            .filter(|c| has_family(c, &family));
        let weight = if style.bold { DWRITE_FONT_WEIGHT_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
        let slant = if style.italic { DWRITE_FONT_STYLE_ITALIC } else { DWRITE_FONT_STYLE_NORMAL };
        unsafe {
            let collection: Option<IDWriteFontCollection> = private.map(|c| c.cast()).transpose()?;
            self.factory.CreateTextFormat(
                &family,
                collection.as_ref(),
                weight,
                slant,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size_px.max(1.0),
                w!("pt-br"),
            )
        }
    }

    /// Layout do texto como o Rainmeter 4.5 monta (TextFormatD2D.cpp). Com `AccurateText=0`
    /// entra o espaçamento da imitação do GDI; na medição ele vem depois e vence o
    /// InlineSetting, no desenho é o contrário.
    fn layout(&self, text: &str, style: &TextStyle, measuring: bool, max: (f32, f32)) -> Result<IDWriteTextLayout> {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let format = self.format(style)?;
        unsafe {
            let layout = self.factory.CreateTextLayout(&utf16, &format, max.0.max(1.0), max.1.max(1.0))?;
            layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let layout1: IDWriteTextLayout1 = layout.cast()?;
            let all = DWRITE_TEXT_RANGE { startPosition: 0, length: utf16.len() as u32 };
            if style.gdi_emulation {
                let em = style.size_px / 6.0 / 24.0;
                layout1.SetCharacterSpacing(em, em, 0.0, all)?;
            }
            if let Some((leading, trailing, minimum)) = style.spacing
                && !(measuring && style.gdi_emulation)
            {
                layout1.SetCharacterSpacing(leading, trailing, minimum, all)?;
            }
            Ok(layout)
        }
    }
}

/// Deslocamento horizontal do desenho na imitação do GDI (Canvas.cpp): à esquerda entra
/// tamanho/6, à direita sai tamanho/6, centralizado fica onde está.
fn gdi_offset(style: &TextStyle, align: Align) -> f32 {
    if !style.gdi_emulation {
        return 0.0;
    }
    match align {
        Align::Start => style.size_px / 6.0,
        Align::End => -style.size_px / 6.0,
        Align::Center => 0.0,
    }
}

fn has_family(collection: &IDWriteFontCollection1, family: &HSTRING) -> bool {
    let (mut index, mut exists) = (0u32, BOOL(0));
    unsafe { collection.FindFamilyName(family, &mut index, &mut exists).is_ok() && exists.as_bool() }
}

/// Largura do medidor como o Rainmeter calcula: `width` (sem o espaço do fim); com
/// `AccurateText=1` arredonda para cima, na imitação do GDI soma tamanho/6 de cada lado.
fn meter_size(layout: &IDWriteTextLayout, style: &TextStyle) -> (f32, f32) {
    let mut m = DWRITE_TEXT_METRICS::default();
    if unsafe { layout.GetMetrics(&mut m) }.is_err() {
        return (0.0, 0.0);
    }
    let width = if style.gdi_emulation { m.width + 2.0 * style.size_px / 6.0 } else { m.width.ceil() };
    (width, m.height)
}

/// O que o kage-skins precisa para posicionar texto e imagem.
pub struct Measurer<'a> {
    pub text: &'a TextEngine,
    pub gpu: &'a Gpu,
}

impl TextMeasurer for Measurer<'_> {
    fn measure_text(&self, text: &str, style: &TextStyle) -> (f32, f32) {
        self.text
            .layout(text, style, true, (100_000.0, 100_000.0))
            .map(|l| meter_size(&l, style))
            .unwrap_or((0.0, 0.0))
    }

    fn image_size(&self, path: &Path) -> Option<(f32, f32)> {
        unsafe {
            let decoder = self
                .gpu
                .wic
                .CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
                .ok()?;
            let (mut w, mut h) = (0u32, 0u32);
            decoder.GetFrame(0).ok()?.GetSize(&mut w, &mut h).ok()?;
            Some((w as f32, h as f32))
        }
    }
}

/// Imagens das skins já na GPU, por arquivo.
#[derive(Default)]
pub struct ImageCache {
    bitmaps: HashMap<PathBuf, Option<ID2D1Bitmap1>>,
}

impl ImageCache {
    fn get(&mut self, gpu: &Gpu, path: &Path) -> Option<ID2D1Bitmap1> {
        self.bitmaps
            .entry(path.to_path_buf())
            .or_insert_with(|| StillImage::open(gpu, path).ok().map(|i| i.bitmap))
            .clone()
    }
}

fn color_f(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r as f32 / 255.0, g: c.g as f32 / 255.0, b: c.b as f32 / 255.0, a: c.a as f32 / 255.0 }
}

fn bitmap_props(options: windows::Win32::Graphics::Direct2D::D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        ..Default::default()
    }
}

/// Imagem do tamanho da janela da skin, que o GDI também lê (para o `UpdateLayeredWindow`).
pub fn layered_bitmap(gpu: &Gpu, width: u32, height: u32) -> Result<ID2D1Bitmap1> {
    let size = D2D_SIZE_U { width: width.max(1), height: height.max(1) };
    let options = D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_GDI_COMPATIBLE;
    unsafe { gpu.d2d.CreateBitmap(size, None, 0, &bitmap_props(options)) }
}

/// Desenha o frame com fundo transparente e, antes de fechar o desenho, chama `present` (que
/// pega o DC da imagem e atualiza a janela da skin).
pub fn render_layered(
    text: &TextEngine,
    gpu: &Gpu,
    target: &ID2D1Bitmap1,
    frame: &Frame,
    images: &mut ImageCache,
    present: &dyn Fn() -> Result<()>,
) -> Result<()> {
    paint(text, gpu, target, frame, images, (0.0, 0.0), None, Some(present))
}

#[allow(clippy::too_many_arguments)]
fn paint(
    text: &TextEngine,
    gpu: &Gpu,
    target: &ID2D1Bitmap1,
    frame: &Frame,
    images: &mut ImageCache,
    offset: (f32, f32),
    backdrop: Option<Color>,
    present: Option<&dyn Fn() -> Result<()>>,
) -> Result<()> {
    let dc = &gpu.d2d;
    let (ox, oy) = offset;
    unsafe {
        dc.SetTarget(target);
        dc.BeginDraw();
        dc.Clear(Some(&color_f(backdrop.unwrap_or(Color { r: 0, g: 0, b: 0, a: 0 }))));
        if let Some(bg) = frame.background {
            let brush = dc.CreateSolidColorBrush(&color_f(bg), None)?;
            dc.FillRectangle(&D2D_RECT_F { left: ox, top: oy, right: ox + frame.width, bottom: oy + frame.height }, &brush);
        }
        if let Some(bg) = &frame.background_image
            && let Some(bitmap) = images.get(gpu, &bg.path)
        {
            draw_background(gpu, &bitmap, bg, (ox, oy), (frame.width, frame.height));
        }
        for item in &frame.items {
            match item {
                Item::Text(t) => {
                    // O texto fica dentro do retângulo do medidor e o DirectWrite alinha.
                    let Ok(layout) = text.layout(&t.text, &t.style, false, (t.width, t.height)) else { continue };
                    let _ = layout.SetTextAlignment(match t.h_align {
                        Align::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
                        Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                        Align::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
                    });
                    let _ = layout.SetParagraphAlignment(match t.v_align {
                        Align::Start => DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
                        Align::Center => DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                        Align::End => DWRITE_PARAGRAPH_ALIGNMENT_FAR,
                    });
                    dc.SetTextAntialiasMode(if t.antialias {
                        D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE
                    } else {
                        D2D1_TEXT_ANTIALIAS_MODE_ALIASED
                    });
                    let left = t.left + gdi_offset(&t.style, t.h_align);
                    let at = |dx: f32, dy: f32| Vector2 { X: ox + left + dx, Y: oy + t.top + dy };
                    if let Some((effect, effect_color)) = t.effect {
                        let brush = dc.CreateSolidColorBrush(&color_f(effect_color), None)?;
                        let offsets: &[(f32, f32)] = match effect {
                            Effect::Shadow => &[(1.0, 1.0)],
                            Effect::Border => &[(-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (0.0, 1.0), (1.0, 1.0)],
                        };
                        for (dx, dy) in offsets {
                            dc.DrawTextLayout(at(*dx, *dy), &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
                        }
                    }
                    let brush = dc.CreateSolidColorBrush(&color_f(t.color), None)?;
                    dc.DrawTextLayout(at(0.0, 0.0), &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
                }
                Item::Image(img) => {
                    let Some(bitmap) = images.get(gpu, &img.path) else { continue };
                    let dest = D2D_RECT_F { left: ox + img.x, top: oy + img.y, right: ox + img.x + img.w, bottom: oy + img.y + img.h };
                    dc.DrawBitmap(&bitmap, Some(&dest), img.alpha as f32 / 255.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
                }
                Item::Shape(s) => {
                    let rect = D2D_RECT_F { left: ox + s.x, top: oy + s.y, right: ox + s.x + s.w, bottom: oy + s.y + s.h };
                    match s.kind {
                        ShapeKind::Rectangle { radius } => {
                            let rounded = D2D1_ROUNDED_RECT { rect, radiusX: radius, radiusY: radius };
                            if let Some(fill) = s.fill {
                                let brush = dc.CreateSolidColorBrush(&color_f(fill), None)?;
                                dc.FillRoundedRectangle(&rounded, &brush);
                            }
                            if let Some((color, width)) = s.stroke {
                                let brush = dc.CreateSolidColorBrush(&color_f(color), None)?;
                                dc.DrawRoundedRectangle(&rounded, &brush, width, None::<&ID2D1StrokeStyle>);
                            }
                        }
                        ShapeKind::Ellipse => {
                            let ellipse = D2D1_ELLIPSE {
                                point: Vector2 { X: (rect.left + rect.right) / 2.0, Y: (rect.top + rect.bottom) / 2.0 },
                                radiusX: s.w / 2.0,
                                radiusY: s.h / 2.0,
                            };
                            if let Some(fill) = s.fill {
                                let brush = dc.CreateSolidColorBrush(&color_f(fill), None)?;
                                dc.FillEllipse(&ellipse, &brush);
                            }
                            if let Some((color, width)) = s.stroke {
                                let brush = dc.CreateSolidColorBrush(&color_f(color), None)?;
                                dc.DrawEllipse(&ellipse, &brush, width, None::<&ID2D1StrokeStyle>);
                            }
                        }
                    }
                }
                Item::Line(l) => {
                    let brush = dc.CreateSolidColorBrush(&color_f(l.color), None)?;
                    dc.DrawLine(
                        Vector2 { X: ox + l.from.0, Y: oy + l.from.1 },
                        Vector2 { X: ox + l.to.0, Y: oy + l.to.1 },
                        &brush,
                        l.width,
                        None::<&ID2D1StrokeStyle>,
                    );
                }
            }
        }
        // O DC do GDI só existe entre o BeginDraw e o EndDraw.
        let presented = present.map_or(Ok(()), |p| p());
        let done = dc.EndDraw(None, None);
        dc.SetTarget(None::<&ID2D1Image>);
        presented.and(done)
    }
}

/// Imagem de fundo da skin: tamanho natural, ladrilhada ou esticada em 9 partes (os cantos
/// ficam do tamanho original e só o miolo estica, como o BackgroundMode=3 do Rainmeter).
fn draw_background(gpu: &Gpu, bitmap: &ID2D1Bitmap1, bg: &BackgroundImage, origin: (f32, f32), size: (f32, f32)) {
    let dc = &gpu.d2d;
    let (ox, oy) = origin;
    let image = unsafe { bitmap.GetSize() };
    let (iw, ih) = (image.width, image.height);
    if iw <= 0.0 || ih <= 0.0 {
        return;
    }
    let draw = |src: D2D_RECT_F, dst: D2D_RECT_F| {
        if src.right > src.left && src.bottom > src.top && dst.right > dst.left && dst.bottom > dst.top {
            unsafe { dc.DrawBitmap(bitmap, Some(&dst), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, Some(&src), None) };
        }
    };
    let rect = |l: f32, t: f32, r: f32, b: f32| D2D_RECT_F { left: l, top: t, right: r, bottom: b };
    match bg.fit {
        BackgroundFit::Natural => draw(rect(0.0, 0.0, iw, ih), rect(ox, oy, ox + iw, oy + ih)),
        BackgroundFit::Tiled => {
            let mut y = 0.0;
            while y < size.1 {
                let mut x = 0.0;
                while x < size.0 {
                    draw(rect(0.0, 0.0, iw, ih), rect(ox + x, oy + y, ox + x + iw, oy + y + ih));
                    x += iw;
                }
                y += ih;
            }
        }
        BackgroundFit::Scaled => {
            let (l, t, r, b) = bg.margins;
            let src_x = [0.0, l, iw - r, iw];
            let src_y = [0.0, t, ih - b, ih];
            let dst_x = [0.0, l, size.0 - r, size.0];
            let dst_y = [0.0, t, size.1 - b, size.1];
            for row in 0..3 {
                for col in 0..3 {
                    draw(
                        rect(src_x[col], src_y[row], src_x[col + 1], src_y[row + 1]),
                        rect(ox + dst_x[col], oy + dst_y[row], ox + dst_x[col + 1], oy + dst_y[row + 1]),
                    );
                }
            }
        }
    }
}

/// Onde a skin fica na tela virtual.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// Canto no X e Y dados, como no Rainmeter.
    Free { x: f32, y: f32 },
    /// Centralizada na horizontal dentro de `area` (um monitor), com o centro da skin em
    /// `height` (0 a 1) da altura do monitor.
    Centered { area: RECT, height: f32 },
}

/// Canto da skin de tamanho `size` conforme a posição pedida.
pub fn place_skin(placement: Placement, size: (f32, f32)) -> (f32, f32) {
    match placement {
        Placement::Free { x, y } => (x, y),
        Placement::Centered { area, height } => {
            let (aw, ah) = ((area.right - area.left) as f32, (area.bottom - area.top) as f32);
            let x = area.left as f32 + (aw - size.0) / 2.0;
            let y = area.top as f32 + ah * height - size.1 / 2.0;
            // A skin inteira dentro do monitor.
            let y = y.clamp(area.top as f32, (area.bottom as f32 - size.1).max(area.top as f32));
            (x.round(), y.round())
        }
    }
}

pub fn local_datetime() -> DateTime {
    let t = unsafe { GetLocalTime() };
    let (year, month, day) = (t.wYear as i32, t.wMonth as u32, t.wDay as u32);
    const BEFORE: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let day_of_year = BEFORE[(month.clamp(1, 12) - 1) as usize] + day + u32::from(leap && month > 2);
    DateTime {
        year,
        month,
        day,
        weekday: t.wDayOfWeek as u32,
        hour: t.wHour as u32,
        minute: t.wMinute as u32,
        second: t.wSecond as u32,
        day_of_year,
    }
}

/// Modo de conferência: desenha a skin sobre um fundo escuro e salva em PNG.
pub fn render_to_png(gpu: &Gpu, skin: &mut Skin, out: &Path) -> Result<(u32, u32)> {
    let text = TextEngine::new()?;
    let mut images = ImageCache::default();
    // CPU e rede precisam de duas leituras com um intervalo entre elas.
    let system = crate::sysinfo::WinSystem::default();
    system.prime();
    std::thread::sleep(Duration::from_millis(500));
    let frame = skin.update(&local_datetime(), &Measurer { text: &text, gpu }, &system);
    const MARGIN: f32 = 24.0;
    let size = D2D_SIZE_U { width: (frame.width + MARGIN * 2.0) as u32, height: (frame.height + MARGIN * 2.0) as u32 };
    unsafe {
        let target = gpu.d2d.CreateBitmap(size, None, 0, &bitmap_props(D2D1_BITMAP_OPTIONS_TARGET))?;
        let backdrop = Color { r: 28, g: 30, b: 38, a: 255 };
        paint(&text, gpu, &target, &frame, &mut images, (MARGIN, MARGIN), Some(backdrop), None)?;

        let readable = gpu.d2d.CreateBitmap(
            size,
            None,
            0,
            &bitmap_props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
        )?;
        readable.CopyFromBitmap(None, &target, None)?;
        let mapped = readable.Map(D2D1_MAP_OPTIONS_READ)?;
        let stride = size.width as usize * 4;
        let mut pixels = vec![0u8; stride * size.height as usize];
        for row in 0..size.height as usize {
            let src = std::slice::from_raw_parts(mapped.bits.add(row * mapped.pitch as usize), stride);
            pixels[row * stride..(row + 1) * stride].copy_from_slice(src);
        }
        readable.Unmap()?;

        let stream = gpu.wic.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(out.as_os_str()), GENERIC_WRITE.0)?;
        let encoder = gpu.wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame_encode: Option<IWICBitmapFrameEncode> = None;
        let mut options: Option<IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame_encode, &mut options)?;
        let frame_encode = frame_encode.expect("CreateNewFrame sem quadro");
        frame_encode.Initialize(options.as_ref())?;
        frame_encode.SetSize(size.width, size.height)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame_encode.SetPixelFormat(&mut format)?;
        let source = gpu.wic.CreateBitmapFromMemory(size.width, size.height, &GUID_WICPixelFormat32bppBGRA, stride as u32, &pixels)?;
        frame_encode.WriteSource(&source, std::ptr::null())?;
        frame_encode.Commit()?;
        encoder.Commit()?;
    }
    for warning in &skin.warnings {
        eprintln!("Aviso: {warning}");
    }
    Ok((size.width, size.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: RECT = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };

    #[test]
    fn centered_skin_is_horizontally_centered_at_the_height_asked() {
        // A Mond: ~600 x 120 px no monitor principal.
        assert_eq!(place_skin(Placement::Centered { area: MAIN, height: 0.5 }, (600.0, 120.0)), (980.0, 660.0));
        assert_eq!(place_skin(Placement::Centered { area: MAIN, height: 0.25 }, (600.0, 120.0)), (980.0, 300.0));
    }

    #[test]
    fn centered_skin_stays_inside_its_monitor() {
        let left = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
        let (x, y) = place_skin(Placement::Centered { area: left, height: 0.0 }, (400.0, 200.0));
        assert_eq!((x, y), (-1160.0, 0.0));
        let (_, y) = place_skin(Placement::Centered { area: left, height: 1.0 }, (400.0, 200.0));
        assert_eq!(y, 880.0);
    }

    #[test]
    fn free_skin_keeps_its_corner() {
        assert_eq!(place_skin(Placement::Free { x: 984.0, y: 300.0 }, (600.0, 120.0)), (984.0, 300.0));
    }
}
