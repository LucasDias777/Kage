//! Uma tela de wallpaper: janela presa ao WorkerW sobre um monitor, com swapchain própria.

use std::mem::ManuallyDrop;
use std::sync::Once;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1ColorMatrix, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_COLORMATRIX_PROP_CLAMP_OUTPUT,
    D2D1_COLORMATRIX_PROP_COLOR_MATRIX, D2D1_INTERPOLATION_MODE, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_PROPERTY_TYPE_BOOL, D2D1_PROPERTY_TYPE_MATRIX_5X4,
    ID2D1Bitmap1, ID2D1Effect, ID2D1Image,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGISurface,
    IDXGISwapChain1,
};
use windows::Win32::Media::MediaFoundation::MFVideoNormalizedRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows_core::{BOOL, Interface, Result, w};
use windows_numerics::Matrix3x2;

use crate::desktop;
use crate::gpu::Gpu;
use crate::layout::Placement;
use crate::monitors::Monitor;
use crate::video::Player;

const CLASS_NAME: windows_core::PCWSTR = w!("KageWallpaperSurface");
const BLACK: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

pub struct Surface {
    hwnd: HWND,
    swapchain: IDXGISwapChain1,
    pub monitor: Monitor,
    /// Índice da fonte (mídia) que esta tela mostra.
    pub source: usize,
    /// `None` quando a mídia não chega neste monitor (Span em modo ajustar, por exemplo).
    pub placement: Option<Placement>,
    /// Textura do quadro de vídeo, usada só quando há ajuste de cor.
    base: Option<(ID3D11Texture2D, ID2D1Bitmap1)>,
    /// Brilho, contraste e saturação do "Customizar". Sem ajuste, o vídeo vai direto para a tela.
    effect: Option<ID2D1Effect>,
}

