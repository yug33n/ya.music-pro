use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use ringbuf::traits::{Consumer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioStreamError {
    #[error("Failed to build output stream: {0}")]
    BuildStream(#[from] cpal::BuildStreamError),

    #[error("Failed to play stream: {0}")]
    PlayStream(#[from] cpal::PlayStreamError),

    #[error("Default stream config error: {0}")]
    DefaultConfig(#[from] cpal::DefaultStreamConfigError),

    #[error("No audio device available")]
    NoDeviceAvailable,

    #[error("Unsupported sample format: {0:?}")]
    UnsupportedSampleFormat(SampleFormat),
}

pub struct AudioOutputStream {
    _stream: Stream,
    producer: HeapProd<f32>,
    sample_rate: u32,
    channels: u16,
    is_muted: Arc<AtomicBool>,
    is_paused: Arc<AtomicBool>,
}

impl AudioOutputStream {
    pub fn new(
        device: &Device,
        buffer_capacity: usize,
    ) -> Result<Self, AudioStreamError> {
        let supported_config = device.default_output_config()?;
        let sample_rate = supported_config.sample_rate().0;
        let channels = supported_config.channels();
        let sample_format = supported_config.sample_format();

        if sample_format != SampleFormat::F32 {
            log::warn!(
                "Device native format is {:?}, will request F32 stream config",
                sample_format
            );
        }

        let config: StreamConfig = StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let rb = HeapRb::<f32>::new(buffer_capacity);
        let (producer, mut consumer): (HeapProd<f32>, HeapCons<f32>) = rb.split();

        let is_muted = Arc::new(AtomicBool::new(false));
        let is_paused = Arc::new(AtomicBool::new(false));

        let is_muted_cb = is_muted.clone();
        let is_paused_cb = is_paused.clone();

        let err_fn = |err| log::error!("Audio output stream error: {}", err);

        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let muted = is_muted_cb.load(Ordering::Relaxed);
                let paused = is_paused_cb.load(Ordering::Relaxed);

                if paused {
                    data.fill(0.0);
                    return;
                }

                for sample in data.iter_mut() {
                    if let Some(s) = consumer.try_pop() {
                        *sample = if muted { 0.0 } else { s };
                    } else {
                        *sample = 0.0;
                    }
                }
            },
            err_fn,
            None,
        )?;

        stream.play()?;

        Ok(Self {
            _stream: stream,
            producer,
            sample_rate,
            channels,
            is_muted,
            is_paused,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn set_muted(&self, muted: bool) {
        self.is_muted.store(muted, Ordering::Relaxed);
    }

    pub fn set_paused(&self, paused: bool) {
        self.is_paused.store(paused, Ordering::Relaxed);
    }

    pub fn producer_mut(&mut self) -> &mut HeapProd<f32> {
        &mut self.producer
    }
}
