//! MPRIS на Linux: медиаклавиши, плеер в панели GNOME и KDE, playerctl. Сервер D-Bus живёт
//! в своём потоке на однопоточном исполнителе: `mpris_server::Player` не Send. Состояние,
//! как и у статуса Discord, он берёт у плеера сам, а команды отдаёт в поток интерфейса.

use crate::api::Track;
use crate::config::log;
use crate::player::{Player, Shared};
use async_io::Timer;
use mpris_server::{Metadata, PlaybackStatus, Player as Server, Time, TrackId};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Команда от системы. Время — в секундах.
pub enum Event {
    PlayPause,
    Play,
    Pause,
    Next,
    Prev,
    Seek(f32),
    SeekBy(f32),
    Volume(f32),
    Raise,
    Quit,
}

/// Поднять сервер. Без сессионной шины D-Bus (WSL, голый оконный менеджер) приложение
/// просто работает без MPRIS, причина остаётся в логе.
pub fn spawn(player: Player, on_event: impl Fn(Event) + Send + 'static) {
    let started = std::thread::Builder::new().name("mpris".into()).spawn(move || {
        if let Err(e) = async_io::block_on(run(player, on_event)) {
            log(format!("MPRIS: {e}"));
        }
    });
    if let Err(e) = started {
        log(format!("MPRIS: поток не запустился: {e}"));
    }
}

async fn run(player: Player, on_event: impl Fn(Event) + 'static) -> mpris_server::zbus::Result<()> {
    let server = Server::builder("yamusic_rs")
        .identity(env!("APP_NAME"))
        .desktop_entry("yamusic-rs")
        .can_raise(true)
        .can_quit(true)
        .can_play(true)
        .can_pause(true)
        .can_seek(true)
        .build()
        .await?;

    let on_event = Rc::new(on_event);
    let send = |e: fn() -> Event| {
        let f = on_event.clone();
        move |_: &Server| f(e())
    };
    server.connect_play_pause(send(|| Event::PlayPause));
    server.connect_play(send(|| Event::Play));
    server.connect_pause(send(|| Event::Pause));
    server.connect_stop(send(|| Event::Pause));
    server.connect_next(send(|| Event::Next));
    server.connect_previous(send(|| Event::Prev));
    server.connect_raise(send(|| Event::Raise));
    server.connect_quit(send(|| Event::Quit));
    let f = on_event.clone();
    server.connect_seek(move |_, offset| f(Event::SeekBy(secs(offset))));
    let f = on_event.clone();
    server.connect_set_position(move |_, _, pos| f(Event::Seek(secs(pos))));
    let f = on_event.clone();
    server.connect_set_volume(move |_, v| f(Event::Volume(v.clamp(0.0, 1.0) as f32)));

    futures_lite::future::or(server.run(), follow(&server, &player)).await;
    Ok(())
}

/// Раз в полсекунды переносит состояние плеера в свойства MPRIS. Сигналы об изменениях
/// сервер шлёт сам, но только когда значение действительно поменялось.
async fn follow(server: &Server, player: &Player) {
    let mut track: Option<String> = None;
    // Где была позиция при прошлой проверке: скачок больше пары секунд — это перемотка,
    // о ней клиентам нужно сообщить сигналом Seeked.
    let mut last: Option<(Instant, f32)> = None;
    loop {
        Timer::after(Duration::from_millis(500)).await;
        let s = player.state();
        let id = s.track.as_ref().map(|t| t.id.clone());
        if id != track {
            let metadata = s.track.as_ref().map_or_else(Metadata::new, |t| metadata(t, s.duration));
            let has = s.track.is_some();
            let _ = server.set_metadata(metadata).await;
            let _ = server.set_can_go_next(has).await;
            let _ = server.set_can_go_previous(has).await;
            track = id;
            last = None;
        }
        let _ = server.set_playback_status(status(&s)).await;
        let _ = server.set_volume(f64::from(s.volume)).await;
        server.set_position(time(s.position));
        let expected = last.map(|(at, pos)| if s.playing { pos + at.elapsed().as_secs_f32() } else { pos });
        if expected.is_some_and(|e| (e - s.position).abs() > 2.0) {
            let _ = server.seeked(time(s.position)).await;
        }
        last = Some((Instant::now(), s.position));
    }
}

fn status(s: &Shared) -> PlaybackStatus {
    match (&s.track, s.playing) {
        (None, _) => PlaybackStatus::Stopped,
        (Some(_), true) => PlaybackStatus::Playing,
        (Some(_), false) => PlaybackStatus::Paused,
    }
}

fn metadata(t: &Track, duration: f32) -> Metadata {
    let title = if t.version.is_empty() { t.title.clone() } else { format!("{} ({})", t.title, t.version) };
    let mut b = Metadata::builder().trackid(track_id(&t.id)).title(title).artist([t.artists.clone()]);
    if !t.album_title.is_empty() {
        b = b.album(t.album_title.clone());
    }
    if duration > 0.0 {
        b = b.length(time(duration));
    } else if t.duration_ms > 0 {
        b = b.length(Time::from_millis(t.duration_ms as i64));
    }
    if let Some(url) = t.cover_url(400) {
        b = b.art_url(url);
    }
    b.build()
}

/// Путь объекта D-Bus для трека: в нём допустимы только буквы, цифры и `_`.
fn track_id(id: &str) -> TrackId {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    TrackId::try_from(format!("/io/github/spolzermod/yamusic_rs/track/t{safe}")).unwrap_or(TrackId::NO_TRACK)
}

fn time(secs: f32) -> Time {
    Time::from_micros((secs.max(0.0) * 1_000_000.0) as i64)
}

fn secs(t: Time) -> f32 {
    t.as_micros() as f32 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    #[test]
    fn track_id() {
        let ok = |id: &str| super::track_id(id) != mpris_server::TrackId::NO_TRACK;
        assert!(ok("12345"));
        assert!(ok("12345:678"));
        assert!(ok(""));
    }
}