/// O que vai para a tela.
enum Content<'a> {
    /// Quadro de vídeo já encaixado no monitor.
    Base(&'a ID2D1Bitmap1),
    Media(Option<(&'a ID2D1Bitmap1, (u32, u32))>),
}

/// Matriz 5x4 do efeito ColorMatrix (linha = cor de entrada, coluna = cor de saída, a última
/// linha soma). Saturação pelos pesos de luminância do Rec. 709, contraste em torno do cinza
/// médio e brilho multiplicando no fim.
pub fn color_matrix(look: &kage_core::Look) -> [f32; 20] {
    let brightness = look.brightness / 100.0;
    let contrast = look.contrast / 100.0;
    let saturation = look.saturation / 100.0;
    let weights = [0.2126f32, 0.7152, 0.0722];
    let gain = brightness * contrast;
    let mut m = [0.0f32; 20];
    for input in 0..3 {
        for output in 0..3 {
            let same = if input == output { saturation } else { 0.0 };
            m[input * 4 + output] = ((1.0 - saturation) * weights[input] + same) * gain;
        }
    }
    m[3 * 4 + 3] = 1.0;
    for output in 0..3 {
        m[4 * 4 + output] = brightness * 0.5 * (1.0 - contrast);
    }
    m
}

impl Surface {
    pub fn new(gpu: &Gpu, workerw: HWND, monitor: &Monitor, source: usize) -> Result<Self> {
        register_class()?;
        let (width, height) = (monitor.width(), monitor.height());
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("Kage"),
                WS_POPUP,
                monitor.rect.left,
                monitor.rect.top,
                width as i32,
                height as i32,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            desktop::attach(hwnd, workerw, monitor.rect)?;

            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
                ..Default::default()
            };
            let swapchain = gpu.factory.CreateSwapChainForHwnd(&gpu.device, hwnd, &desc, None, None)?;
            gpu.factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;

            Ok(Self { hwnd, swapchain, monitor: monitor.clone(), source, placement: None, base: None, effect: None })
        }
    }

    /// Liga ou desliga o ajuste de cor desta tela.
    pub fn set_colors(&mut self, gpu: &Gpu, look: &kage_core::Look) -> Result<()> {
        if look.colors_untouched() {
            self.effect = None;
            self.base = None;
            return Ok(());
        }
        let matrix = color_matrix(look);
        let clamp = BOOL(1);
        unsafe {
            let effect = gpu.d2d.CreateEffect(&CLSID_D2D1ColorMatrix)?;
            let bytes = std::slice::from_raw_parts(matrix.as_ptr().cast::<u8>(), size_of_val(&matrix));
            effect.SetValue(D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32, D2D1_PROPERTY_TYPE_MATRIX_5X4, bytes)?;
            let flag = std::slice::from_raw_parts((&clamp as *const BOOL).cast::<u8>(), size_of::<BOOL>());
            effect.SetValue(D2D1_COLORMATRIX_PROP_CLAMP_OUTPUT.0 as u32, D2D1_PROPERTY_TYPE_BOOL, flag)?;
            self.effect = Some(effect);
        }
        Ok(())
    }

    fn create_base(&self, gpu: &Gpu) -> Result<(ID3D11Texture2D, ID2D1Bitmap1)> {
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: self.monitor.width(),
                Height: self.monitor.height(),
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                ..Default::default()
            };
            let mut texture = None;
            gpu.device.CreateTexture2D(&desc, None, Some(&mut texture))?;
            let texture = texture.expect("CreateTexture2D sem textura");
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                colorContext: ManuallyDrop::new(None),
            };
            let surface: IDXGISurface = texture.cast()?;
            let bitmap = gpu.d2d.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            Ok((texture, bitmap))
        }
    }

    /// Quadro de vídeo. Sem ajuste de cor, o Media Engine converte e redimensiona direto na
    /// tela, pela GPU; com ajuste, o quadro passa por uma textura e pelo efeito.
    pub fn draw_video(&mut self, gpu: &Gpu, player: &Player) -> Result<()> {
        let Some(p) = self.placement else { return self.draw_bitmap(gpu, None, (0, 0)) };
        let source = MFVideoNormalizedRect {
            left: p.source.left,
            top: p.source.top,
            right: p.source.right,
            bottom: p.source.bottom,
        };
        if self.effect.is_some() {
            if self.base.is_none() {
                self.base = Some(self.create_base(gpu)?);
            }
            let (texture, base) = self.base.clone().expect("textura criada acima");
            // Sem quadro pronto ainda (vídeo acabou de abrir): fica o anterior.
            let _ = player.transfer(&texture, &source, &p.target);
            return self.paint(gpu, Content::Base(&base));
        }
        unsafe {
            let back_buffer: ID3D11Texture2D = self.swapchain.GetBuffer(0)?;
            player.transfer(&back_buffer, &source, &p.target)?;
        }
        self.present()
    }

    /// Imagem ou quadro de GIF pelo Direct2D. Sem bitmap, fundo preto.
    pub fn draw_bitmap(&self, gpu: &Gpu, bitmap: Option<&ID2D1Bitmap1>, size: (u32, u32)) -> Result<()> {
        self.paint(gpu, Content::Media(bitmap.map(|b| (b, size))))
    }

    /// Desenha `source` (pixels da imagem) em `dest` (pixels da tela), passando pelo ajuste de cor.
    fn draw(&self, gpu: &Gpu, bitmap: &ID2D1Bitmap1, dest: D2D_RECT_F, source: D2D_RECT_F, mode: D2D1_INTERPOLATION_MODE) {
        let dc = &gpu.d2d;
        unsafe {
            let Some(effect) = &self.effect else {
                dc.DrawBitmap(bitmap, Some(&dest), 1.0, mode, Some(&source), None);
                return;
            };
            let image: ID2D1Image = match bitmap.cast() {
                Ok(image) => image,
                Err(_) => return,
            };
            effect.SetInput(0, &image, true);
            let Ok(output) = effect.GetOutput() else { return };
            let sx = (dest.right - dest.left) / (source.right - source.left).max(1.0);
            let sy = (dest.bottom - dest.top) / (source.bottom - source.top).max(1.0);
            dc.SetTransform(&Matrix3x2 {
                M11: sx,
                M12: 0.0,
                M21: 0.0,
                M22: sy,
                M31: dest.left - source.left * sx,
                M32: dest.top - source.top * sy,
            });
            dc.DrawImage(&output, None, Some(&source), mode, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            dc.SetTransform(&Matrix3x2::identity());
        }
    }

    fn paint(&self, gpu: &Gpu, content: Content) -> Result<()> {
        unsafe {
            let back_buffer: IDXGISurface = self.swapchain.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let target = gpu.d2d.CreateBitmapFromDxgiSurface(&back_buffer, Some(&props))?;
            gpu.d2d.SetTarget(&target);
            gpu.d2d.BeginDraw();
            gpu.d2d.Clear(Some(&BLACK));
            match content {
                Content::Base(base) => {
                    let (w, h) = (self.monitor.width() as f32, self.monitor.height() as f32);
                    let all = D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: h };
                    self.draw(gpu, base, all, all, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR);
                }
                Content::Media(Some((bitmap, size))) => {
                    if let Some(p) = self.placement {
                        let (w, h) = (size.0 as f32, size.1 as f32);
                        let source = D2D_RECT_F {
                            left: p.source.left * w,
                            top: p.source.top * h,
                            right: p.source.right * w,
                            bottom: p.source.bottom * h,
                        };
                        let dest = D2D_RECT_F {
                            left: p.target.left as f32,
                            top: p.target.top as f32,
                            right: p.target.right as f32,
                            bottom: p.target.bottom as f32,
                        };
                        self.draw(gpu, bitmap, dest, source, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC);
                    }
                }
                Content::Media(None) => {}
            }
            let drawn = gpu.d2d.EndDraw(None, None);
            gpu.d2d.SetTarget(None::<&ID2D1Image>);
            drawn?;
        }
        self.present()
    }

    fn present(&self) -> Result<()> {
        // Sem esperar vsync aqui: o laço principal já sincroniza com o compositor.
        unsafe { self.swapchain.Present(0, DXGI_PRESENT(0)).ok() }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn register_class() -> Result<()> {
    static REGISTER: Once = Once::new();
    let mut result = Ok(());
    REGISTER.call_once(|| unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: GetModuleHandleW(None).map(Into::into).unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            result = Err(windows_core::Error::from_thread());
        }
    });
    result
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untouched_look_is_the_identity_matrix() {
        let m = color_matrix(&kage_core::Look::default());
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        for (a, b) in m.iter().zip(identity) {
            assert!((a - b).abs() < 1e-6, "{m:?}");
        }
    }

    #[test]
    fn no_saturation_turns_every_channel_into_luminance() {
        let look = kage_core::Look { saturation: 0.0, ..Default::default() };
        let m = color_matrix(&look);
        // Vermelho puro sai cinza com o peso do vermelho nos três canais.
        assert!((m[0] - 0.2126).abs() < 1e-6 && (m[1] - 0.2126).abs() < 1e-6 && (m[2] - 0.2126).abs() < 1e-6);
        let darker = color_matrix(&kage_core::Look { contrast: 50.0, ..Default::default() });
        assert!((darker[0] - 0.5).abs() < 1e-6 && (darker[16] - 0.25).abs() < 1e-6, "metade do contraste puxa para o cinza");
    }
}
