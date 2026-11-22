pub mod bitperfect;
pub mod controller;
pub mod decoder;
pub mod device;
pub mod gapless;
pub mod resampler;
pub mod stream;

#[cfg(test)]
mod tests;

pub use bitperfect::{scale_sample_f64_tpdf, BitPerfectConfig, TpdfPrng};
pub use gapless::{DecoderSlot, DualBufferPipeline};

pub use controller::{AudioController, AudioEngine, AudioEngineError};
pub use decoder::{DecoderError, SymphoniaDecoder};
pub use device::{AudioDeviceInfo, DeviceManager};
pub use resampler::AudioResampler;
pub use stream::{AudioOutputStream, AudioStreamError};
