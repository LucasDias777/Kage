//! Tipos de mídia aceitos e a fonte aberta de cada wallpaper.

use std::path::{Path, PathBuf};
use std::time::Instant;

use windows_core::Result;

use crate::gpu::Gpu;
use crate::image::{AnimatedGif, StillImage};
use crate::video::Player;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Image,
    Gif,
}

pub fn kind_of(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" => Some(MediaKind::Video),
        "png" | "jpg" | "jpeg" | "bmp" | "webp" | "tif" | "tiff" | "jxr" => Some(MediaKind::Image),
        "gif" => Some(MediaKind::Gif),
        _ => None,
    }
}

/// Uma mídia aberta. Monitores que mostram o mesmo arquivo dividem a mesma fonte.
pub enum Source {
    Video(VideoSource),
    Still(StillImage),
    Gif(AnimatedGif),
}

impl Source {
    pub fn open(gpu: &Gpu, path: &Path) -> Result<Self> {
        match kind_of(path) {
            Some(MediaKind::Video) => Ok(Self::Video(VideoSource::open(gpu, path)?)),
            Some(MediaKind::Image) => Ok(Self::Still(StillImage::open(gpu, path)?)),
            Some(MediaKind::Gif) => Ok(Self::Gif(AnimatedGif::open(gpu, path)?)),
            None => Err(windows_core::Error::new(
                windows::Win32::Foundation::E_INVALIDARG,
                format!("formato não suportado: {}", path.display()),
            )),
        }
    }

    /// Tamanho em pixels; o vídeo só sabe depois de ler o cabeçalho.
    pub fn size(&self) -> Option<(u32, u32)> {
        match self {
            Self::Video(v) => v.size(),
            Self::Still(i) => Some(i.size),
            Self::Gif(g) => Some(g.size),
        }
    }

    pub fn is_ready(&self) -> bool {
        match self {
            Self::Video(v) => v.is_ready(),
            _ => true,
        }
    }
}

/// Vídeo que pode soltar o decodificador quando fica muito tempo pausado e voltar de onde parou.
pub struct VideoSource {
    path: PathBuf,
    player: Option<Player>,
    size: Option<(u32, u32)>,
    playing: bool,
    start_pending: bool,
    resume_at: f64,
    /// Velocidade escolhida no "Customizar" (1 = normal).
    pub rate: f64,
}

impl VideoSource {
    fn open(gpu: &Gpu, path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            player: Some(Player::open(gpu, path)?),
            size: None,
            playing: false,
            start_pending: false,
            resume_at: 0.0,
            rate: 1.0,
        })
    }

    pub fn player(&self) -> Option<&Player> {
        self.player.as_ref()
    }

    pub fn is_ready(&self) -> bool {
        self.player.as_ref().is_none_or(Player::is_ready)
    }

    pub fn size(&self) -> Option<(u32, u32)> {
        self.size.or_else(|| self.player.as_ref()?.video_size())
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn is_released(&self) -> bool {
        self.player.is_none()
    }

    pub fn resume_position(&self) -> f64 {
        self.resume_at
    }

    pub fn set_playing(&mut self, gpu: &Gpu, play: bool) -> Result<()> {
        if play == self.playing {
            return Ok(());
        }
        self.playing = play;
        if play {
            if self.player.is_none() {
                self.player = Some(Player::open(gpu, &self.path)?);
            }
            // Começa no `tick`, quando o arquivo estiver pronto.
            self.start_pending = true;
        } else {
            self.start_pending = false;
            if let Some(player) = &self.player {
                player.pause()?;
            }
        }
        Ok(())
    }

    /// Solta o decodificador e a memória dele, guardando a posição. Só vale pausado.
    pub fn release(&mut self) -> bool {
        if self.playing {
            return false;
        }
        match self.player.take() {
            Some(player) => {
                self.resume_at = player.position();
                true
            }
            None => false,
        }
    }

    /// Avança a fonte. Devolve true quando há quadro novo para desenhar.
    pub fn tick(&mut self) -> Result<bool> {
        let Some(player) = &self.player else { return Ok(false) };
        if let Some(hr) = player.error() {
            return Err(windows_core::Error::new(hr, "erro durante a reprodução"));
        }
        if self.start_pending && player.is_ready() {
            if self.size.is_none() {
                self.size = player.video_size();
            }
            if self.resume_at > 0.0 {
                player.seek(self.resume_at)?;
                self.resume_at = 0.0;
            }
            player.play()?;
            if self.rate != 1.0 {
                player.set_rate(self.rate)?;
            }
            self.start_pending = false;
        }
        Ok(self.playing && !self.start_pending && player.has_new_frame())
    }
}

impl AnimatedGif {
    /// Pausar o GIF é só parar de avançar; ao voltar, o relógio recomeça de agora.
    pub fn set_playing(&mut self, play: bool, now: Instant) {
        if play && !self.playing {
            self.restart_clock(now);
        }
        self.playing = play;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kind_by_extension_ignoring_case() {
        assert_eq!(kind_of(Path::new("a/katana.MP4")), Some(MediaKind::Video));
        assert_eq!(kind_of(Path::new("fundo.webp")), Some(MediaKind::Image));
        assert_eq!(kind_of(Path::new("chuva.gif")), Some(MediaKind::Gif));
        assert_eq!(kind_of(Path::new("leia.txt")), None);
        assert_eq!(kind_of(Path::new("sem-extensao")), None);
    }
}
