//! Аудио-движок: очередь, загрузка и предзагрузка треков, фидбек волне, воспроизведение.
//! Живёт в своём потоке, UI шлёт ему команды и читает `Shared`.

use crate::ahead::Ahead;
use crate::api::{Account, Api, Batch, Res, Track, WaveSession, WAVE};
use crate::config::log;
use crate::eq::{Eq, EqParams};
use crate::stream::{Buf, Reader};
use rodio::{Decoder, OutputStream, Sink, Source as _};
use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq, Default, Debug)]
pub enum Status {
    #[default]
    LoggedOut,
    Connecting,
    Ready,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ListKind {
    Playlist,
    Album,
    Artist,
    Likes,
    Tracks,
}

/// Что сейчас играет.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum Source {
    #[default]
    Wave,
    /// Волна по станции: `artist:ID`, `track:ID`, `album:ID`, `playlist:UID_KIND`.
    Seed { station: String, header: String, title: String },
    List { kind: ListKind, id: String, title: String },
}

impl Source {
    pub fn station(&self) -> Option<&str> {
        match self {
            Source::Wave => Some(WAVE),
            Source::Seed { station, .. } => Some(station),
            Source::List { .. } => None,
        }
    }
}

/// Что играть, когда список закончится.
#[derive(Clone, Debug)]
pub enum After {
    Loop,
    Wave(Source),
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Default)]
pub struct Shared {
    pub status: Status,
    pub uid: String,
    pub login: String,
    pub has_plus: bool,
    pub track: Option<Track>,
    pub source: Source,
    pub upcoming: Vec<Track>,
    pub playing: bool,
    pub loading: bool,
    pub position: f32,
    pub duration: f32,
    pub volume: f32,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub error: Option<(String, Instant)>,
}

/// Как положение ползунка громкости переводится в усиление.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Loudness {
    Square,
    /// 50 дБ на всю шкалу: каждый шаг ползунка на слух одинаковый (#44).
    Decibels,
}

pub enum Cmd {
    Login(String),
    Logout,
    PlayPause,
    /// Снять с паузы (если уже играет — ничего не делать).
    Resume,
    Next,
    Prev,
    Seek(f32),
    Volume(f32),
    Loudness(Loudness),
    LikeTrack(Track),
    Dislike,
    Wave(Source),
    PlayList { source: Source, tracks: Vec<Track>, start: usize, after: After },
    /// Перейти к треку из очереди (индекс в `upcoming`).
    Jump(usize),
    Repeat(Repeat),
    Shuffle(bool),
    /// «Играть следующим» (`next`) или «Добавить в конец очереди».
    Enqueue { track: Track, next: bool },
    /// Переключатель «Треки 18+».
    AllowExplicit(bool),
}

enum Msg {
    Cmd(Cmd),
    Account(Res<Account>),
    Likes(Res<Vec<String>>),
    Batch { res: Res<Batch>, replace: bool, gen: u64 },
    Audio { gen: u64, res: Res<Audio> },
    Prefetch { id: String, res: Res<Audio> },
    Error(String),
}

type Notify = Arc<dyn Fn() + Send + Sync>;
type Job = Box<dyn FnOnce() + Send>;
pub type Likes = Arc<Mutex<HashSet<String>>>;

#[derive(Clone)]
pub struct Player {
    tx: Sender<Msg>,
    shared: Arc<Mutex<Shared>>,
    likes: Likes,
    pub eq: Arc<EqParams>,
}

impl Player {
    pub fn spawn(notify: Notify) -> Player {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared { volume: 0.6, ..Default::default() }));
        let likes: Likes = Default::default();
        let eq: Arc<EqParams> = Default::default();
        let p = Player { tx: tx.clone(), shared: shared.clone(), likes: likes.clone(), eq: eq.clone() };
        thread::Builder::new()
            .name("audio".into())
            .spawn(move || Engine::new(tx, shared, likes, notify, eq).run(rx))
            .expect("audio thread");
        p
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(Msg::Cmd(c));
    }

    pub fn state(&self) -> Shared {
        self.shared.lock().unwrap().clone()
    }

    pub fn is_liked(&self, id: &str) -> bool {
        self.likes.lock().unwrap().contains(id)
    }

}

/// Сколько живёт сессия волны без прослушивания: оригинал начинает новую через 9 часов.
const WAVE_SESSION_TTL: u64 = 9 * 3600;

/// `from` «Моей волны» у оригинала, когда её запускают с главной.
const WAVE_FROM: &str = "desktop-wave_landing_screen-my_wave-radio-default";

struct WaveState {
    uid: String,
    session: String,
    /// Последний трек волны: с ним сессия клонируется после перезапуска.
    last: String,
    at: u64,
}

fn load_wave_state() -> Option<WaveState> {
    let text = std::fs::read_to_string(crate::config::dir().join("wave.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let str_at = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    Some(WaveState {
        uid: str_at("uid"),
        session: str_at("session"),
        last: str_at("last"),
        at: v.get("at").and_then(|x| x.as_u64()).unwrap_or(0),
    })
}

/// Где звучит трек — для `/plays`, в тех же полях, что у оригинала.
#[derive(Clone)]
struct PlayContext {
    from: String,
    context: &'static str,
    item: Option<String>,
    playlist: Option<String>,
    /// Сессия волны; пустая у списков.
    session: String,
}

/// Одно прослушивание трека: сколько его реально слушали, была ли пауза и перемотка.
/// Уходит в `/plays`, когда трек сменился.
struct Listen {
    track: Track,
    ctx: PlayContext,
    play_id: String,
    add_time: String,
    start: String,
    played: f64,
    since: Option<Instant>,
    pause: bool,
    seek: bool,
    repeated: bool,
}

impl Listen {
    fn played(&self) -> f64 {
        self.played + self.since.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0)
    }

    fn stop(&mut self) {
        if let Some(s) = self.since.take() {
            self.played += s.elapsed().as_secs_f64();
        }
    }

    fn go(&mut self) {
        self.since.get_or_insert_with(Instant::now);
    }
}

/// Случайные 128 бит без отдельной зависимости: sha256 от времени, счётчика и pid.
fn random_u128() -> u128 {
    use sha2::{Digest, Sha256};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let h = Sha256::digest(format!("{t}:{n}:{}", std::process::id()));
    u128::from_le_bytes(h[..16].try_into().unwrap())
}

