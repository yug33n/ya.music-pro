use std::f32::consts::FRAC_PI_2;

/// Equal-Power crossfading processor
/// Preserves acoustic energy across tracks: Gain_out(t)^2 + Gain_in(t)^2 = 1.0
pub struct EqualPowerCrossfader {
    duration_seconds: u32,
    sample_rate: u32,
    total_frames: u64,
    current_frame: u64,
    is_active: bool,
}

impl EqualPowerCrossfader {
    pub fn new(duration_seconds: u32, sample_rate: u32) -> Self {
        let clamped_secs = duration_seconds.clamp(1, 12);
        let total_frames = clamped_secs as u64 * sample_rate as u64;

        Self {
            duration_seconds: clamped_secs,
            sample_rate,
            total_frames,
            current_frame: 0,
            is_active: false,
        }
    }

    pub fn set_duration_seconds(&mut self, seconds: u32) {
        self.duration_seconds = seconds.clamp(1, 12);
        self.total_frames = self.duration_seconds as u64 * self.sample_rate as u64;
    }

    pub fn duration_seconds(&self) -> u32 {
        self.duration_seconds
    }

    pub fn start(&mut self) {
        self.current_frame = 0;
        self.is_active = true;
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn is_completed(&self) -> bool {
        self.current_frame >= self.total_frames
    }

    pub fn reset(&mut self) {
        self.current_frame = 0;
        self.is_active = false;
    }

    /// Process a stereo pair [left, right] from outgoing track and incoming track
    #[inline(always)]
    pub fn mix_stereo_frame(
        &mut self,
        out_l: f32,
        out_r: f32,
        in_l: f32,
        in_r: f32,
    ) -> (f32, f32) {
        if !self.is_active || self.total_frames == 0 {
            return (out_l, out_r);
        }

        let progress = (self.current_frame as f32 / self.total_frames as f32).clamp(0.0, 1.0);
        let angle = FRAC_PI_2 * progress;

        let gain_out = angle.cos();
        let gain_in = angle.sin();

        self.current_frame += 1;
        if self.current_frame >= self.total_frames {
            self.is_active = false;
        }

        let res_l = out_l * gain_out + in_l * gain_in;
        let res_r = out_r * gain_out + in_r * gain_in;

        (res_l, res_r)
    }

    /// Calculate instantaneous equal-power gains (gain_out, gain_in) for a given progress (0.0 to 1.0)
    #[inline(always)]
    pub fn calculate_gains(progress: f32) -> (f32, f32) {
        let p = progress.clamp(0.0, 1.0);
        let angle = FRAC_PI_2 * p;
        (angle.cos(), angle.sin())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equal_power_gain_conservation() {
        // Test gain formula at various progress points
        for i in 0..=100 {
            let progress = i as f32 / 100.0;
            let (gain_out, gain_in) = EqualPowerCrossfader::calculate_gains(progress);

            // Equal-Power identity: gain_out^2 + gain_in^2 == 1.0
            let power_sum = gain_out * gain_out + gain_in * gain_in;
            assert!(
                (power_sum - 1.0).abs() < 1e-5,
                "Equal-power conservation failed at progress {}: sum = {}",
                progress,
                power_sum
            );
        }
    }

    #[test]
    fn test_boundary_values() {
        let (out_start, in_start) = EqualPowerCrossfader::calculate_gains(0.0);
        assert!((out_start - 1.0).abs() < 1e-6);
        assert!(in_start.abs() < 1e-6);

        let (out_mid, in_mid) = EqualPowerCrossfader::calculate_gains(0.5);
        let expected_halfway = 0.5f32.sqrt(); // ~0.70710678
        assert!((out_mid - expected_halfway).abs() < 1e-5);
        assert!((in_mid - expected_halfway).abs() < 1e-5);

        let (out_end, in_end) = EqualPowerCrossfader::calculate_gains(1.0);
        assert!(out_end.abs() < 1e-6);
        assert!((in_end - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_crossfader_frame_processing() {
        let mut fader = EqualPowerCrossfader::new(2, 48000);
        fader.start();
        assert!(fader.is_active());

        let (mixed_l, mixed_r) = fader.mix_stereo_frame(1.0, 1.0, 0.0, 0.0);
        assert!((mixed_l - 1.0).abs() < 0.001);
        assert!((mixed_r - 1.0).abs() < 0.001);
    }
}
