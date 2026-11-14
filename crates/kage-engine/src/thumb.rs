//! Miniaturas da biblioteca: um quadro do vídeo (ou a própria imagem) reduzido para JPEG.

use std::path::Path;

use kage_core::MediaKind;
use windows::Win32::Foundation::{E_FAIL, GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Graphics::Imaging::{
    GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR, GUID_WICPixelFormat32bppBGR, IWICBitmapFrameEncode,
    IWICBitmapSource, IWICImagingFactory, IWICPalette, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::Media::MediaFoundation::{
    MF_ACCESSMODE_READ, MF_FILEFLAGS_NONE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READERF_ENDOFSTREAM, MFCreateAttributes, MFCreateFile, MFCreateMediaType,
    MFCreateSourceReaderFromByteStream, MFMediaType_Video, MFVideoFormat_RGB32,
};
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPVARIANT};
use windows::Win32::System::Variant::VT_I8;
use windows_core::{Error, GUID, HSTRING, Interface, Result};

const THUMB_WIDTH: u32 = 480;
/// Posição do quadro: o começo de muitos vídeos é preto.
const FRAME_AT_100NS: i64 = 20_000_000;

pub fn make(wic: &IWICImagingFactory, input: &Path, output: &Path) -> Result<()> {
    make_sized(wic, input, output, THUMB_WIDTH)
}

/// Mesma coisa com a largura escolhida. Com poucos pixels (48), a janela do Kage aumenta a
/// imagem e ela fica desfocada: é o fundo "Dinâmico" do diálogo Tema.
pub fn make_sized(wic: &IWICImagingFactory, input: &Path, output: &Path, width: u32) -> Result<()> {
    unsafe {
        let source: IWICBitmapSource = match kage_core::kind_of(input) {
            Some(MediaKind::Video) => {
                let (pixels, w, h) = video_frame(input)?;
                wic.CreateBitmapFromMemory(w, h, &GUID_WICPixelFormat32bppBGR, w * 4, &pixels)?.cast()?
            }
            _ => wic
                .CreateDecoderFromFilename(&HSTRING::from(input.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?
                .GetFrame(0)?
                .cast()?,
        };
        let (mut w, mut h) = (0u32, 0u32);
        source.GetSize(&mut w, &mut h)?;
        let thumb_w = width.min(w).max(1);
        let thumb_h = ((h as f64 * thumb_w as f64 / w.max(1) as f64).round() as u32).max(1);

        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&source, thumb_w, thumb_h, WICBitmapInterpolationModeFant)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat24bppBGR,
            WICBitmapDitherTypeNone,
            None::<&IWICPalette>,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        save_jpeg(wic, &converter.cast()?, thumb_w, thumb_h, output)
    }
}

/// Um quadro do vídeo em BGRA de cima para baixo.
fn video_frame(path: &Path) -> Result<(Vec<u8>, u32, u32)> {
    unsafe {
        let stream =
            MFCreateFile(MF_ACCESSMODE_READ, MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_FILEFLAGS_NONE, &HSTRING::from(path.as_os_str()))?;
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1)?;
        let attributes = attributes.expect("MFCreateAttributes sem atributos");
        // Pede ao leitor que converta para RGB32 (ele usa o processador de vídeo).
        attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        let reader = MFCreateSourceReaderFromByteStream(&stream, &attributes)?;
        let index = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

        let wanted = MFCreateMediaType()?;
        wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        wanted.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader.SetCurrentMediaType(index, None, &wanted)?;

        let mut position = PROPVARIANT::default();
        (*position.Anonymous.Anonymous).vt = VT_I8;
        (*position.Anonymous.Anonymous).Anonymous.hVal = FRAME_AT_100NS;
        // Vídeo mais curto que 2 s: fica no começo mesmo.
        let _ = reader.SetCurrentPosition(&GUID::zeroed(), &position);

        let mut sample = None;
        for _ in 0..120 {
            let mut flags = 0u32;
            reader.ReadSample(index, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            if sample.is_some() || flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
        }
        let sample = sample.ok_or_else(|| Error::new(E_FAIL, "o vídeo não devolveu nenhum quadro"))?;

        let current = reader.GetCurrentMediaType(index)?;
        let size = current.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (w, h) = ((size >> 32) as u32, (size & 0xFFFF_FFFF) as u32);
        let stride = current.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or((w * 4) as i32);

        let buffer = sample.ConvertToContiguousBuffer()?;
        let mut data = std::ptr::null_mut();
        let mut length = 0u32;
        buffer.Lock(&mut data, None, Some(&mut length))?;
        let row = (w * 4) as usize;
        let mut pixels = vec![0u8; row * h as usize];
        for y in 0..h as usize {
            // Passo negativo: a imagem vem de baixo para cima.
            let source_row = if stride < 0 { h as usize - 1 - y } else { y };
            let offset = source_row * stride.unsigned_abs() as usize;
            if offset + row <= length as usize {
                pixels[y * row..(y + 1) * row].copy_from_slice(std::slice::from_raw_parts(data.add(offset), row));
            }
        }
        buffer.Unlock()?;
        Ok((pixels, w, h))
    }
}

fn save_jpeg(wic: &IWICImagingFactory, source: &IWICBitmapSource, w: u32, h: u32, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    unsafe {
        let stream = wic.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatJpeg, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut options: Option<IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.expect("CreateNewFrame sem quadro");
        frame.Initialize(options.as_ref())?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(source, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()
    }
}
