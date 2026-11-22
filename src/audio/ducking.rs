use std::time::{Duration, Instant};

/// Voice activity audio ducker with smooth exponential attack and release
pub struct AudioDucker {
    enabled: bool,
    target_duck_level: f32, // Target attenuation (e.g. 0.25 = 25% of master volume)
    current_gain: f32,
    target_gain: f32,
    attack_coeff: f32,      // Per-sample smoothing coefficient for ~250ms attack
    release_coeff: f32,     // Per-sample smoothing coefficient for ~600ms release
    hold_duration: Duration, // Hold time (1.5 - 2.0s) before releasing ducked volume
    last_voice_activity: Option<Instant>,
    is_voice_active: bool,
}

impl AudioDucker {
    pub const DEFAULT_DUCK_LEVEL: f32 = 0.25;
    pub const DEFAULT_ATTACK_SECS: f32 = 0.250;
    pub const DEFAULT_RELEASE_SECS: f32 = 0.600;
    pub const DEFAULT_HOLD_SECS: f32 = 1.8;

    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        // Standard audio time-constant for 95% settling time: tau = T / 3.0
        let attack_coeff = 1.0 - (-3.0 / (sr * Self::DEFAULT_ATTACK_SECS)).exp();
        let release_coeff = 1.0 - (-3.0 / (sr * Self::DEFAULT_RELEASE_SECS)).exp();

        Self {
            enabled: true,
            target_duck_level: Self::DEFAULT_DUCK_LEVEL,
            current_gain: 1.0,
            target_gain: 1.0,
            attack_coeff,
            release_coeff,
            hold_duration: Duration::from_millis((Self::DEFAULT_HOLD_SECS * 1000.0) as u64),
            last_voice_activity: None,
            is_voice_active: false,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.target_gain = 1.0;
            self.current_gain = 1.0;
            self.is_voice_active = false;
            self.last_voice_activity = None;
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_duck_level(&mut self, level: f32) {
        self.target_duck_level = level.clamp(0.05, 0.95);
        if self.is_voice_active {
            self.target_gain = self.target_duck_level;
        }
    }

    pub fn duck_level(&self) -> f32 {
        self.target_duck_level
    }

    pub fn set_voice_active(&mut self, active: bool) {
        self.is_voice_active = active;
        if active {
            self.last_voice_activity = Some(Instant::now());
            if self.enabled {
                self.target_gain = self.target_duck_level;
            }
        }
    }

    pub fn is_voice_active(&self) -> bool {
        self.is_voice_active
    }

    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }

    /// Process interleaved stereo audio samples in-place applying smooth ducking attenuation
    pub fn process_interleaved_stereo(&mut self, samples: &mut [f32]) {
        if !self.enabled && (self.current_gain - 1.0).abs() < 1e-4 {
            return;
        }

        // Update target gain based on hold timer when not actively speaking
        if !self.is_voice_active {
            if let Some(last_act) = self.last_voice_activity {
                if last_act.elapsed() >= self.hold_duration {
                    self.target_gain = 1.0;
                } else if self.enabled {
                    self.target_gain = self.target_duck_level;
                }
            } else {
                self.target_gain = 1.0;
            }
        }

        let (chunks, _) = samples.as_chunks_mut::<2>();
        for frame in chunks {
            let coeff = if self.target_gain < self.current_gain {
                self.attack_coeff
            } else {
                self.release_coeff
            };

            self.current_gain += (self.target_gain - self.current_gain) * coeff;
            frame[0] *= self.current_gain;
            frame[1] *= self.current_gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ducking_initial_state() {
        let ducker = AudioDucker::new(48000);
        assert!(ducker.is_enabled());
        assert_eq!(ducker.duck_level(), 0.25);
        assert_eq!(ducker.current_gain(), 1.0);
        assert!(!ducker.is_voice_active());
    }

    #[test]
    fn test_ducking_attack_transition() {
        let mut ducker = AudioDucker::new(48000);
        ducker.set_voice_active(true);

        // Process 250ms of audio (12000 stereo frames = 24000 samples)
        let mut samples = vec![1.0; 24000];
        ducker.process_interleaved_stereo(&mut samples);

        // Gain should smoothly ramp down towards 0.25
        assert!(ducker.current_gain() < 0.35);
        assert!(ducker.current_gain() >= 0.25);
    }

    #[test]
    fn test_ducking_release_after_hold() {
        let mut ducker = AudioDucker::new(48000);
        ducker.set_voice_active(true);

        // Attack phase
        let mut samples = vec![1.0; 24000];
        ducker.process_interleaved_stereo(&mut samples);
        assert!(ducker.current_gain() < 0.35);

        // Voice ceases, but within hold time gain remains ducked
        ducker.set_voice_active(false);
        ducker.process_interleaved_stereo(&mut samples);
        assert!(ducker.current_gain() < 0.35);

        // Manually advance last activity past hold duration to test release
        ducker.last_voice_activity = Some(Instant::now() - Duration::from_secs(3));
        // Process 600ms of audio (28800 stereo frames = 57600 samples)
        let mut release_samples = vec![1.0; 57600];
        ducker.process_interleaved_stereo(&mut release_samples);

        // Gain should smoothly restore towards 1.0
        assert!(ducker.current_gain() > 0.90);
    }

    #[test]
    fn test_ducking_disabled_bypass() {
        let mut ducker = AudioDucker::new(48000);
        ducker.set_enabled(false);
        ducker.set_voice_active(true);

        let mut samples = vec![0.5; 100];
        ducker.process_interleaved_stereo(&mut samples);

        // Signal remains untouched
        assert_eq!(samples[0], 0.5);
        assert_eq!(ducker.current_gain(), 1.0);
    }
}
