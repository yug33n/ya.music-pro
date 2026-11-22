/// Real-time LUFS (ITU-R BS.1770-4 / EBU R128) loudness normalizer and peak limiter
pub const DEFAULT_TARGET_LUFS: f32 = -14.0;

pub struct LufsNormalizer {
    target_lufs: f32,
    current_gain: f32,
    target_gain: f32,
    smoothing_factor: f32,
    is_enabled: bool,
    // K-weighting filter states for stereo channels
    k_state_l: [f32; 4],
    k_state_r: [f32; 4],
    sum_squares: f64,
    sample_count: u64,
}

impl LufsNormalizer {
    pub fn new(target_lufs: f32, sample_rate: u32) -> Self {
        // Smoothing factor for ~100ms exponential moving average gain transition
        let smoothing_factor = 1.0 - (-1.0 / (sample_rate as f32 * 0.1)).exp();

        Self {
            target_lufs,
            current_gain: 1.0,
            target_gain: 1.0,
            smoothing_factor,
            is_enabled: true,
            k_state_l: [0.0; 4],
            k_state_r: [0.0; 4],
            sum_squares: 0.0,
            sample_count: 0,
        }
    }

    pub fn set_target_lufs(&mut self, target_lufs: f32) {
        self.target_lufs = target_lufs;
    }

    pub fn target_lufs(&self) -> f32 {
        self.target_lufs
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.is_enabled = enabled;
        if !enabled {
            self.target_gain = 1.0;
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    /// Set track integrated loudness if known from metadata/ReplayGain (e.g. -9.5 LUFS)
    pub fn set_track_lufs(&mut self, track_lufs: f32) {
        if !self.is_enabled {
            self.target_gain = 1.0;
            return;
        }

        let delta_db = (self.target_lufs - track_lufs).clamp(-18.0, 12.0);
        self.target_gain = 10.0f32.powf(delta_db / 20.0);
    }

    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }

    /// Process interleaved stereo samples in-place: apply smoothed gain and transparent peak limiter
    pub fn process_interleaved_stereo(&mut self, samples: &mut [f32]) {
        if !self.is_enabled && (self.current_gain - 1.0).abs() < 1e-4 {
            return;
        }

        let (chunks, _) = samples.as_chunks_mut::<2>();
        for frame in chunks {
            // Smoothly interpolate current gain towards target gain to prevent clicks
            self.current_gain += (self.target_gain - self.current_gain) * self.smoothing_factor;

            let in_l = frame[0] * self.current_gain;
            let in_r = frame[1] * self.current_gain;

            // Transparent soft-clipping peak limiter (tanh saturation above 0.95)
            frame[0] = soft_clip(in_l);
            frame[1] = soft_clip(in_r);

            // Accumulate power for real-time LUFS estimation
            self.accumulate_k_weighted(frame[0], frame[1]);
        }
    }

    fn accumulate_k_weighted(&mut self, l: f32, r: f32) {
        // Simplified K-weighting pre-filter stage
        let k_l = 0.5 * l + 0.5 * self.k_state_l[0];
        let k_r = 0.5 * r + 0.5 * self.k_state_r[0];
        self.k_state_l[0] = l;
        self.k_state_r[0] = r;

        let frame_power = (k_l * k_l + k_r * k_r) as f64;
        self.sum_squares += frame_power;
        self.sample_count += 1;
    }

    /// Calculate estimated current integrated LUFS
    pub fn estimated_lufs(&self) -> f32 {
        if self.sample_count == 0 || self.sum_squares <= 1e-12 {
            return -70.0;
        }
        let mean_square = self.sum_squares / self.sample_count as f64;
        let lufs = -0.691 + 10.0 * mean_square.log10();
        (lufs as f32).clamp(-70.0, 0.0)
    }

    pub fn reset_meter(&mut self) {
        self.sum_squares = 0.0;
        self.sample_count = 0;
        self.k_state_l = [0.0; 4];
        self.k_state_r = [0.0; 4];
    }
}

/// Smooth soft-clipping limiter with unity gain in linear range [-0.95, +0.95]
#[inline(always)]
fn soft_clip(x: f32) -> f32 {
    const THRESHOLD: f32 = 0.95;
    const MARGIN: f32 = 0.049;
    if x > THRESHOLD {
        THRESHOLD + MARGIN * ((x - THRESHOLD) / MARGIN).tanh()
    } else if x < -THRESHOLD {
        -THRESHOLD - MARGIN * ((-x - THRESHOLD) / MARGIN).tanh()
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lufs_gain_calculation() {
        let mut normalizer = LufsNormalizer::new(-14.0, 48000);

        // Loud track (-8 LUFS) -> should attenuate by -6 dB (~0.501 gain)
        normalizer.set_track_lufs(-8.0);
        let expected_gain = 10.0f32.powf(-6.0 / 20.0);
        assert!((normalizer.target_gain - expected_gain).abs() < 0.01);

        // Quiet track (-20 LUFS) -> should boost by +6 dB (~1.995 gain)
        normalizer.set_track_lufs(-20.0);
        let expected_boost = 10.0f32.powf(6.0 / 20.0);
        assert!((normalizer.target_gain - expected_boost).abs() < 0.01);
    }

    #[test]
    fn test_soft_clipping_limiter() {
        // Linear region pass-through
        assert_eq!(soft_clip(0.5), 0.5);
        assert_eq!(soft_clip(-0.5), -0.5);

        // Loud peaks are rounded smoothly without exceeding 1.0
        let clipped = soft_clip(1.5);
        assert!(clipped < 1.0 && clipped > 0.95);

        let clipped_neg = soft_clip(-2.0);
        assert!(clipped_neg > -1.0 && clipped_neg < -0.95);
    }
}
