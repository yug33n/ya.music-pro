pub mod bitperfect;
pub mod controller;
pub mod crossfade;
pub mod decoder;
pub mod device;
pub mod ducking;
pub mod gapless;
pub mod resampler;
pub mod stream;

#[cfg(test)]
mod tests;

pub use bitperfect::{scale_sample_f64_tpdf, BitPerfectConfig, TpdfPrng};
pub use crossfade::EqualPowerCrossfader;
pub use ducking::AudioDucker;
pub use gapless::{DecoderSlot, DualBufferPipeline};

pub use controller::{AudioController, AudioEngine, AudioEngineError};
pub use decoder::{DecoderError, SymphoniaDecoder};
pub use device::{AudioDeviceInfo, DeviceManager};
pub use resampler::AudioResampler;
pub use stream::{AudioOutputStream, AudioStreamError};
