//! O que está tocando no Windows (Spotify, navegador, Filmes e TV...), pelos controles de mídia do
//! sistema: a mesma informação do quadrinho que aparece com as teclas de volume. Alimenta as skins
//! com o plugin NowPlaying e recebe os comandos dos botões delas. O player escolhido na skin
//! (`PlayerName`) não importa: vale o que o Windows diz que está tocando.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use kage_skins::NowPlaying;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

/// De quanto em quanto tempo pergunta de novo ao Windows; no meio, a posição anda pelo relógio.
const REFRESH: Duration = Duration::from_secs(1);
/// Logo depois de um botão, pergunta bem mais vezes: o player leva um instante para mudar.
const QUICK_REFRESH: Duration = Duration::from_millis(150);
/// Por quanto tempo vale a pressa e o estado previsto depois de um botão.
const SETTLE: Duration = Duration::from_secs(2);
/// Unidade do TimeSpan e do DateTime do Windows: 100 ns.
const TICKS_PER_SECOND: f64 = 10_000_000.0;

/// Comandos dos botões (o `!CommandMeasure` do NowPlaying).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaCommand {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    /// Segundos desde o começo da música.
    SetPosition(f64),
}

#[derive(Default)]
pub struct MediaSource {
    manager: RefCell<Option<Manager>>,
    /// Já tentou abrir o gerenciador. Ele só abre quando alguma skin usa o "tocando agora".
    opened: Cell<bool>,
    last: RefCell<Option<(Instant, Option<NowPlaying>)>>,
    /// Até quando perguntar com pressa, e o estado que o último botão deve trazer (tocando,
    /// pausado ou parado). Enquanto o player não confirma, a skin já mostra o estado novo.
    settle: Cell<Option<Instant>>,
    expected: Cell<Option<u8>>,
}

impl MediaSource {
    /// O que está tocando agora, com a posição em dia.
    pub fn snapshot(&self) -> Option<NowPlaying> {
        let now = Instant::now();
        let settling = self.settle.get().is_some_and(|until| now < until);
        let refresh = if settling { QUICK_REFRESH } else { REFRESH };
        if let Some((at, info)) = self.last.borrow().as_ref()
            && now.duration_since(*at) < refresh
        {
            return moved(info, now.duration_since(*at));
        }
        let mut info = self.read();
        match (self.expected.get(), info.as_mut()) {
            // O player ainda não mudou: mostra o que o botão pediu até ele confirmar.
            (Some(state), Some(n)) if settling && n.state != state => n.state = state,
            _ => self.expected.set(None),
        }
        if !settling {
            self.settle.set(None);
        }
        *self.last.borrow_mut() = Some((now, info.clone()));
        info
    }

    /// Manda o comando ao player que está tocando. Sem player, não faz nada.
    pub fn command(&self, command: MediaCommand) {
        let Some(session) = self.session() else { return };
        let request = match command {
            MediaCommand::PlayPause => session.TryTogglePlayPauseAsync(),
            MediaCommand::Play => session.TryPlayAsync(),
            MediaCommand::Pause => session.TryPauseAsync(),
            MediaCommand::Stop => session.TryStopAsync(),
            MediaCommand::Next => session.TrySkipNextAsync(),
            MediaCommand::Previous => session.TrySkipPreviousAsync(),
            MediaCommand::SetPosition(seconds) => {
                session.TryChangePlaybackPositionAsync((seconds.max(0.0) * TICKS_PER_SECOND) as i64)
            }
        };
        let _ = request.and_then(|op| op.join());
        // O botão troca na hora pelo estado previsto, e as próximas leituras vêm com pressa até o
        // player confirmar.
        let now = Instant::now();
        let mut last = self.last.borrow_mut();
        let current = last.as_ref().and_then(|(at, info)| moved(info, now.duration_since(*at)));
        let expected = match command {
            MediaCommand::PlayPause => current.as_ref().map(|n| if n.state == 1 { 2 } else { 1 }),
            MediaCommand::Play => Some(1),
            MediaCommand::Pause => Some(2),
            MediaCommand::Stop => Some(0),
            _ => None,
        };
        self.expected.set(expected);
        self.settle.set(Some(now + SETTLE));
        *last = Some((
            now,
            current.map(|mut n| {
                if let Some(state) = expected {
                    n.state = state;
                }
                if let MediaCommand::SetPosition(seconds) = command {
                    n.position = seconds.max(0.0);
                }
                n
            }),
        ));
    }

    fn session(&self) -> Option<Session> {
        if !self.opened.replace(true) {
            *self.manager.borrow_mut() = Manager::RequestAsync().and_then(|op| op.join()).ok();
        }
        self.manager.borrow().as_ref()?.GetCurrentSession().ok()
    }

    fn read(&self) -> Option<NowPlaying> {
        let session = self.session()?;
        let properties = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let timeline = session.GetTimelineProperties().ok()?;
        let state = match session.GetPlaybackInfo().ok()?.PlaybackStatus().ok()? {
            Status::Playing => 1,
            Status::Paused => 2,
            _ => 0,
        };
        let start = timeline.StartTime().map(|t| seconds(t.Duration)).unwrap_or(0.0);
        let duration = (timeline.EndTime().map(|t| seconds(t.Duration)).unwrap_or(0.0) - start).max(0.0);
        let mut position = timeline.Position().map(|t| seconds(t.Duration)).unwrap_or(0.0) - start;
        // O player só atualiza a posição de vez em quando: soma o tempo desde a última vez.
        if state == 1
            && let Ok(updated) = timeline.LastUpdatedTime()
        {
            position = advance(position, seconds(now_filetime() - updated.UniversalTime), duration);
        }
        let text = |value: windows_core::Result<windows_core::HSTRING>| value.map(|s| s.to_string()).unwrap_or_default();
        Some(NowPlaying {
            artist: text(properties.Artist()),
            title: text(properties.Title()),
            album: text(properties.AlbumTitle()),
            position: position.max(0.0),
            duration,
            state,
            cover: None,
        })
    }
}

/// A leitura guardada, com a posição andada pelo tempo que passou desde ela.
fn moved(info: &Option<NowPlaying>, elapsed: Duration) -> Option<NowPlaying> {
    info.clone().map(|mut n| {
        if n.state == 1 {
            n.position = advance(n.position, elapsed.as_secs_f64(), n.duration);
        }
        n
    })
}

fn seconds(ticks: i64) -> f64 {
    ticks as f64 / TICKS_PER_SECOND
}

/// Posição andando `elapsed` segundos, sem passar do fim (transmissão ao vivo não tem fim).
fn advance(position: f64, elapsed: f64, duration: f64) -> f64 {
    let moved = position + elapsed.max(0.0);
    if duration > 0.0 { moved.min(duration) } else { moved }
}

/// Agora no relógio do DateTime do Windows (100 ns desde 1601, UTC).
fn now_filetime() -> i64 {
    let t = unsafe { GetSystemTimeAsFileTime() };
    ((t.dwHighDateTime as i64) << 32) | t.dwLowDateTime as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_advances_but_never_past_the_end() {
        assert_eq!(advance(10.0, 2.5, 200.0), 12.5);
        assert_eq!(advance(199.0, 5.0, 200.0), 200.0);
        assert_eq!(advance(30.0, 5.0, 0.0), 35.0, "ao vivo, sem duração");
        assert_eq!(advance(30.0, -1.0, 200.0), 30.0, "relógio atrasado não volta a música");
    }
}
