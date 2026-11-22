//! Эквалайзер из «Настроек звука»: предусиление и 10 пиковых фильтров ±12 дБ.

use rodio::source::SeekError;
use rodio::Source;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const FREQS: [f32; 10] = [60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];

/// Пресеты в порядке выпадающего списка. Первый — «Своя настройка».
pub const PRESETS: [(&str, [f32; 10]); 19] = [
    ("Своя настройка", [0.0; 10]),
    ("По умолчанию", [0.0; 10]),
    ("Классическая музыка", [-0.5, -0.5, -0.5, -0.5, -0.5, -0.5, -3.5, -3.5, -3.5, -4.5]),
    ("Клубная музыка", [-0.5, -0.5, 4.0, 2.5, 2.5, 2.5, 1.5, -0.5, -0.5, -0.5]),
    ("Танцевальная музыка", [4.5, 3.5, 1.0, -0.5, -0.5, -2.5, -3.5, -3.5, -0.5, -0.5]),
    ("Усиление НЧ", [4.0, 4.5, 4.5, 2.5, 0.5, -2.0, -4.0, -5.0, -5.5, -5.5]),
    ("Усиление НЧ и ВЧ", [3.5, 2.5, -0.5, -3.5, -2.0, 0.5, 4.0, 5.5, 6.0, 6.0]),
    ("Усиление ВЧ", [-4.5, -4.5, -4.5, -2.0, 1.0, 5.5, 8.0, 8.0, 8.0, 8.0]),
    ("Колонки ноутбука", [2.0, 5.5, 2.5, -1.5, -1.0, 0.5, 2.0, 4.5, 6.0, 7.0]),
    ("Большой зал", [5.0, 5.0, 2.5, 2.5, -0.5, -2.0, -2.0, -2.0, -0.5, -0.5]),
    ("Концерт", [-2.0, -0.5, 2.0, 2.5, 2.5, 2.5, 2.0, 1.0, 1.0, 1.0]),
    ("Вечеринка", [3.5, 3.5, -0.5, -0.5, -0.5, -0.5, -0.5, -0.5, 3.5, 3.5]),
    ("Поп", [-0.5, 2.0, 3.5, 4.0, 2.5, -0.5, -1.0, -1.0, -0.5, -0.5]),
    ("Регги", [-0.5, -0.5, -0.5, -2.5, -0.5, 3.0, 3.0, -0.5, -0.5, -0.5]),
    ("Рок", [4.0, 2.0, -2.5, -4.0, -1.5, 2.0, 4.0, 5.5, 5.5, 5.5]),
    ("Ска", [-1.0, -2.0, -2.0, -0.5, 2.0, 2.5, 4.0, 4.5, 5.5, 4.5]),
    ("Мягкое звучание", [2.0, 0.5, -0.5, -1.0, -0.5, 2.0, 4.0, 4.5, 5.5, 6.0]),
    ("Софт-рок", [2.0, 2.0, 1.0, -0.5, -2.0, -2.5, -1.5, -0.5, 1.0, 4.0]),
    ("Техно", [4.0, 2.5, -0.5, -2.5, -2.0, -0.5, 4.0, 4.5, 4.5, 4.0]),
];

/// Настройки, общие для UI и аудиопотока.
pub struct EqParams {
    on: AtomicBool,
    preamp: AtomicU32,
    gains: [AtomicU32; 10],
    version: AtomicU64,
}

impl Default for EqParams {
    fn default() -> Self {
        EqParams {
            on: AtomicBool::new(false),
            preamp: AtomicU32::new(1.0f32.to_bits()),
            gains: Default::default(),
            version: AtomicU64::new(1),
        }
    }
}

impl EqParams {
    /// `preamp` — множитель громкости 0,5…1,5, `gains` — дБ по полосам.
    pub fn set(&self, on: bool, preamp: f32, gains: &[f32; 10]) {
        self.on.store(on, Ordering::Relaxed);
        self.preamp.store(preamp.to_bits(), Ordering::Relaxed);
        for (a, g) in self.gains.iter().zip(gains) {
            a.store(g.to_bits(), Ordering::Relaxed);
        }
        self.version.fetch_add(1, Ordering::Release);
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    /// Пиковый фильтр по RBJ Audio EQ Cookbook, добротность 1,4 — примерно октава.
    fn peaking(freq: f32, gain_db: f32, rate: f32) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (freq.min(rate * 0.45)) / rate;
        let alpha = w0.sin() / (2.0 * 1.4);
        let cos = w0.cos();
        let a0 = 1.0 + alpha / a;
        Biquad {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

pub struct Eq<S> {
    inner: S,
    params: Arc<EqParams>,
    version: u64,
    on: bool,
    preamp: f32,
    bands: Vec<(usize, Biquad)>,
    /// Состояние фильтров: [канал][полоса] = (x1, x2, y1, y2).
    state: Vec<[[f32; 4]; 10]>,
    ch: usize,
    count: u32,
}

impl<S: Source<Item = f32>> Eq<S> {
    pub fn new(inner: S, params: Arc<EqParams>) -> Self {
        let mut eq = Eq { inner, params, version: 0, on: false, preamp: 1.0, bands: Vec::new(), state: Vec::new(), ch: 0, count: 0 };
        eq.refresh();
        eq
    }

    fn refresh(&mut self) {
        let v = self.params.version.load(Ordering::Acquire);
        if v == self.version {
            return;
        }
        self.version = v;
        self.on = self.params.on.load(Ordering::Relaxed);
        self.preamp = f32::from_bits(self.params.preamp.load(Ordering::Relaxed));
        let rate = self.inner.sample_rate().max(8000) as f32;
        self.bands = (0..10)
            .filter_map(|i| {
                let g = f32::from_bits(self.params.gains[i].load(Ordering::Relaxed));
                (g.abs() > 0.01).then(|| (i, Biquad::peaking(FREQS[i], g, rate)))
            })
            .collect();
        let channels = self.inner.channels().max(1) as usize;
        if self.state.len() != channels {
            self.state = vec![[[0.0; 4]; 10]; channels];
        }
    }
}

impl<S: Source<Item = f32>> Iterator for Eq<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let x = self.inner.next()?;
        self.count += 1;
        if self.count >= 2048 && self.ch == 0 {
            self.count = 0;
            self.refresh();
        }
        let channels = self.state.len().max(1);
        let ch = self.ch % channels;
        self.ch = (self.ch + 1) % channels;
        if !self.on {
            return Some(x);
        }
        let mut y = x * self.preamp;
        let st = &mut self.state[ch];
        for &(i, f) in &self.bands {
            let s = &mut st[i];
            let out = f.b0 * y + f.b1 * s[0] + f.b2 * s[1] - f.a1 * s[2] - f.a2 * s[3];
            s[1] = s[0];
            s[0] = y;
            s[3] = s[2];
            s[2] = out;
            y = out;
        }
        Some(y.clamp(-1.0, 1.0))
    }
}

impl<S: Source<Item = f32>> Source for Eq<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.state.fill([[0.0; 4]; 10]);
        self.ch = 0;
        self.inner.try_seek(pos)
    }
}
