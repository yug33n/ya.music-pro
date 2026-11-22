//! Обложки: загрузка и декодирование в фоне, кэш в UI-потоке с бюджетом по байтам.
//! Последний запрос обслуживается первым — при прокрутке видимые обложки приходят раньше.

use crate::config::log;
use reqwest::blocking::Client;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Бюджет кэша и размер, до которого он ужимается после ухода со страницы.
/// В лёгком режиме вдвое меньше: обложки чаще качаются заново, зато памяти меньше.
const BUDGET: [usize; 2] = [16 << 20, 8 << 20];
const TRIMMED: [usize; 2] = [10 << 20, 4 << 20];
const WORKERS: usize = 4;

/// (адрес, размер, можно отменить при уходе со страницы)
type Queue = Arc<(Mutex<Vec<(String, u32, bool)>>, Condvar)>;
type Waiter = Box<dyn FnOnce(&Image)>;

struct Entry {
    image: Image,
    bytes: usize,
    tick: u64,
}

#[derive(Default)]
struct Inner {
    cache: HashMap<String, Entry>,
    bytes: usize,
    tick: u64,
    waiting: HashMap<String, Vec<Waiter>>,
}

#[derive(Clone)]
pub struct Images {
    inner: Rc<RefCell<Inner>>,
    queue: Queue,
    lite: Rc<Cell<bool>>,
}

thread_local! {
    static CURRENT: RefCell<Option<Images>> = const { RefCell::new(None) };
}

fn decode(bytes: &[u8], px: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let mut img = image::load_from_memory(bytes).ok()?.to_rgba8();
    if img.width() > px || img.height() > px {
        let (w, h) = if img.width() >= img.height() {
            (px, (px as u64 * img.height() as u64 / img.width() as u64).max(1) as u32)
        } else {
            ((px as u64 * img.width() as u64 / img.height() as u64).max(1) as u32, px)
        };
        img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    }
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

fn worker(http: Client, queue: Queue) {
    loop {
        let (url, px) = {
            let (lock, cv) = &*queue;
            let mut q = lock.lock().unwrap();
            while q.is_empty() {
                q = cv.wait(q).unwrap();
            }
            let (url, px, _) = q.pop().unwrap();
            (url, px)
        };
        let src = url.rsplit_once('#').map_or(url.as_str(), |(u, _)| u);
        let buf = http
            .get(src)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.bytes())
            .map_err(|e| log(format!("обложка {url}: {}", crate::config::chain(&e))))
            .ok()
            .and_then(|b| decode(&b, px));
        let _ = slint::invoke_from_event_loop(move || {
            CURRENT.with(|c| {
                if let Some(images) = c.borrow().as_ref() {
                    images.done(url, buf);
                }
            })
        });
    }
}

impl Images {
    pub fn new(lite: bool) -> Images {
        let http = Client::builder().timeout(Duration::from_secs(30)).build().expect("http client");
        let queue: Queue = Default::default();
        for i in 0..WORKERS {
            let (http, queue) = (http.clone(), queue.clone());
            std::thread::Builder::new().name(format!("img{i}")).spawn(move || worker(http, queue)).expect("img thread");
        }
        let images = Images { inner: Default::default(), queue, lite: Rc::new(Cell::new(lite)) };
        CURRENT.with(|c| *c.borrow_mut() = Some(images.clone()));
        images
    }

    /// `px` — сторона в физических пикселях. Колбэк вызывается сразу, если картинка уже в кэше.
    pub fn get(&self, url: &str, px: u32, f: impl FnOnce(&Image) + 'static) {
        self.request(url, px, false, f)
    }

    /// То же, но запрос снимается `cancel_pending`, если до него не дошла очередь.
    pub fn get_cancellable(&self, url: &str, px: u32, f: impl FnOnce(&Image) + 'static) {
        self.request(url, px, true, f)
    }

    fn request(&self, url: &str, px: u32, cancellable: bool, f: impl FnOnce(&Image) + 'static) {
        if url.trim_start_matches("https://").is_empty() {
            return;
        }
        // Один адрес в разных размерах — разные записи: картинка декодируется точно под место.
        let key = format!("{url}#{px}");
        let url = key.as_str();
        let mut inner = self.inner.borrow_mut();
        inner.tick += 1;
        let tick = inner.tick;
        if let Some(e) = inner.cache.get_mut(url) {
            e.tick = tick;
            let image = e.image.clone();
            drop(inner);
            f(&image);
            return;
        }
        let first = !inner.waiting.contains_key(url);
        inner.waiting.entry(url.to_string()).or_default().push(Box::new(f));
        drop(inner);
        if first {
            let (lock, cv) = &*self.queue;
            lock.lock().unwrap().push((url.to_string(), px, cancellable));
            cv.notify_one();
        }
    }

    /// Убрать из очереди отменяемые запросы, которые ещё не начали качать (ушли со страницы).
    pub fn cancel_pending(&self) {
        let (lock, _) = &*self.queue;
        let mut q = lock.lock().unwrap();
        let dropped: Vec<String> = q.iter().filter(|e| e.2).map(|e| e.0.clone()).collect();
        q.retain(|e| !e.2);
        drop(q);
        let mut inner = self.inner.borrow_mut();
        for u in dropped {
            inner.waiting.remove(&u);
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.cache.clear();
        inner.bytes = 0;
    }

    /// Ушли со страницы: выкинуть самые старые обложки сверх `TRIMMED`.
    pub fn trim(&self) {
        let mut inner = self.inner.borrow_mut();
        Self::evict(&mut inner, TRIMMED[self.lite.get() as usize]);
    }

    /// Сменился режим работы: в лёгком кэш сразу ужимается до его бюджета.
    pub fn set_lite(&self, lite: bool) {
        self.lite.set(lite);
        let mut inner = self.inner.borrow_mut();
        Self::evict(&mut inner, BUDGET[lite as usize]);
    }

    fn evict(inner: &mut Inner, limit: usize) {
        while inner.bytes > limit && inner.cache.len() > 1 {
            let oldest = inner.cache.iter().min_by_key(|(_, e)| e.tick).map(|(k, _)| k.clone()).unwrap();
            let e = inner.cache.remove(&oldest).unwrap();
            inner.bytes -= e.bytes;
        }
    }

    fn done(&self, url: String, buf: Option<SharedPixelBuffer<Rgba8Pixel>>) {
        let mut inner = self.inner.borrow_mut();
        let waiters = inner.waiting.remove(&url).unwrap_or_default();
        let Some(buf) = buf else { return };
        let bytes = buf.as_bytes().len();
        let image = Image::from_rgba8(buf);
        inner.tick += 1;
        let tick = inner.tick;
        inner.bytes += bytes;
        inner.cache.insert(url, Entry { image: image.clone(), bytes, tick });
        Self::evict(&mut inner, BUDGET[self.lite.get() as usize]);
        drop(inner);
        for w in waiters {
            w(&image);
        }
    }
}
