//! Reprodutor de vídeo sobre o Media Engine do Media Foundation.
//!
//! Roda em "modo servidor de quadros": o Media Engine decodifica na GPU e o motor copia
//! cada quadro novo, já convertido e redimensionado pela GPU, para a tela de cada monitor.
//! Um único decodificador atende todos os monitores que mostram o mesmo vídeo.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use windows::Win32::Foundation::{RECT, S_OK};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFMediaEngine, IMFMediaEngineClassFactory,
    IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, MF_ACCESSMODE_READ,
    MF_FILEFLAGS_NONE, MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_DXGI_MANAGER,
    MF_MEDIA_ENGINE_EVENT, MF_MEDIA_ENGINE_EVENT_ERROR, MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA,
    MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
    MF_OPENMODE_FAIL_IF_NOT_EXIST, MFARGB, MFCreateAttributes, MFCreateFile,
    MFVideoNormalizedRect,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows_core::{BSTR, HRESULT, HSTRING, Interface, Result, implement};

use crate::gpu::Gpu;

#[derive(Default)]
struct EngineState {
    metadata_loaded: AtomicBool,
    error: AtomicI32,
}

#[implement(IMFMediaEngineNotify)]
struct Notify {
    state: Arc<EngineState>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, param2: u32) -> Result<()> {
        if std::env::var_os("KAGE_DEBUG").is_some() {
            eprintln!("[media engine] evento {event} param2={param2:#x}");
        }
        match MF_MEDIA_ENGINE_EVENT(event as i32) {
            MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA => {
                self.state.metadata_loaded.store(true, Ordering::SeqCst);
            }
            MF_MEDIA_ENGINE_EVENT_ERROR => {
                // param2 traz o HRESULT do erro.
                self.state.error.store(param2 as i32, Ordering::SeqCst);
            }
            _ => {}
        }
        Ok(())
    }
}

pub struct Player {
    engine: IMFMediaEngine,
    state: Arc<EngineState>,
}

impl Player {
    pub fn open(gpu: &Gpu, path: &Path) -> Result<Self> {
        unsafe {
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;

            let state = Arc::new(EngineState::default());
            let notify: IMFMediaEngineNotify = Notify { state: state.clone() }.into();

            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 3)?;
            let attributes = attributes.expect("MFCreateAttributes sem atributos");
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &gpu.device_manager)?;
            attributes.SetUINT32(
                &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
            )?;

            let engine = factory.CreateInstance(0, &attributes)?;
            engine.SetMuted(true)?;
            engine.SetLoop(true)?;
            engine.SetAutoPlay(false)?;
            engine.SetPreload(MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC)?;

            // O Media Engine só aceita URL; um caminho como C:\... vira "sem fonte".
            // Abrindo o arquivo aqui, espaços e acentos no caminho não importam.
            let wide_path = HSTRING::from(path.as_os_str());
            let stream = MFCreateFile(
                MF_ACCESSMODE_READ,
                MF_OPENMODE_FAIL_IF_NOT_EXIST,
                MF_FILEFLAGS_NONE,
                &wide_path,
            )?;
            let engine_ex: IMFMediaEngineEx = engine.cast()?;
            // A URL aqui só serve de dica do formato, pela extensão.
            engine_ex.SetSourceFromByteStream(&stream, &BSTR::from(path.to_string_lossy().as_ref()))?;

            Ok(Self { engine, state })
        }
    }

    pub fn is_ready(&self) -> bool {
        self.state.metadata_loaded.load(Ordering::SeqCst)
    }

    /// Estado interno para diagnóstico: (readyState, networkState, código de erro).
    pub fn debug_state(&self) -> (u16, u16, u16) {
        unsafe {
            let ready = self.engine.GetReadyState();
            let network = self.engine.GetNetworkState();
            let error = self.engine.GetError().map(|e| e.GetErrorCode()).unwrap_or(0);
            (ready, network, error)
        }
    }

    pub fn error(&self) -> Option<HRESULT> {
        match self.state.error.load(Ordering::SeqCst) {
            0 => None,
            code => Some(HRESULT(code)),
        }
    }

    pub fn video_size(&self) -> Option<(u32, u32)> {
        let (mut width, mut height) = (0u32, 0u32);
        unsafe { self.engine.GetNativeVideoSize(Some(&mut width), Some(&mut height)) }.ok()?;
        (width > 0 && height > 0).then_some((width, height))
    }

    pub fn play(&self) -> Result<()> {
        unsafe { self.engine.Play() }
    }

    /// Velocidade (1 = normal). Vale também depois de um seek.
    pub fn set_rate(&self, rate: f64) -> Result<()> {
        unsafe {
            self.engine.SetDefaultPlaybackRate(rate)?;
            self.engine.SetPlaybackRate(rate)
        }
    }

    /// Pausado, o Media Engine para de decodificar: a GPU fica parada.
    pub fn pause(&self) -> Result<()> {
        unsafe { self.engine.Pause() }
    }

    /// Posição atual em segundos.
    pub fn position(&self) -> f64 {
        unsafe { self.engine.GetCurrentTime() }
    }

    pub fn seek(&self, seconds: f64) -> Result<()> {
        unsafe { self.engine.SetCurrentTime(seconds) }
    }

    /// true quando há um quadro novo para mostrar. O retorno S_FALSE (sem quadro novo)
    /// vira Ok no wrapper do windows-rs, por isso a chamada vai direto pela vtable.
    pub fn has_new_frame(&self) -> bool {
        let mut pts = 0i64;
        let hr = unsafe {
            (Interface::vtable(&self.engine).OnVideoStreamTick)(
                Interface::as_raw(&self.engine),
                &mut pts,
            )
        };
        hr == S_OK
    }

    /// Copia o quadro atual para a textura, recortando `source` (0 a 1) e encaixando em `target`.
    pub fn transfer(
        &self,
        texture: &ID3D11Texture2D,
        source: &MFVideoNormalizedRect,
        target: &RECT,
    ) -> Result<()> {
        let border = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
        unsafe { self.engine.TransferVideoFrame(texture, Some(source), target, Some(&border)) }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}