/// uuid v4, как `playId` у оригинала.
fn uuid4() -> String {
    let mut b = random_u128().to_be_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Трек, который качается, и декодер, уже прочитавший его начало.
type Audio = (Arc<Buf>, Decoder<Reader>);

/// Имя выхода Windows по умолчанию. На Linux звук идёт в «default», и за сменой устройства
/// там следит PipeWire или PulseAudio.
#[cfg(windows)]
fn default_output_name() -> Option<String> {
    use rodio::cpal::traits::HostTrait;
    use rodio::DeviceTrait;
    rodio::cpal::default_host().default_output_device()?.name().ok()
}

/// Многоканальные выходы Windows (виртуальные 7.1 у гарнитур) искажают f32, а i32 играют чисто: cpal#892.
#[cfg(windows)]
fn open_surround() -> Option<(OutputStream, rodio::OutputStreamHandle)> {
    use rodio::cpal::traits::HostTrait;
    use rodio::cpal::SampleFormat;
    use rodio::DeviceTrait;
    let dev = rodio::cpal::default_host().default_output_device()?;
    let def = dev.default_output_config().ok()?;
    if def.channels() <= 2 || def.sample_format() == SampleFormat::I32 {
        return None;
    }
    let rate = def.sample_rate();
    let cfg = dev
        .supported_output_configs()
        .ok()?
        .find(|c| c.sample_format() == SampleFormat::I32 && c.channels() == def.channels() && c.min_sample_rate() <= rate && rate <= c.max_sample_rate())?
        .with_sample_rate(rate);
    let out = OutputStream::try_from_device_config(&dev, cfg).ok()?;
    log(format!("аудиовыход: {} каналов, i32", def.channels()));
    Some(out)
}

/// Только выход по умолчанию: `try_default` при неудаче молча берёт любое другое устройство,
/// и звук уходит, например, в неподключённый HDMI (#53).
fn open_output() -> Result<(OutputStream, rodio::OutputStreamHandle), rodio::StreamError> {
    use rodio::cpal::traits::HostTrait;
    #[cfg(windows)]
    if let Some(out) = open_surround() {
        return Ok(out);
    }
    let dev = rodio::cpal::default_host().default_output_device().ok_or(rodio::StreamError::NoDevice)?;
    OutputStream::try_from_device(&dev)
}

#[cfg(windows)]
const NO_OUTPUT: &str = "Не найдено аудиоустройство";
#[cfg(not(windows))]
const NO_OUTPUT: &str = "Не открылся звуковой выход по умолчанию. Для PipeWire нужен pipewire-alsa";

/// Сколько позиция может стоять на месте у играющего скачанного трека.
const STALL_LIMIT: Duration = Duration::from_secs(3);

fn load_track(api: &Api, t: &Track) -> Res<Audio> {
    let url = api.stream_url(&t.id)?;
    let buf = Buf::download(api.open(&url)?)?;
    let dec = Decoder::new(buf.reader()).map_err(|e| format!("не удалось декодировать: {e}"))?;
    Ok((buf, dec))
}

/// Перемешивание без лишних зависимостей (xorshift от времени).
pub fn shuffle<T>(v: &mut [T]) {
    let mut x = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

enum End {
    Finished,
    Skip,
    Dislike,
    /// Волну запустили заново: пропуск, но очередь уже свежая.
    Restart,
}

struct ListPlay {
    deferred: VecDeque<Track>,
    all: Vec<Track>,
    after: After,
}

struct Engine {
    tx: Sender<Msg>,
    bg: Sender<Job>,
    shared: Arc<Mutex<Shared>>,
    likes: Likes,
    notify: Notify,
    eq: Arc<EqParams>,
    audio: Option<(OutputStream, Sink)>,
    /// Выход Windows по умолчанию, на котором открыт звук, и когда его последний раз сверяли.
    #[cfg(windows)]
    out_name: Option<String>,
    #[cfg(windows)]
    out_checked: Instant,
    /// Выход пропал посреди трека: с этого места трек продолжится по Play.
    held_pos: Option<f32>,
    /// Позиция играющего трека и с какого момента она не меняется.
    stall: (f32, Instant),
    api: Option<Api>,
    uid: String,
    source: Source,
    list: Option<ListPlay>,
    queue: VecDeque<Track>,
    history: Vec<Track>,
    current: Option<Track>,
    current_buf: Option<Arc<Buf>>,
    batch_id: String,
    /// Сессия ротора текущей волны; radioStarted уходит в неё, когда она появится.
    session: String,
    radio_started_pending: bool,
    /// Пропуски и дослушанные треки, которые уйдут со следующим запросом треков волны.
    deferred: Vec<serde_json::Value>,
    /// Станция, для которой уже проверена сохранённая сессия «Моей волны».
    played_station: String,
    /// Сессия «Моей волны» с прошлого запуска уже подхвачена (или не нашлась).
    wave_restored: bool,
    /// Сессия с прошлого запуска: первый пакет берётся её клоном, как у оригинала.
    clone_pending: bool,
    /// Последний трек волны с прошлого запуска — queue для клона.
    restore_last: Option<String>,
    /// Текущее прослушивание для `/plays`.
    listen: Option<Listen>,
    fetching: bool,
    batch_gen: u64,
    retry_at: Option<Instant>,
    gen: u64,
    loaded: bool,
    paused: bool,
    volume: f32,
    loudness: Loudness,
    repeat: Repeat,
    shuffle: bool,
    prefetch: Option<(String, Audio)>,
    prefetching: Option<String>,
    errors_in_row: u32,
    was_playing: bool,
}

impl Engine {
    fn new(tx: Sender<Msg>, shared: Arc<Mutex<Shared>>, likes: Likes, notify: Notify, eq: Arc<EqParams>) -> Self {
        // Фидбек и лайки — по порядку в отдельном потоке, чтобы не блокировать плеер.
        let (bg, bg_rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("bg".into())
            .spawn(move || {
                for job in bg_rx {
                    job();
                }
            })
            .expect("bg thread");
        Engine {
            tx,
            bg,
            shared,
            likes,
            notify,
            eq,
            audio: None,
            #[cfg(windows)]
            out_name: None,
            #[cfg(windows)]
            out_checked: Instant::now(),
            held_pos: None,
            stall: (0.0, Instant::now()),
            api: None,
            uid: String::new(),
            source: Source::Wave,
            list: None,
            queue: VecDeque::new(),
            history: Vec::new(),
            current: None,
            current_buf: None,
            batch_id: String::new(),
            session: String::new(),
            radio_started_pending: false,
            deferred: Vec::new(),
            played_station: String::new(),
            wave_restored: false,
            clone_pending: false,
            restore_last: None,
            listen: None,
            fetching: false,
            batch_gen: 0,
            retry_at: None,
            gen: 0,
            loaded: false,
            paused: true,
            volume: 0.6,
            loudness: Loudness::Square,
            repeat: Repeat::Off,
            shuffle: false,
            prefetch: None,
            prefetching: None,
            errors_in_row: 0,
            was_playing: false,
        }
    }

    fn run(mut self, rx: Receiver<Msg>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(msg) => {
                    self.handle(msg);
                    self.sync();
                    (self.notify)();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
        }
    }

    // ---------- вспомогательное ----------

    fn sh(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap()
    }

    fn sink(&self) -> Option<&Sink> {
        self.audio.as_ref().map(|(_, s)| s)
    }

    fn ensure_audio(&mut self) -> bool {
        if self.audio.is_none() {
            match open_output() {
                Ok((stream, handle)) => match Sink::try_new(&handle) {
                    Ok(sink) => {
                        self.audio = Some((stream, sink));
                        #[cfg(windows)]
                        {
                            self.out_name = default_output_name();
                        }
                    }
                    Err(e) => log(format!("sink: {e}")),
                },
                Err(e) => log(format!("audio device: {e}")),
            }
        }
        self.audio.is_some()
    }

    fn gain(&self) -> f32 {
        let v = self.volume;
        match self.loudness {
            Loudness::Square => v * v,
            Loudness::Decibels if v <= 0.0 => 0.0,
            Loudness::Decibels => 10f32.powf((v - 1.0) * 50.0 / 20.0),
        }
    }

    fn pos(&self) -> f32 {
        match self.sink() {
            Some(s) => s.get_pos().as_secs_f32(),
            None => self.held_pos.unwrap_or(0.0),
        }
    }

    /// Звук открыт на одном устройстве и сам за выходом Windows не переходит. Раз в секунду
    /// сверяем выход по умолчанию: сменился или пропал — переоткрываем.
    #[cfg(windows)]
    fn check_output(&mut self) {
        if self.audio.is_none() || self.out_checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.out_checked = Instant::now();
        let now = default_output_name();
        if now == self.out_name {
            return;
        }
        log(format!("аудиовыход: {} → {}", self.out_name.as_deref().unwrap_or("нет"), now.as_deref().unwrap_or("нет")));
        let pos = self.pos();
        self.reopen_output(pos, now.is_some());
    }

    /// Трек играет и скачан целиком, а позиция стоит: выход умер молча. На Linux так бывает
    /// после блокировки экрана (#49).
    fn check_stall(&mut self) {
        let pos = self.pos();
        let playing = self.loaded
            && !self.paused
            && self.current_buf.as_ref().is_some_and(|b| b.done())
            && self.sink().is_some_and(|s| !s.empty());
        if !playing || pos != self.stall.0 {
            self.stall = (pos, Instant::now());
            return;
        }
        if self.stall.1.elapsed() < STALL_LIMIT {
            return;
        }
        log(format!("звук встал на {pos:.1} с, выход переоткрывается"));
        self.stall = (pos, Instant::now());
        self.reopen_output(pos, true);
    }

    /// Закрыть выход и продолжить трек с `pos` на новом. Не вышло — пауза до Play с того же места.
    fn reopen_output(&mut self, pos: f32, available: bool) {
        self.audio = None;
        #[cfg(windows)]
        {
            self.out_name = None;
        }
        if !self.loaded {
            return;
        }
        if !available || !self.restore_at(pos) {
            self.held_pos = Some(pos);
            if !self.paused {
                self.paused = true;
                if let Some(l) = &mut self.listen {
                    l.stop();
                    l.pause = true;
                }
            }
            (self.notify)();
        }
    }

    /// Открыть выход заново и продолжить текущий трек из скачанного буфера с места `pos`.
    fn restore_at(&mut self, pos: f32) -> bool {
        let Some(buf) = self.current_buf.clone() else { return false };
        if !self.ensure_audio() {
            return false;
        }
        let Ok(src) = Decoder::new(buf.reader()) else { return false };
        let src = Ahead::new(Eq::new(src.convert_samples::<f32>(), self.eq.clone()));
        let (vol, paused) = (self.gain(), self.paused);
        let Some(sink) = self.sink() else { return false };
        sink.pause();
        sink.set_volume(vol);
        sink.append(src);
        if let Err(e) = sink.try_seek(Duration::from_secs_f32(pos)) {
            log(format!("seek: {e:?}"));
        }
        if !paused {
            sink.play();
        }
        self.held_pos = None;
        true
    }

    fn error(&self, e: impl Into<String>) {
        let e = e.into();
        log(format!("error: {e}"));
        self.sh().error = Some((e, Instant::now()));
        (self.notify)();
    }

    fn feedback(&mut self, kind: &'static str, track: Option<&Track>, played: Option<f64>) {
        self.send_feedback(kind, track, played, true);
    }

    /// `from` волны, как у оригинала: «Моя волна» с главной или волна по исполнителю, альбому…
    fn rotor_from(&self) -> String {
        match &self.source {
            Source::Seed { station, .. } => format!("desktop-radio-{}-default", station.replace(':', "-")),
            _ => WAVE_FROM.to_string(),
        }
    }

    /// Фидбек волне. Пропуск, дизлайк и «дослушал» при `defer` ждут следующего запроса треков
    /// и уходят вместе с ним, как у оригинала; «назад» он отправляет сразу.
    fn send_feedback(&mut self, kind: &'static str, track: Option<&Track>, played: Option<f64>, defer: bool) {
        let Some(station) = self.source.station().map(String::from) else { return };
        let Some(api) = self.api.clone() else { return };
        if self.session.is_empty() && kind == "radioStarted" {
            self.radio_started_pending = true;
            return;
        }
        // Фидбек идёт с пакетом самого трека: следующий пакет часто приходит, пока доигрывает прошлый.
        let batch = track
            .map(|t| t.batch_id.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| self.batch_id.clone());
        let length = track
            .filter(|t| kind == "trackFinished" && t.duration_ms > 0)
            .map(|t| t.duration_ms as f64 / 1000.0);
        let track = track.map(|t| t.full_id());
        let session = self.session.clone();
        let from = self.rotor_from();
        if session.is_empty() {
            let _ = self.bg.send(Box::new(move || {
                if let Err(e) = api.feedback(&station, kind, track.as_deref(), played, &batch, &from) {
                    log(format!("feedback {kind}: {e}"));
                }
            }));
            return;
        }
        let body = crate::api::session_event(kind, track.as_deref(), played, length, &batch, &from);
        if defer && matches!(kind, "skip" | "dislike" | "trackFinished") {
            self.deferred.push(body);
            return;
        }
        let _ = self.bg.send(Box::new(move || {
            if let Err(e) = api.session_feedback(&session, &body) {
                log(format!("feedback {kind}: {e}"));
            }
        }));
    }

    /// Отложенные фидбеки, которые не ушли с запросом треков, отправляются отдельно.
    fn flush_feedback(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        let list = std::mem::take(&mut self.deferred);
        let (Some(api), false) = (self.api.clone(), self.session.is_empty()) else { return };
        let session = self.session.clone();
        let _ = self.bg.send(Box::new(move || {
            for body in list {
                if let Err(e) = api.session_feedback(&session, &body) {
                    log(format!("feedback: {e}"));
                }
            }
        }));
    }

    /// Контекст `/plays` для трека, который сейчас заиграл.
    fn play_context(&self) -> PlayContext {
        let radio = |from: String, item: &str| PlayContext {
            from,
            context: "radio",
            item: Some(item.to_string()),
            playlist: None,
            session: self.session.clone(),
        };
        let list = |from: &str, context: &'static str, item: Option<String>, playlist: Option<String>| PlayContext {
            from: from.to_string(),
            context,
            item,
            playlist,
            session: String::new(),
        };
        match &self.source {
            Source::Wave => radio(WAVE_FROM.to_string(), WAVE),
            Source::Seed { station, .. } => radio(self.rotor_from(), station),
            Source::List { kind, id, .. } => match kind {
                ListKind::Album => list("desktop-album-album-default", "album", Some(id.clone()), None),
                ListKind::Artist => list("desktop-artist-artist-default", "artist", Some(id.clone()), None),
                ListKind::Playlist => list("desktop-playlist-playlist-default", "playlist", Some(id.clone()), Some(id.clone())),
                ListKind::Likes => {
                    let likes = format!("{}:3", self.uid);
                    list("desktop-playlist-playlist-default", "playlist", Some(likes.clone()), Some(likes))
                }
                ListKind::Tracks if id == "search" => list("desktop-search-search-default", "search", None, None),
                ListKind::Tracks => list("desktop-other-other-default", "other", None, None),
            },
        }
    }

    /// Трек стал текущим: новое прослушивание. Время слушания идёт, только когда трек звучит.
    fn begin_listen(&mut self, track: &Track, repeated: bool) {
        let ctx = self.play_context();
        let radio = ctx.context == "radio";
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        self.listen = Some(Listen {
            track: track.clone(),
            ctx,
            play_id: uuid4(),
            add_time: format!("{:016}-{}", random_u128() % 10_000_000_000_000_000, ms.div_ceil(1000)),
            start: crate::api::iso_now(),
            played: 0.0,
            since: (self.loaded && !self.paused).then(Instant::now),
            pause: false,
            // В волне оригинал всегда шлёт seek: при старте трека плеер ставит его на начало.
            seek: radio,
            repeated,
        });
    }

    /// Трек сменился: фидбек волне (если нужен) и запись в `/plays`, как у оригинала.
    /// Второй раз для того же прослушивания ничего не шлёт.
    fn end_current(&mut self, fb: Option<&'static str>, defer: bool, reason: &'static str) {
        let Some(mut l) = self.listen.take() else { return };
        l.stop();
        let len = l.track.duration_ms as f64 / 1000.0;
        let near = |x: f64| if (len - x).abs() < 0.5 { len } else { round3(x) };
        let played = near(l.played());
        let end = if reason == "finish" { len } else { near(self.pos() as f64) };
        if let Some(kind) = fb {
            let t = l.track.clone();
            self.send_feedback(kind, Some(&t), Some(played), defer);
        }
        let (Some(api), false) = (self.api.clone(), self.uid.is_empty()) else { return };
        let t = &l.track;
        let mut play = serde_json::json!({
            "playId": l.play_id,
            "from": l.ctx.from,
            "totalPlayedSeconds": played,
            "endPositionSeconds": end,
            "trackLengthSeconds": len,
            "timestamp": crate::api::iso_now(),
            "context": l.ctx.context,
            "contextItem": l.ctx.item,
            "addTracksToPlayerTime": l.add_time,
            "fromCache": false,
            "isRestored": false,
            "audioAuto": "none",
            "audioOutputName": "Динамик",
            "audioOutputType": "Speaker",
            "trackId": t.id,
            "seek": l.seek,
            "pause": l.pause,
            "startTimestamp": l.start,
            "maxPlayerStage": "play",
            "isRepeated": l.repeated,
            "changeReason": reason,
            "isLivePlayableIndex": l.ctx.context == "radio",
        });
        if let Some(a) = &t.album_id {
            play["albumId"] = a.clone().into();
        }
        if let Some(p) = &l.ctx.playlist {
            play["playlistId"] = p.clone().into();
        }
        if l.ctx.context == "radio" {
            let batch = if t.batch_id.is_empty() { self.batch_id.clone() } else { t.batch_id.clone() };
            play["radioSessionId"] = l.ctx.session.clone().into();
            play["batchId"] = batch.into();
            play["isFromAutoflow"] = false.into();
            play["isFromPumpkin"] = false.into();
        }
        let uid = self.uid.clone();
        let _ = self.bg.send(Box::new(move || {
            if let Err(e) = api.plays(&uid, &play) {
                log(format!("plays: {e}"));
            }
        }));
    }

    /// queue для следующего пакета, как у оригинала: трек, на котором сменился трек, и тот,
    /// что заиграл после него. Всю историю оригинал туда не кладёт.
    fn rotor_queue(&self) -> Vec<String> {
        let mut q: Vec<String> = self.history.last().filter(|t| !t.batch_id.is_empty()).map(|t| t.full_id()).into_iter().collect();
        if let Some(c) = &self.current {
            q.push(c.full_id());
        }
        q
    }

    fn stop_audio(&mut self) {
        if let Some(s) = self.sink() {
            s.clear();
        }
        self.loaded = false;
        self.held_pos = None;
    }

    fn clear_queue(&mut self) {
        self.flush_feedback();
        self.queue.clear();
        self.prefetch = None;
        self.prefetching = None;
        self.fetching = false;
        self.batch_gen += 1;
        self.retry_at = None;
        self.batch_id.clear();
        self.session.clear();
    }

    fn reset(&mut self) {
        self.stop_audio();
        self.gen += 1;
        self.clear_queue();
        self.history.clear();
        self.listen = None;
        self.played_station.clear();
        self.wave_restored = false;
        self.clone_pending = false;
        self.restore_last = None;
        self.current = None;
        self.current_buf = None;
        self.list = None;
        self.source = Source::Wave;
        let mut s = self.sh();
        s.track = None;
        s.source = Source::Wave;
        s.loading = false;
    }

    fn set_source(&mut self, src: Source) {
        self.source = src.clone();
        self.sh().source = src;
    }

    /// Состояние движка → Shared для UI.
    fn sync(&mut self) {
        let playing = self.loaded && !self.paused && self.sink().map(|s| !s.is_paused()).unwrap_or(false);
        let pos = if self.loaded { self.pos() } else { 0.0 };
        let upcoming: Vec<Track> = self.queue.iter().take(30).cloned().collect();
        let mut s = self.shared.lock().unwrap();
        s.playing = playing;
        s.position = pos;
        s.volume = self.volume;
        s.repeat = self.repeat;
        s.shuffle = self.shuffle;
        s.upcoming = upcoming;
    }

    // ---------- главный цикл ----------

    fn tick(&mut self) {
        #[cfg(windows)]
        self.check_output();
        self.check_stall();
        if let Some(at) = self.retry_at {
            if Instant::now() >= at {
                self.retry_at = None;
                self.fetch_batch();
            }
        }
        if self.loaded && !self.paused && self.sink().map(|s| s.empty()).unwrap_or(false) {
            if self.repeat == Repeat::One && self.current_buf.is_some() {
                if let Some(l) = &mut self.listen {
                    l.repeated = true;
                }
                self.end_current(None, true, "finish");
                if self.replay() {
                    if let Some(t) = self.current.clone() {
                        self.begin_listen(&t, false);
                    }
                    return;
                }
            }
            self.advance(End::Finished);
            self.sync();
            (self.notify)();
            return;
        }
        self.sync();
        if self.loaded && self.prefetch.is_none() && self.prefetching.is_none() {
            self.prefetch_next();
        }
        let playing = self.sh().playing;
        if playing != self.was_playing {
            self.was_playing = playing;
            (self.notify)();
        }
    }

    fn replay(&mut self) -> bool {
        let Some(buf) = self.current_buf.clone() else { return false };
        let Ok(src) = Decoder::new(buf.reader()) else { return false };
        let src = Ahead::new(Eq::new(src.convert_samples::<f32>(), self.eq.clone()));
        if let Some(sink) = self.sink() {
            sink.append(src);
            sink.play();
        }
        true
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Cmd(c) => self.cmd(c),
            Msg::Account(res) => match res {
                Ok(acc) => {
                    self.uid = acc.uid.clone();
                    {
                        let mut s = self.sh();
                        s.status = Status::Ready;
                        s.uid = acc.uid.clone();
                        s.login = acc.login;
                        s.has_plus = acc.has_plus;
                        s.error = None;
                    }
                    self.feedback("radioStarted", None, None);
                    self.fetch_batch();
                    if let Some(api) = self.api.clone() {
                        let tx = self.tx.clone();
                        let uid = acc.uid;
                        thread::spawn(move || {
                            let _ = tx.send(Msg::Likes(api.liked_ids(&uid)));
                        });
                    }
                }
                Err(e) => {
                    log(format!("account: {e}"));
                    self.api = None;
                    self.sh().status = Status::Failed(e);
                }
            },
            Msg::Likes(res) => match res {
                Ok(ids) => self.likes.lock().unwrap().extend(ids),
                Err(e) => log(format!("likes: {e}")),
            },
            Msg::Batch { res, replace, gen } => {
                if gen != self.batch_gen {
                    return;
                }
                self.fetching = false;
                match res {
                    Ok(b) => self.on_batch(b, replace),
                    Err(e) => {
                        self.error(format!("Не удалось получить треки волны: {e}"));
                        self.retry_at = Some(Instant::now() + Duration::from_secs(5));
                    }
                }
            }
            Msg::Audio { gen, res } => {
                if gen == self.gen {
                    match res {
                        Ok(b) => self.play_loaded(b),
                        Err(e) => self.on_load_error(e),
                    }
                }
            }
            Msg::Prefetch { id, res } => {
                if self.prefetching.as_deref() == Some(id.as_str()) {
                    self.prefetching = None;
                }
                let waiting = !self.loaded && self.current.as_ref().map(|t| t.id == id).unwrap_or(false);
                match res {
                    Ok(b) if waiting => self.play_loaded(b),
                    Ok(b) => self.prefetch = Some((id, b)),
                    Err(e) => {
                        log(format!("prefetch {id}: {e}"));
                        if waiting {
                            self.start_download();
                        }
                    }
                }
            }
            Msg::Error(e) => self.error(e),
        }
    }

    fn cmd(&mut self, c: Cmd) {
        match c {
            Cmd::Login(token) => {
                self.reset();
                let api = Api::new(&token);
                self.api = Some(api.clone());
                {
                    let mut s = self.sh();
                    s.status = Status::Connecting;
                    s.error = None;
                }
                let tx = self.tx.clone();
                thread::spawn(move || {
                    let _ = tx.send(Msg::Account(api.account()));
                });
            }
            Cmd::Logout => {
                self.reset();
                self.api = None;
                self.uid.clear();
                self.likes.lock().unwrap().clear();
                let mut s = self.sh();
                s.status = Status::LoggedOut;
                s.login.clear();
                s.uid.clear();
            }
            Cmd::Resume => self.resume(),
            Cmd::PlayPause if self.paused => self.resume(),
            Cmd::PlayPause => {
                self.paused = true;
                if let Some(l) = &mut self.listen {
                    l.stop();
                    l.pause = true;
                }
                if let Some(sink) = self.sink() { sink.pause(); }
            }
            Cmd::Next => {
                self.errors_in_row = 0;
                if self.current.is_some() {
                    self.advance(End::Skip);
                }
            }
            Cmd::Prev => self.prev(),
            Cmd::Seek(sec) => {
                if let Some(l) = &mut self.listen {
                    l.seek = true;
                }
                if self.loaded {
                    if let Some(s) = self.sink() {
                        if let Err(e) = s.try_seek(Duration::from_secs_f32(sec.max(0.0))) {
                            log(format!("seek: {e:?}"));
                        }
                    } else if self.held_pos.is_some() {
                        self.held_pos = Some(sec.max(0.0));
                    }
                }
            }
            Cmd::Volume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                if let Some(s) = self.sink() {
                    s.set_volume(self.gain());
                }
            }
            Cmd::Loudness(l) => {
                self.loudness = l;
                if let Some(s) = self.sink() {
                    s.set_volume(self.gain());
                }
            }
            Cmd::LikeTrack(t) => self.toggle_like(t),
            Cmd::Dislike => {
                if let (Some(api), Some(t)) = (self.api.clone(), self.current.as_ref()) {
                    let id = t.full_id();
                    let uid = self.uid.clone();
                    let tx = self.tx.clone();
                    let _ = self.bg.send(Box::new(move || {
                        if let Err(e) = api.dislike(&uid, &id) {
                            let _ = tx.send(Msg::Error(format!("Дизлайк не сохранился: {e}")));
                        }
                    }));
                }
                self.advance(End::Dislike);
            }
            Cmd::Wave(src) => self.start_wave(src, true),
            Cmd::PlayList { source, tracks, start, after } => {
                if tracks.is_empty() {
                    return;
                }
                // Играть нечего: текущий трек не трогаем, а говорим, почему ничего не включилось.
                if !crate::api::allow_explicit() && tracks.iter().all(|t| t.explicit) {
                    self.error("Здесь только треки 18+. Их можно включить в меню профиля");
                    return;
                }
                self.end_current(Some("skip"), true, "other_track");
                self.clear_queue();
                let start = start.min(tracks.len() - 1);
                let mut order: Vec<Track> = tracks.clone();
                order.rotate_left(start);
                let first = order.remove(0);
                if self.shuffle {
                    shuffle(&mut order);
                }
                self.queue.push_back(first);
                self.queue.extend(order);
                self.list = Some(ListPlay { deferred: VecDeque::new(), all: tracks, after });
                self.set_source(source);
                self.advance(End::Skip);
            }
            Cmd::Jump(i) => {
                if i >= self.queue.len() {
                    return;
                }
                let skipped: Vec<Track> = self.queue.drain(..i).collect();
                self.end_current(Some("skip"), true, "other_track");
                if let Some(cur) = self.current.take() {
                    self.history.push(cur);
                }
                self.history.extend(skipped);
                self.trim_history();
                self.stop_audio();
                self.play_from_queue();
                self.refresh_wave();
                self.flush_feedback();
            }
            Cmd::Repeat(r) => self.repeat = r,
            Cmd::Enqueue { track, next } => {
                if next {
                    self.queue.push_front(track);
                    self.prefetch = None;
                    if self.loaded {
                        self.prefetch_next();
                    }
                } else {
                    self.queue.push_back(track);
                }
            }
            Cmd::AllowExplicit(allow) => self.set_allow_explicit(allow),
            Cmd::Shuffle(on) => {
                self.shuffle = on;
                if self.list.is_some() {
                    let mut v: Vec<Track> = self.queue.drain(..).collect();
                    if on {
                        shuffle(&mut v);
                    } else if let (Some(l), Some(cur)) = (&self.list, &self.current) {
                        let i = l.all.iter().position(|t| t.id == cur.id).map(|i| i + 1).unwrap_or(0);
                        v = l.all[i.min(l.all.len())..].to_vec();
                    }
                    self.queue.extend(v);
                    self.prefetch = None;
                    if self.loaded {
                        self.prefetch_next();
                    }
                }
            }
        }
    }

    fn toggle_like(&mut self, t: Track) {
        let Some(api) = self.api.clone() else { return };
        let remove = {
            let mut l = self.likes.lock().unwrap();
            let had = l.remove(&t.id);
            if !had {
                l.insert(t.id.clone());
            }
            had
        };
        let (uid, id, tx) = (self.uid.clone(), t.full_id(), self.tx.clone());
        let _ = self.bg.send(Box::new(move || {
            if let Err(e) = api.like(&uid, &id, remove) {
                let _ = tx.send(Msg::Error(format!("Лайк не сохранился: {e}")));
            }
        }));
        // Лайк трека из волны оригинал сообщает и самой волне, чтобы она его учла.
        if self.session.is_empty() {
            return;
        }
        let in_wave = self
            .current
            .iter()
            .chain(self.queue.iter())
            .chain(self.history.iter())
            .find(|w| w.id == t.id && !w.batch_id.is_empty())
            .cloned();
        if let Some(w) = in_wave {
            self.feedback(if remove { "unlike" } else { "like" }, Some(&w), None);
        }
    }

    // ---------- очередь ----------

    fn start_wave(&mut self, src: Source, fetch: bool) {
        // Пропуск уходит ещё в старую волну: clear_queue отправит его туда.
        self.end_current(Some("skip"), true, "other_track");
        self.clear_queue();
        self.list = None;
        self.set_source(src);
        self.feedback("radioStarted", None, None);
        if fetch {
            self.fetch_batch_with(true);
        }
    }

    fn fetch_batch(&mut self) {
        self.fetch_batch_with(false);
    }

    fn fetch_batch_with(&mut self, replace: bool) {
        if self.fetching || self.list.is_some() {
            return;
        }
        let Some(station) = self.source.station().map(String::from) else { return };
        let Some(api) = self.api.clone() else { return };
        self.fetching = true;
        self.retry_at = None;
        if self.played_station != station {
            self.played_station = station.clone();
            if station == WAVE {
                self.restore_wave();
            }
        }
        let (session, queue) = if self.session.is_empty() {
            (WaveSession::New, if replace { Vec::new() } else { self.rotor_queue() })
        } else if self.clone_pending {
            (WaveSession::Clone(self.session.clone()), self.restore_last.clone().into_iter().collect())
        } else {
            (WaveSession::Next(self.session.clone()), self.rotor_queue())
        };
        let feedbacks = std::mem::take(&mut self.deferred);
        let tx = self.tx.clone();
        let gen = self.batch_gen;
        // В очереди фонового потока: запрос уйдёт после уже отправленных фидбеков.
        let _ = self.bg.send(Box::new(move || {
            let res = api.wave_session(&station, session, &queue, &feedbacks);
            let _ = tx.send(Msg::Batch { res, replace, gen });
        }));
    }

    fn on_batch(&mut self, b: Batch, replace: bool) {
        self.batch_id = b.batch_id;
        self.clone_pending = false;
        self.restore_last = None;
        if !b.session.is_empty() {
            // Сессия могла подхватиться с прошлого запуска — radioStarted нужен и ей.
            let changed = b.session != self.session;
            self.session = b.session;
            if std::mem::take(&mut self.radio_started_pending) {
                self.feedback("radioStarted", None, None);
            }
            if changed {
                self.save_wave();
            }
        }
        self.likes.lock().unwrap().extend(b.liked);
        if replace {
            self.queue.clear();
            self.prefetch = None;
        }
        // Пакет берётся как есть, как у оригинала: сервер сам решает, что повторить.
        // Убираем только то, что уже стоит в очереди или играет, и 18+, если их выключили.
        let queued = |t: &Track| {
            self.queue.iter().any(|q| q.id == t.id) || (!replace && self.current.as_ref().is_some_and(|c| c.id == t.id))
        };
        let no_explicit = !crate::api::allow_explicit();
        let tracks: Vec<Track> = b.tracks.iter().filter(|t| !queued(t) && !(no_explicit && t.explicit)).cloned().collect();
        self.queue.extend(tracks);
        if replace {
            self.advance(End::Restart);
        } else if self.current.is_none() {
            self.play_from_queue();
        } else if self.loaded {
            self.prefetch_next();
        }
    }

    fn trim_history(&mut self) {
        if self.history.len() > 50 {
            let n = self.history.len() - 50;
            self.history.drain(..n);
        }
    }

    /// Смена трека. Фидбек «дослушал» и «пропуск» ждёт следующего запроса треков и уходит
    /// вместе с ним, как у оригинала.
    fn advance(&mut self, end: End) {
        let (kind, reason) = match end {
            End::Finished => ("trackFinished", "finish"),
            End::Skip => ("skip", "skip"),
            End::Dislike => ("dislike", "dislike"),
            End::Restart => ("skip", "other_track"),
        };
        self.end_current(Some(kind), true, reason);
        if let Some(cur) = self.current.take() {
            self.history.push(cur);
            self.trim_history();
        }
        self.stop_audio();
        self.play_from_queue();
        if matches!(end, End::Skip | End::Dislike) {
            self.refresh_wave();
        }
    }

    /// Пропуск в волне, как у оригинала: заиграл следующий трек, остальные несыгранные треки
    /// волны убираются и сразу просятся новые, а пропуск уходит в том же запросе. Так волна
    /// подстраивается сразу, а не через пакет. Треки, добавленные вручную, остаются.
    fn refresh_wave(&mut self) {
        if self.list.is_some() || self.source.station().is_none() || self.session.is_empty() {
            return;
        }
        self.queue.retain(|t| t.batch_id.is_empty());
        if let Some((id, _)) = &self.prefetch {
            if !self.queue.iter().any(|t| t.id == *id) {
                self.prefetch = None;
            }
        }
        self.fetch_batch();
    }

    /// Список закончился: по кругу (исполнитель) или волна по нему (плейлист, альбом).
    fn list_finished(&mut self) {
        let Some(l) = self.list.take() else { return };
        match l.after {
            After::Loop => {
                let mut all = l.all.clone();
                if self.shuffle {
                    shuffle(&mut all);
                }
                self.queue.extend(all);
                self.list = Some(l);
            }
            After::Wave(src) => {
                self.flush_feedback();
                self.set_source(src);
                self.session.clear();
                self.feedback("radioStarted", None, None);
            }
        }
    }

    /// Как у оригинала при смене настроек волны: текущий трек доигрывает (если он не 18+, а их
    /// выключили), а несыгранные треки волны заменяет новая сессия уже с этим флагом.
    fn set_allow_explicit(&mut self, allow: bool) {
        crate::api::ALLOW_EXPLICIT.store(allow, std::sync::atomic::Ordering::Relaxed);
        let wave = self.list.is_none() && self.source.station().is_some();
        let skip_current = !allow && self.current.as_ref().is_some_and(|t| t.explicit);
        if wave {
            self.queue.retain(|t| t.batch_id.is_empty());
        }
        if !allow {
            if self.blocked_list() {
                if skip_current {
                    self.queue.push_front(self.current.as_ref().unwrap().clone());
                }
                self.defer_explicit();
            } else {
                self.queue.retain(|t| !t.explicit);
            }
        }
        self.prefetch = None;
        if skip_current {
            self.end_current(Some("skip"), true, "skip");
            if let Some(cur) = self.current.take() {
                self.history.push(cur);
                self.trim_history();
            }
            self.stop_audio();
        }
        if wave {
            self.flush_feedback();
            self.session.clear();
            self.fetching = false;
            self.batch_gen += 1;
        }
        if skip_current {
            self.play_from_queue();
        }
        if wave {
            self.fetch_batch();
        }
    }

    fn blocked_list(&self) -> bool {
        !crate::api::allow_explicit()
            && self.list.as_ref().is_some_and(|list| !list.all.is_empty() && list.all.iter().all(|t| t.explicit))
    }

    fn defer_explicit(&mut self) {
        if let Some(list) = &mut self.list {
            self.queue.retain(|track| {
                if track.explicit { list.deferred.push_back(track.clone()); }
                !track.explicit
            });
        }
    }

    fn resume(&mut self) {
        self.paused = false;
        if let Some(pos) = self.held_pos.filter(|_| self.loaded && self.audio.is_none()) {
            if !self.restore_at(pos) {
                self.paused = true;
                self.error(NO_OUTPUT);
                return;
            }
        }
        let loaded = self.loaded;
        if let Some(l) = self.listen.as_mut().filter(|_| loaded) {
            l.go();
        }
        if self.current.is_none() {
            if self.list.is_some() {
                self.play_from_queue();
                return;
            }
            if self.sh().status == Status::Ready { self.fetch_batch(); }
        }
        if let Some(sink) = self.sink() { sink.play(); }
    }

    fn play_from_queue(&mut self) {
        let blocked_list = self.blocked_list();
        if blocked_list {
            self.defer_explicit();
        } else if crate::api::allow_explicit() {
            if let Some(list) = &mut self.list { self.queue.append(&mut list.deferred); }
        } else {
            self.queue.retain(|t| !t.explicit);
        }
        if self.queue.is_empty() && self.list.is_some() && !blocked_list {
            match (&self.list, self.repeat) {
                (Some(l), Repeat::All) => {
                    let all = l.all.clone();
                    self.queue.extend(all);
                }
                _ => self.list_finished(),
            }
        }
        if !crate::api::allow_explicit() {
            self.queue.retain(|t| !t.explicit);
        }
        match self.queue.pop_front() {
            Some(t) => {
                self.current = Some(t);
                self.start_current();
            }
            None => {
                self.current = None;
                self.current_buf = None;
                if self.list.is_some() { self.paused = true; }
                self.gen += 1;
                let mut s = self.sh();
                s.track = None;
                s.loading = self.list.is_none() && self.source.station().is_some();
            }
        }
        // Как у оригинала: новый пакет волны нужен, когда заиграл последний трек очереди.
        if self.queue.is_empty() {
            self.fetch_batch();
        }
    }

    fn prev(&mut self) {
        if self.loaded && self.pos() > 4.0 {
            if let Some(s) = self.sink() {
                let _ = s.try_seek(Duration::ZERO);
            }
            return;
        }
        if !crate::api::allow_explicit() {
            self.history.retain(|t| !t.explicit);
        }
        if let Some(p) = self.history.pop() {
            self.end_current(Some("skip"), false, "back_skip");
            if let Some(c) = self.current.take() {
                self.queue.push_front(c);
            }
            self.stop_audio();
            self.current = Some(p);
            self.start_current();
        } else if self.loaded {
            if let Some(s) = self.sink() {
                let _ = s.try_seek(Duration::ZERO);
            }
        }
    }

    // ---------- загрузка ----------

    fn start_current(&mut self) {
        self.gen += 1;
        self.loaded = false;
        self.listen = None;
        self.current_buf = None;
        let Some(t) = self.current.clone() else { return };
        // Как у оригинала, трек начат сразу, а не когда докачался: иначе пропуск до загрузки
        // не доходит до волны, и она отдаёт этот трек снова.
        self.begin_listen(&t, false);
        self.feedback("trackStarted", Some(&t), None);
        {
            let mut s = self.sh();
            s.track = Some(t.clone());
            s.position = 0.0;
            s.duration = t.duration_ms as f32 / 1000.0;
            s.loading = true;
        }
        if self.prefetch.as_ref().map(|(id, _)| *id == t.id).unwrap_or(false) {
            let (_, b) = self.prefetch.take().unwrap();
            self.play_loaded(b);
            return;
        }
        if self.prefetching.as_deref() == Some(t.id.as_str()) {
            return;
        }
        self.start_download();
    }

    fn start_download(&mut self) {
        let (Some(api), Some(t)) = (self.api.clone(), self.current.clone()) else { return };
        let gen = self.gen;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let res = load_track(&api, &t);
            let _ = tx.send(Msg::Audio { gen, res });
        });
    }

    fn prefetch_next(&mut self) {
        // Следующий трек качаем под конец текущего: не держать в памяти два файла всё время.
        let near_end = {
            let s = self.shared.lock().unwrap();
            s.duration > 0.0 && s.duration - s.position <= 45.0
        };
        if !near_end {
            return;
        }
        let Some(next) = self.queue.front().cloned() else { return };
        let have = self.prefetch.as_ref().map(|(id, _)| *id == next.id).unwrap_or(false);
        if have || self.prefetching.is_some() {
            return;
        }
        let Some(api) = self.api.clone() else { return };
        self.prefetching = Some(next.id.clone());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let res = load_track(&api, &next);
            let _ = tx.send(Msg::Prefetch { id: next.id, res });
        });
    }

    fn play_loaded(&mut self, (buf, src): Audio) {
        let Some(t) = self.current.clone() else { return };
        if !self.ensure_audio() {
            self.error("Не найдено аудиоустройство");
            self.sh().loading = false;
            return;
        }
        let src = Ahead::new(Eq::new(src.convert_samples::<f32>(), self.eq.clone()));
        let vol = self.gain();
        let paused = self.paused;
        if let Some(sink) = self.sink() {
            sink.clear();
            sink.set_volume(vol);
            sink.append(src);
            if paused {
                sink.pause()
            } else {
                sink.play()
            }
        }
        self.current_buf = Some(buf);
        self.loaded = true;
        self.held_pos = None;
        self.errors_in_row = 0;
        self.sh().loading = false;
        if let Some(l) = self.listen.as_mut().filter(|_| !paused) {
            l.go();
        }
        self.remember_wave(&t);
        self.prefetch_next();
    }

    // ---------- память волны ----------

    fn remember_wave(&mut self, t: &Track) {
        if self.source.station() == Some(self.played_station.as_str()) && !t.batch_id.is_empty() {
            self.save_wave_with(Some(t.full_id()));
        }
    }

    /// «Моя волна» после перезапуска продолжается, как у оригинала: сервер клонирует прошлую
    /// сессию вместе с её историей. Иначе волна начинает с чистого листа.
    fn restore_wave(&mut self) {
        let first = !std::mem::replace(&mut self.wave_restored, true);
        let Some(w) = load_wave_state() else { return };
        if w.uid != self.uid {
            return;
        }
        // Сессию берём только при запуске; если сервер её уже закрыл, wave_session начнёт новую.
        let fresh = crate::config::now_secs().saturating_sub(w.at) < WAVE_SESSION_TTL;
        if first && fresh && self.session.is_empty() && !w.session.is_empty() {
            self.session = w.session;
            self.clone_pending = true;
            self.restore_last = Some(w.last).filter(|l| !l.is_empty());
        }
    }

    fn save_wave(&self) {
        let last = self.current.as_ref().filter(|t| !t.batch_id.is_empty()).map(|t| t.full_id());
        self.save_wave_with(last);
    }

    fn save_wave_with(&self, last: Option<String>) {
        if self.played_station != WAVE || self.source.station() != Some(WAVE) || self.uid.is_empty() {
            return;
        }
        let w = serde_json::json!({
            "uid": self.uid,
            "session": self.session,
            "last": last.unwrap_or_default(),
            "at": crate::config::now_secs(),
        });
        let _ = self.bg.send(Box::new(move || {
            let path = crate::config::dir().join("wave.json");
            if let Err(e) = std::fs::write(&path, w.to_string()) {
                log(format!("wave.json: {e}"));
            }
        }));
    }

    fn on_load_error(&mut self, e: String) {
        self.errors_in_row += 1;
        let name = self.current.as_ref().map(|t| t.title.clone()).unwrap_or_default();
        self.error(format!("«{name}» не загрузился: {e}"));
        self.listen = None;
        if self.errors_in_row < 5 {
            self.current = None;
            self.stop_audio();
            self.play_from_queue();
        } else {
            self.sh().loading = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Сетевой тест: ссылка на поток → скачивание → декодирование (без токена — 30-сек превью).
    #[test]
    #[ignore]
    fn stream_and_decode() {
        let api = Api::new("");
        let url = api.stream_url("57703").expect("stream url");
        let bytes = api.fetch_bytes(&url).expect("download");
        assert!(bytes.len() > 50_000);
        let dec = Decoder::new(Cursor::new(bytes)).expect("decode");
        let samples = dec.take(44100).count();
        assert!(samples > 1000);
    }
}

#[cfg(test)]
#[test]
fn explicit_filter_survives_repeat_and_previous() {
    let mut engine = Engine::new(mpsc::channel().0, Default::default(), Default::default(), Arc::new(|| {}), Default::default());
    let blocked = crate::api::parse_track(&serde_json::json!({"id":1,"title":"E","explicit":true})).unwrap();
    let allowed = crate::api::parse_track(&serde_json::json!({"id":2,"title":"A"})).unwrap();
    let previous = crate::api::ALLOW_EXPLICIT.swap(false, std::sync::atomic::Ordering::Relaxed);
    engine.list = Some(ListPlay { deferred: VecDeque::new(), all: vec![blocked.clone(), allowed.clone()], after: After::Loop });
    engine.repeat = Repeat::All;
    engine.play_from_queue();
    let repeated = engine.current.take().map(|t| t.id);
    engine.history = vec![allowed, blocked];
    engine.prev();
    let previous_track = engine.current.take().unwrap().id;
    let explicit = engine.list.as_ref().unwrap().all[0].clone();
    engine.list.as_mut().unwrap().all = vec![explicit.clone()];
    engine.list.as_mut().unwrap().after = After::Wave(Source::Wave);
    engine.repeat = Repeat::Off;
    engine.queue = VecDeque::from([explicit]);
    engine.play_from_queue();
    engine.set_allow_explicit(true);
    engine.cmd(Cmd::PlayPause);
    let resumed = engine.current.take().map(|t| t.id);
    // Список только из 18+ при выключенном фильтре не включается и не трогает текущий трек.
    crate::api::ALLOW_EXPLICIT.store(false, std::sync::atomic::Ordering::Relaxed);
    let kept = engine.list.as_ref().unwrap().all[0].clone();
    let mut other = kept.clone();
    other.id = "3".into();
    engine.current = Some(kept.clone());
    engine.cmd(Cmd::PlayList { source: Source::Wave, tracks: vec![other, kept], start: 1, after: After::Loop });
    let still = engine.current.as_ref().map(|t| t.id.clone());
    let told = engine.sh().error.is_some();
    crate::api::ALLOW_EXPLICIT.store(previous, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(still.as_deref(), Some("1"));
    assert!(told);
    assert_eq!(resumed.as_deref(), Some("1"));
    assert_eq!(repeated.as_deref(), Some("2"));
    assert_eq!(previous_track, "2");
}
