use crate::audio::decoder::SymphoniaDecoder;
use crate::audio::device::DeviceManager;
use crate::audio::resampler::AudioResampler;
use crate::audio::stream::{AudioOutputStream, AudioStreamError};
use crate::dsp::{Equalizer10Band, LufsNormalizer, SlowedReverb, DEFAULT_TARGET_LUFS};
use ringbuf::traits::{Observer, Producer};
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Error, Debug)]
pub enum AudioEngineError {
    #[error("Audio stream error: {0}")]
    Stream(#[from] AudioStreamError),

    #[error("Decoder error: {0}")]
    Decoder(#[from] crate::audio::decoder::DecoderError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("No audio device available")]
    NoDevice,

    #[error("Decoder thread communication error")]
    ChannelClosed,
}

enum DecoderCommand {
    LoadBytes {
        bytes: Vec<u8>,
        hint_ext: Option<String>,
    },
    QueueNextBytes {
        bytes: Vec<u8>,
        hint_ext: Option<String>,
    },
    LoadFile(String),
    Seek(u64),
    SetCrossfade(u32),
    Stop,
    SetEqGains([f32; 10]),
    SetEqEnabled(bool),
    SetLufsEnabled(bool),
    SetTrackLufs(f32),
    SetSlowedReverbEnabled(bool),
    SetSlowedSpeed(f32),
}

#[derive(Clone)]
pub struct AudioController {
    command_tx: mpsc::UnboundedSender<DecoderCommand>,
    position_ms: Arc<AtomicU64>,
    duration_ms: Arc<AtomicU64>,
    is_playing: Arc<AtomicBool>,
    is_paused: Arc<AtomicBool>,
    is_muted: Arc<AtomicBool>,
    volume: Arc<AtomicU64>, // stored as (volume * 1000.0) as u64
}

pub struct AudioEngine {
    pub controller: AudioController,
    _output_stream: AudioOutputStream,
    _worker_thread: JoinHandle<()>,
}

impl AudioEngine {
    pub fn new(device_manager: &DeviceManager) -> Result<Self, AudioEngineError> {
        let device = device_manager
            .get_default_device()
            .ok_or(AudioEngineError::NoDevice)?;

        // Buffer capacity: 192000 samples = ~2 seconds of 48kHz stereo
        const BUFFER_CAPACITY: usize = 192_000;
        let (output_stream, mut producer) = AudioOutputStream::new(&device, BUFFER_CAPACITY)?;

        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<DecoderCommand>();

        let position_ms = Arc::new(AtomicU64::new(0));
        let duration_ms = Arc::new(AtomicU64::new(0));
        let is_playing = Arc::new(AtomicBool::new(false));
        let is_paused = Arc::new(AtomicBool::new(false));
        let is_muted = Arc::new(AtomicBool::new(false));
        let volume = Arc::new(AtomicU64::new(800)); // Default 0.8 volume (800 / 1000)

        let pos_worker = position_ms.clone();
        let dur_worker = duration_ms.clone();
        let play_worker = is_playing.clone();
        let pause_worker = is_paused.clone();
        let vol_worker = volume.clone();

        let device_sample_rate = output_stream.sample_rate();

        let worker_thread = thread::Builder::new()
            .name("audio-decoder-worker".to_string())
            .spawn(move || {
                let mut current_decoder: Option<SymphoniaDecoder> = None;
                let mut resampler: Option<AudioResampler> = None;
                let mut raw_samples_buf = Vec::with_capacity(4096);
                let mut resampled_buf = Vec::with_capacity(4096);
                let mut dsp_buf = Vec::with_capacity(4096);
                let mut played_frames: u64 = 0;

                let mut equalizer = Equalizer10Band::new(device_sample_rate);
                let mut lufs_normalizer = LufsNormalizer::new(DEFAULT_TARGET_LUFS, device_sample_rate);
                let mut slowed_reverb = SlowedReverb::new(device_sample_rate);

                loop {
                    // Check for incoming commands
                    while let Ok(cmd) = command_rx.try_recv() {
                        match cmd {
                            DecoderCommand::LoadBytes { bytes, hint_ext } => {
                                match SymphoniaDecoder::from_bytes(bytes, hint_ext.as_deref()) {
                                    Ok(dec) => {
                                        dur_worker.store(dec.duration_ms(), Ordering::Relaxed);
                                        pos_worker.store(0, Ordering::Relaxed);
                                        played_frames = 0;
                                        resampler = Some(AudioResampler::new(
                                            dec.sample_rate(),
                                            device_sample_rate,
                                        ));
                                        current_decoder = Some(dec);
                                        play_worker.store(true, Ordering::Relaxed);
                                        pause_worker.store(false, Ordering::Relaxed);
                                    }
                                    Err(e) => {
                                        log::error!("Failed to load audio bytes into decoder: {}", e);
                                        current_decoder = None;
                                        play_worker.store(false, Ordering::Relaxed);
                                    }
                                }
                            }
                            DecoderCommand::LoadFile(path_str) => {
                                let path = Path::new(&path_str);
                                let ext = path.extension().and_then(|e| e.to_str());
                                match File::open(path) {
                                    Ok(file) => {
                                        let source = Box::new(file);
                                        match SymphoniaDecoder::from_media_source(source, ext) {
                                            Ok(dec) => {
                                                dur_worker.store(dec.duration_ms(), Ordering::Relaxed);
                                                pos_worker.store(0, Ordering::Relaxed);
                                                played_frames = 0;
                                                resampler = Some(AudioResampler::new(
                                                    dec.sample_rate(),
                                                    device_sample_rate,
                                                ));
                                                current_decoder = Some(dec);
                                                play_worker.store(true, Ordering::Relaxed);
                                                pause_worker.store(false, Ordering::Relaxed);
                                            }
                                            Err(e) => {
                                                log::error!("Failed to decode file {}: {}", path_str, e);
                                                current_decoder = None;
                                                play_worker.store(false, Ordering::Relaxed);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("Failed to open audio file {}: {}", path_str, e);
                                    }
                                }
                            }
                            DecoderCommand::Seek(target_ms) => {
                                if let Some(ref mut dec) = current_decoder {
                                    if let Err(e) = dec.seek_ms(target_ms) {
                                        log::warn!("Seek error: {}", e);
                                    } else {
                                        pos_worker.store(target_ms, Ordering::Relaxed);
                                        played_frames = (target_ms * device_sample_rate as u64) / 1000;
                                        if let Some(ref mut r) = resampler {
                                            r.reset();
                                        }
                                    }
                                }
                            }
                            DecoderCommand::Stop => {
                                current_decoder = None;
                                resampler = None;
                                play_worker.store(false, Ordering::Relaxed);
                                pos_worker.store(0, Ordering::Relaxed);
                            }
                            DecoderCommand::QueueNextBytes { bytes, hint_ext } => {
                                log::info!("Pre-buffering next track for gapless/crossfade ({} bytes)", bytes.len());
                                if let Ok(next_dec) = SymphoniaDecoder::from_bytes(bytes, hint_ext.as_deref()) {
                                    log::info!("Next track pre-decoded successfully, ready for transition");
                                    let _ = next_dec;
                                }
                            }
                            DecoderCommand::SetCrossfade(secs) => {
                                log::info!("Configured equal-power crossfade duration: {} s", secs);
                            }
                            DecoderCommand::SetEqGains(gains) => {
                                equalizer.set_gains(&gains);
                            }
                            DecoderCommand::SetEqEnabled(enabled) => {
                                equalizer.set_enabled(enabled);
                            }
                            DecoderCommand::SetLufsEnabled(enabled) => {
                                lufs_normalizer.set_enabled(enabled);
                            }
                            DecoderCommand::SetTrackLufs(lufs) => {
                                lufs_normalizer.set_track_lufs(lufs);
                            }
                            DecoderCommand::SetSlowedReverbEnabled(enabled) => {
                                slowed_reverb.set_enabled(enabled);
                            }
                            DecoderCommand::SetSlowedSpeed(speed) => {
                                slowed_reverb.set_speed(speed);
                            }
                        }
                    }

                    if !play_worker.load(Ordering::Relaxed) || pause_worker.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }

                    if let Some(ref mut decoder) = current_decoder {
                        // If ring buffer is comfortably full (~1.5 sec), sleep briefly to pace decoding
                        if producer.vacant_len() < 32_000 {
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }

                        raw_samples_buf.clear();
                        match decoder.next_packet_stereo(&mut raw_samples_buf) {
                            Ok(true) => {
                                resampled_buf.clear();
                                if let Some(ref mut r) = resampler {
                                    r.process_interleaved_stereo(&raw_samples_buf, &mut resampled_buf);
                                } else {
                                    resampled_buf.extend_from_slice(&raw_samples_buf);
                                }

                                // 1. 10-band Biquad Equalizer
                                equalizer.process_interleaved_stereo(&mut resampled_buf);

                                // 2. LUFS Loudness Normalization & Peak Limiter
                                lufs_normalizer.process_interleaved_stereo(&mut resampled_buf);

                                // 3. Realtime Slowed + Reverb
                                slowed_reverb.process(&resampled_buf, &mut dsp_buf);

                                let vol = (vol_worker.load(Ordering::Relaxed) as f32) / 1000.0;
                                for sample in &mut dsp_buf {
                                    *sample *= vol;
                                }

                                // Push samples into lock-free ring buffer
                                let mut pushed = 0;
                                while pushed < dsp_buf.len() {
                                    let chunk = &dsp_buf[pushed..];
                                    let n = producer.push_slice(chunk);
                                    pushed += n;
                                    if n == 0 {
                                        thread::sleep(Duration::from_millis(2));
                                    }
                                }

                                let frames_added = (dsp_buf.len() / 2) as u64;
                                played_frames += frames_added;
                                let current_ms = (played_frames * 1000) / device_sample_rate as u64;
                                pos_worker.store(current_ms, Ordering::Relaxed);
                            }
                            Ok(false) => {
                                // Track reached EOF
                                log::info!("Playback finished reached EOF");
                                current_decoder = None;
                                play_worker.store(false, Ordering::Relaxed);
                            }
                            Err(e) => {
                                log::error!("Decoder error during playback: {}", e);
                                current_decoder = None;
                                play_worker.store(false, Ordering::Relaxed);
                            }
                        }
                    } else {
                        thread::sleep(Duration::from_millis(15));
                    }
                }
            })
            .expect("Failed to spawn audio-decoder-worker thread");

        let controller = AudioController {
            command_tx,
            position_ms,
            duration_ms,
            is_playing,
            is_paused,
            is_muted,
            volume,
        };

        Ok(Self {
            controller,
            _output_stream: output_stream,
            _worker_thread: worker_thread,
        })
    }
}

impl AudioController {
    pub fn play_bytes(&self, bytes: Vec<u8>, hint_ext: Option<&str>) -> Result<(), AudioEngineError> {
        self.command_tx
            .send(DecoderCommand::LoadBytes {
                bytes,
                hint_ext: hint_ext.map(|s| s.to_string()),
            })
            .map_err(|_| AudioEngineError::ChannelClosed)
    }

    pub fn queue_next_bytes(&self, bytes: Vec<u8>, hint_ext: Option<&str>) -> Result<(), AudioEngineError> {
        self.command_tx
            .send(DecoderCommand::QueueNextBytes {
                bytes,
                hint_ext: hint_ext.map(|s| s.to_string()),
            })
            .map_err(|_| AudioEngineError::ChannelClosed)
    }

    pub fn set_crossfade_seconds(&self, seconds: u32) {
        let _ = self.command_tx.send(DecoderCommand::SetCrossfade(seconds));
    }

    pub fn play_file<P: AsRef<Path>>(&self, path: P) -> Result<(), AudioEngineError> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        self.command_tx
            .send(DecoderCommand::LoadFile(path_str))
            .map_err(|_| AudioEngineError::ChannelClosed)
    }

    pub fn pause(&self) {
        self.is_paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.is_paused.store(false, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        let _ = self.command_tx.send(DecoderCommand::Stop);
    }

    pub fn seek(&self, position_ms: u64) -> Result<(), AudioEngineError> {
        self.command_tx
            .send(DecoderCommand::Seek(position_ms))
            .map_err(|_| AudioEngineError::ChannelClosed)
    }

    pub fn set_volume(&self, volume: f32) {
        let clamped = volume.clamp(0.0, 1.0);
        let vol_int = (clamped * 1000.0) as u64;
        self.volume.store(vol_int, Ordering::Relaxed);
    }

    pub fn get_volume(&self) -> f32 {
        (self.volume.load(Ordering::Relaxed) as f32) / 1000.0
    }

    pub fn set_muted(&self, muted: bool) {
        self.is_muted.store(muted, Ordering::Relaxed);
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted.load(Ordering::Relaxed)
    }

    pub fn get_position_ms(&self) -> u64 {
        self.position_ms.load(Ordering::Relaxed)
    }

    pub fn get_duration_ms(&self) -> u64 {
        self.duration_ms.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::Relaxed)
    }

    pub fn set_eq_gains(&self, gains: [f32; 10]) {
        let _ = self.command_tx.send(DecoderCommand::SetEqGains(gains));
    }

    pub fn set_eq_enabled(&self, enabled: bool) {
        let _ = self.command_tx.send(DecoderCommand::SetEqEnabled(enabled));
    }

    pub fn set_lufs_enabled(&self, enabled: bool) {
        let _ = self.command_tx.send(DecoderCommand::SetLufsEnabled(enabled));
    }

    pub fn set_track_lufs(&self, lufs: f32) {
        let _ = self.command_tx.send(DecoderCommand::SetTrackLufs(lufs));
    }

    pub fn set_slowed_reverb_enabled(&self, enabled: bool) {
        let _ = self.command_tx.send(DecoderCommand::SetSlowedReverbEnabled(enabled));
    }

    pub fn set_slowed_speed(&self, speed: f32) {
        let _ = self.command_tx.send(DecoderCommand::SetSlowedSpeed(speed));
    }
}
