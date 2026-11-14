//! Imagem parada e GIF animado, decodificados pelo WIC e desenhados pelo Direct2D.

use std::path::Path;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, ID2D1Bitmap1,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    GUID_WICPixelFormat32bppPBGRA, IWICBitmapDecoder, IWICBitmapSource, IWICImagingFactory,
    IWICMetadataQueryReader, IWICPalette, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Variant::{VT_UI1, VT_UI2};
use windows_core::{HSTRING, Interface, PCWSTR, Result, w};

use crate::gpu::Gpu;

/// Converte qualquer quadro do WIC para BGRA pré-multiplicado, o formato do Direct2D.
fn to_pbgra(factory: &IWICImagingFactory, source: &IWICBitmapSource) -> Result<IWICBitmapSource> {
    unsafe {
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            source,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None::<&IWICPalette>,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        converter.cast()
    }
}

fn open_decoder(gpu: &Gpu, path: &Path) -> Result<IWICBitmapDecoder> {
    unsafe {
        gpu.wic.CreateDecoderFromFilename(
            &HSTRING::from(path.as_os_str()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
    }
}

pub struct StillImage {
    pub bitmap: ID2D1Bitmap1,
    pub size: (u32, u32),
}

impl StillImage {
    pub fn open(gpu: &Gpu, path: &Path) -> Result<Self> {
        unsafe {
            let frame = open_decoder(gpu, path)?.GetFrame(0)?;
            let pixels = to_pbgra(&gpu.wic, &frame.cast()?)?;
            let (mut width, mut height) = (0u32, 0u32);
            pixels.GetSize(&mut width, &mut height)?;
            // A cópia fica só na GPU; o decodificador é descartado aqui.
            let bitmap = gpu.d2d.CreateBitmapFromWicBitmap(&pixels, None)?;
            Ok(Self { bitmap, size: (width, height) })
        }
    }
}

/// Como o GIF limpa a área do quadro anterior antes de desenhar o próximo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposal {
    Keep,
    Background,
    Previous,
}

impl Disposal {
    fn from_code(code: u16) -> Self {
        match code {
            2 => Self::Background,
            3 => Self::Previous,
            _ => Self::Keep,
        }
    }
}

/// Tempo de um quadro. Navegadores tratam atraso menor que 20 ms como 100 ms; seguimos igual.
fn frame_delay(centiseconds: u16) -> Duration {
    if centiseconds < 2 { Duration::from_millis(100) } else { Duration::from_millis(centiseconds as u64 * 10) }
}

pub struct AnimatedGif {
    decoder: IWICBitmapDecoder,
    factory: IWICImagingFactory,
    pub bitmap: ID2D1Bitmap1,
    pub size: (u32, u32),
    frame_count: u32,
    index: u32,
    canvas: Vec<u8>,
    saved: Option<Vec<u8>>,
    pending_disposal: Disposal,
    pending_rect: (u32, u32, u32, u32),
    next_due: Instant,
    pub playing: bool,
    /// Velocidade escolhida no "Customizar" (1 = normal).
    pub speed: f32,
}

impl AnimatedGif {
    pub fn open(gpu: &Gpu, path: &Path) -> Result<Self> {
        unsafe {
            let decoder = open_decoder(gpu, path)?;
            let frame_count = decoder.GetFrameCount()?.max(1);
            let global = decoder.GetMetadataQueryReader()?;
            let first = decoder.GetFrame(0)?;
            let (mut fw, mut fh) = (0u32, 0u32);
            first.GetSize(&mut fw, &mut fh)?;
            let width = read_u16(&global, w!("/logscrdesc/Width")).map(u32::from).unwrap_or(fw);
            let height = read_u16(&global, w!("/logscrdesc/Height")).map(u32::from).unwrap_or(fh);

            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                ..Default::default()
            };
            let bitmap = gpu.d2d.CreateBitmap(D2D_SIZE_U { width, height }, None, 0, &props)?;

            let mut gif = Self {
                decoder,
                factory: gpu.wic.clone(),
                bitmap,
                size: (width, height),
                frame_count,
                index: 0,
                canvas: vec![0; (width * height * 4) as usize],
                saved: None,
                pending_disposal: Disposal::Keep,
                pending_rect: (0, 0, 0, 0),
                next_due: Instant::now(),
                playing: false,
                speed: 1.0,
            };
            let delay = gif.compose(0)?;
            gif.next_due = Instant::now() + delay;
            Ok(gif)
        }
    }

    /// Avança para o próximo quadro se já deu a hora. Devolve true quando a imagem mudou.
    pub fn advance(&mut self, now: Instant) -> Result<bool> {
        if !self.playing || self.frame_count < 2 || now < self.next_due {
            return Ok(false);
        }
        self.index = (self.index + 1) % self.frame_count;
        let delay = self.compose(self.index)?.div_f32(self.speed.clamp(0.1, 4.0));
        self.next_due += delay;
        // Se ficou muito para trás (PC suspenso, por exemplo), recomeça a contagem de agora.
        if self.next_due < now {
            self.next_due = now + delay;
        }
        Ok(true)
    }

    pub fn next_due(&self) -> Instant {
        self.next_due
    }

    pub fn restart_clock(&mut self, now: Instant) {
        self.next_due = now;
    }

    /// Monta o quadro `index` sobre o canvas e envia para a GPU. Devolve o tempo dele.
    fn compose(&mut self, index: u32) -> Result<Duration> {
        let (width, height) = self.size;
        // Desfaz o quadro anterior conforme o modo de descarte dele.
        match self.pending_disposal {
            Disposal::Background => {
                let (x, y, w, h) = self.pending_rect;
                fill_rect(&mut self.canvas, width, height, x, y, w, h);
            }
            Disposal::Previous => {
                if let Some(saved) = self.saved.take() {
                    self.canvas = saved;
                }
            }
            Disposal::Keep => {}
        }
        if index == 0 {
            self.canvas.fill(0);
        }

        unsafe {
            let frame = self.decoder.GetFrame(index)?;
            let meta = frame.GetMetadataQueryReader()?;
            let x = read_u16(&meta, w!("/imgdesc/Left")).unwrap_or(0) as u32;
            let y = read_u16(&meta, w!("/imgdesc/Top")).unwrap_or(0) as u32;
            let delay = frame_delay(read_u16(&meta, w!("/grctlext/Delay")).unwrap_or(10));
            let disposal = Disposal::from_code(read_u16(&meta, w!("/grctlext/Disposal")).unwrap_or(0));

            let pixels = to_pbgra(&self.factory, &frame.cast()?)?;
            let (mut fw, mut fh) = (0u32, 0u32);
            pixels.GetSize(&mut fw, &mut fh)?;
            let mut buffer = vec![0u8; (fw * fh * 4) as usize];
            pixels.CopyPixels(std::ptr::null(), fw * 4, &mut buffer)?;

            if disposal == Disposal::Previous {
                self.saved = Some(self.canvas.clone());
            }
            blend(&mut self.canvas, width, height, &buffer, fw, fh, x, y);
            self.pending_disposal = disposal;
            self.pending_rect = (x, y, fw, fh);

            self.bitmap.CopyFromMemory(None, self.canvas.as_ptr().cast(), width * 4)?;
            Ok(delay)
        }
    }
}

