//! Dispositivo de GPU compartilhado: Direct3D 11 para o vídeo, Direct2D para imagens e GIFs.

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::{
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_MULTI_THREADED, D2D1CreateFactory,
    ID2D1DeviceContext, ID2D1Factory1,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11CreateDevice, ID3D11Device, ID3D11Multithread,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3, IDXGIDevice,
    IDXGIDevice3, IDXGIFactory2,
};
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::Media::MediaFoundation::{IMFDXGIDeviceManager, MFCreateDXGIDeviceManager};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows_core::{Interface, Result};

pub struct Gpu {
    pub device: ID3D11Device,
    pub factory: IDXGIFactory2,
    pub device_manager: IMFDXGIDeviceManager,
    pub d2d: ID2D1DeviceContext,
    pub wic: IWICImagingFactory,
}

impl Gpu {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut device = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
            let device = device.expect("D3D11CreateDevice sem dispositivo");

            // O Media Foundation usa o mesmo dispositivo em outra thread.
            let multithread: ID3D11Multithread = device.cast()?;
            let _ = multithread.SetMultithreadProtected(true);

            let dxgi_device: IDXGIDevice = device.cast()?;
            let factory: IDXGIFactory2 = dxgi_device.GetAdapter()?.GetParent()?;

            let mut token = 0u32;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let device_manager = manager.expect("MFCreateDXGIDeviceManager sem gerenciador");
            device_manager.ResetDevice(&device, token)?;

            // Direct2D em modo multithread porque divide o dispositivo com o Media Foundation.
            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)?;
            let d2d = d2d_factory
                .CreateDevice(&dxgi_device)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;

            let wic = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

            Ok(Self { device, factory, device_manager, d2d, wic })
        }
    }

    /// VRAM que este processo usa agora, em MB, como o driver informa.
    pub fn video_memory_mb(&self) -> u64 {
        unsafe {
            let info = self
                .device
                .cast::<IDXGIDevice>()
                .and_then(|d| d.GetAdapter())
                .and_then(|a| a.cast::<IDXGIAdapter3>())
                .and_then(|a| {
                    let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
                    a.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info).map(|_| info)
                });
            info.map(|i| i.CurrentUsage / (1024 * 1024)).unwrap_or(0)
        }
    }

    /// Devolve de fato a memória de vídeo depois de soltar um decodificador.
    /// O Direct3D 11 só destrói recursos quando a fila da GPU é esvaziada; o Trim pede ao
    /// driver as alocações temporárias de volta.
    pub fn trim(&self) {
        unsafe {
            let Ok(multithread) = self.device.cast::<ID3D11Multithread>() else { return };
            multithread.Enter();
            if let Ok(context) = self.device.GetImmediateContext() {
                context.ClearState();
                context.Flush();
            }
            if let Ok(dxgi) = self.device.cast::<IDXGIDevice3>() {
                dxgi.Trim();
            }
            multithread.Leave();
        }
    }
}
