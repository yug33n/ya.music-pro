use crate::core::types::AudioOutputMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitPerfectConfig {
    pub mode: AudioOutputMode,
    pub target_sample_rate: Option<u32>,
    pub enable_tpdf_dither: bool,
}

impl Default for BitPerfectConfig {
    fn default() -> Self {
        Self {
            mode: AudioOutputMode::Shared,
            target_sample_rate: None,
            enable_tpdf_dither: true,
        }
    }
}

/// Fast, lock-free, zero-allocation PRNG for real-time audio threads (Xorshift64)
pub struct TpdfPrng {
    state: u64,
}

impl TpdfPrng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x5EED_CAFE_BABE_F00D } else { seed },
        }
    }

    #[inline(always)]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Returns uniform random float in [-0.5, 0.5]
    #[inline(always)]
    pub fn next_uniform_f64(&mut self) -> f64 {
        // Map top 32 bits to [0, 1) then subtract 0.5
        let u32_val = (self.next_u64() >> 32) as u32;
        (u32_val as f64 / 4294967296.0) - 0.5
    }

    /// Triangular Probability Density Function (TPDF) dither value in [-1.0, 1.0] LSB
    #[inline(always)]
    pub fn next_tpdf_f64(&mut self) -> f64 {
        self.next_uniform_f64() - self.next_uniform_f64()
    }
}

impl Default for TpdfPrng {
    fn default() -> Self {
        Self::new(0x84A0_1234_BADC_0DE1)
    }
}

/// Scale sample using 64-bit precision with optional TPDF dither to prevent quantization distortion
#[inline(always)]
pub fn scale_sample_f64_tpdf(sample: f32, gain_f64: f64, enable_dither: bool, prng: &mut TpdfPrng) -> f32 {
    let s64 = sample as f64 * gain_f64;

    if !enable_dither || (gain_f64 - 1.0).abs() < 1e-6 {
        return s64.clamp(-1.0, 1.0) as f32;
    }

    // 24-bit audio LSB amplitude: 1.0 / 2^23 ≈ 1.1920929e-7
    const LSB_24BIT: f64 = 1.0 / 8388608.0;
    let dither = prng.next_tpdf_f64() * LSB_24BIT;

    (s64 + dither).clamp(-1.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tpdf_prng_distribution() {
        let mut prng = TpdfPrng::new(123456789);
        let mut sum = 0.0;
        let count = 100_000;

        for _ in 0..count {
            let t = prng.next_tpdf_f64();
            assert!(t >= -1.0 && t <= 1.0);
            sum += t;
        }

        let mean = sum / count as f64;
        println!("TPDF sample mean over 100k samples: {:.6}", mean);
        // Mean should be very close to 0.0
        assert!(mean.abs() < 0.02);
    }

    #[test]
    fn test_scale_sample_f64_tpdf_clamping() {
        let mut prng = TpdfPrng::default();

        let s_normal = scale_sample_f64_tpdf(0.5, 0.5, true, &mut prng);
        assert!((s_normal - 0.25).abs() < 0.001);

        // Clamping test
        let s_over = scale_sample_f64_tpdf(1.0, 2.0, true, &mut prng);
        assert_eq!(s_over, 1.0);

        let s_under = scale_sample_f64_tpdf(-1.0, 2.0, true, &mut prng);
        assert_eq!(s_under, -1.0);
    }
}