/// Copia os pixels opacos do quadro para o canvas (no GIF a transparência é tudo ou nada).
#[allow(clippy::too_many_arguments)]
fn blend(canvas: &mut [u8], cw: u32, ch: u32, frame: &[u8], fw: u32, fh: u32, x: u32, y: u32) {
    for row in 0..fh {
        let cy = y + row;
        if cy >= ch {
            break;
        }
        for col in 0..fw {
            let cx = x + col;
            if cx >= cw {
                break;
            }
            let src = ((row * fw + col) * 4) as usize;
            if frame[src + 3] != 0 {
                let dst = ((cy * cw + cx) * 4) as usize;
                canvas[dst..dst + 4].copy_from_slice(&frame[src..src + 4]);
            }
        }
    }
}

fn fill_rect(canvas: &mut [u8], cw: u32, ch: u32, x: u32, y: u32, w: u32, h: u32) {
    for cy in y..(y + h).min(ch) {
        let start = ((cy * cw + x.min(cw)) * 4) as usize;
        let end = ((cy * cw + (x + w).min(cw)) * 4) as usize;
        canvas[start..end].fill(0);
    }
}

fn read_u16(reader: &IWICMetadataQueryReader, name: PCWSTR) -> Option<u16> {
    unsafe {
        let mut value = PROPVARIANT::default();
        reader.GetMetadataByName(name, &mut value).ok()?;
        let inner = &value.Anonymous.Anonymous;
        let result = match inner.vt {
            VT_UI2 => Some(inner.Anonymous.uiVal),
            VT_UI1 => Some(inner.Anonymous.bVal as u16),
            _ => None,
        };
        let _ = PropVariantClear(&mut value);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_delays_become_100ms() {
        assert_eq!(frame_delay(0), Duration::from_millis(100));
        assert_eq!(frame_delay(1), Duration::from_millis(100));
        assert_eq!(frame_delay(4), Duration::from_millis(40));
    }

    #[test]
    fn blend_skips_transparent_pixels_and_clips() {
        // Canvas 2x1 vermelho; quadro 2x1 com um pixel transparente, deslocado 1 para a direita.
        let mut canvas = vec![0, 0, 255, 255, 0, 0, 255, 255];
        let frame = vec![255, 0, 0, 255, 0, 0, 0, 0];
        blend(&mut canvas, 2, 1, &frame, 2, 1, 1, 0);
        assert_eq!(&canvas[0..4], &[0, 0, 255, 255]);
        assert_eq!(&canvas[4..8], &[255, 0, 0, 255]);
    }

    #[test]
    fn fill_rect_clears_only_inside() {
        let mut canvas = vec![9u8; 3 * 2 * 4];
        fill_rect(&mut canvas, 3, 2, 1, 0, 5, 1);
        assert_eq!(&canvas[0..4], &[9, 9, 9, 9]);
        assert!(canvas[4..12].iter().all(|&b| b == 0));
        assert!(canvas[12..].iter().all(|&b| b == 9));
    }
}
