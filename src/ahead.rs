//! Звук декодируется заранее в своём потоке, а аудиопоток только забирает готовые сэмплы:
//! когда процессор занят, декодер и эквалайзер не успевали внутри колбэка, и звук хрипел (#47).

use rodio::source::SeekError;
use rodio::Source;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, Thread};
use std::time::Duration;

/// Сэмплов в одной порции.
const CHUNK: usize = 1024;
/// Сколько звука держать готовым. Больше — дольше доходят правки эквалайзера.
const AHEAD: Duration = Duration::from_millis(250);

struct Shared {
    /// Растёт при каждой перемотке: порции со старым номером выбрасываются.
    gen: AtomicU64,
    closed: AtomicBool,
}

pub struct Ahead<S> {
    inner: Arc<Mutex<S>>,
    shared: Arc<Shared>,
    rx: Receiver<(u64, Vec<f32>)>,
    worker: Thread,
    chunk: Vec<f32>,
    at: usize,
    channels: u16,
    rate: u32,
    total: Option<Duration>,
}

impl<S: Source<Item = f32> + Send + 'static> Ahead<S> {
    pub fn new(inner: S) -> Self {
        let (channels, rate, total) = (inner.channels(), inner.sample_rate(), inner.total_duration());
        let per_sec = rate as usize * channels.max(1) as usize;
        let (tx, rx) = sync_channel((per_sec * AHEAD.as_millis() as usize / 1000 / CHUNK).max(2));
        let inner = Arc::new(Mutex::new(inner));
        let shared = Arc::new(Shared { gen: AtomicU64::new(0), closed: AtomicBool::new(false) });
        let (src, sh) = (inner.clone(), shared.clone());
        let worker = thread::Builder::new()
            .name("decode".into())
            .spawn(move || loop {
                let (gen, chunk) = {
                    let mut s = src.lock().unwrap();
                    let s: &mut S = &mut s;
                    (sh.gen.load(Ordering::SeqCst), s.take(CHUNK).collect::<Vec<f32>>())
                };
                let end = chunk.is_empty();
                if tx.send((gen, chunk)).is_err() {
                    return;
                }
                // Конец трека: ждём перемотку назад или закрытия.
                while end && sh.gen.load(Ordering::SeqCst) == gen {
                    if sh.closed.load(Ordering::SeqCst) {
                        return;
                    }
                    thread::park_timeout(Duration::from_millis(200));
                }
            })
            .expect("decode thread")
            .thread()
            .clone();
        Ahead { inner, shared, rx, worker, chunk: Vec::new(), at: 0, channels, rate, total }
    }
}

impl<S> Drop for Ahead<S> {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        self.worker.unpark();
    }
}

impl<S> Iterator for Ahead<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(&x) = self.chunk.get(self.at) {
                self.at += 1;
                return Some(x);
            }
            let (gen, chunk) = self.rx.recv().ok()?;
            if gen != self.shared.gen.load(Ordering::SeqCst) {
                continue;
            }
            if chunk.is_empty() {
                return None;
            }
            self.chunk = chunk;
            self.at = 0;
        }
    }
}

impl<S: Source<Item = f32>> Source for Ahead<S> {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let res = {
            let mut s = self.inner.lock().unwrap();
            let res = s.try_seek(pos);
            self.shared.gen.fetch_add(1, Ordering::SeqCst);
            res
        };
        self.chunk.clear();
        self.at = 0;
        self.worker.unpark();
        res
    }
}
