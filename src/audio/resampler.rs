/// Resampler for converting audio sample rates (e.g. 44.1 kHz to 48 kHz).
/// Uses high-quality cubic interpolation for arbitrary chunk sizes with zero latency.
pub struct AudioResampler {
    input_rate: u32,
    output_rate: u32,
    ratio: f64, // input_rate / output_rate
    phase: f64,
    prev_samples: [f32; 2],
}

impl AudioResampler {
    pub fn new(input_rate: u32, output_rate: u32) -> Self {
        let ratio = if output_rate > 0 {
            input_rate as f64 / output_rate as f64
        } else {
            1.0
        };

        Self {
            input_rate,
            output_rate,
            ratio,
            phase: 0.0,
            prev_samples: [0.0, 0.0],
        }
    }

    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }

    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }

    pub fn is_identity(&self) -> bool {
        self.input_rate == self.output_rate
    }

    /// Resample interleaved stereo f32 samples
    pub fn process_interleaved_stereo(&mut self, input: &[f32], output: &mut Vec<f32>) {
        if self.is_identity() {
            output.extend_from_slice(input);
            return;
        }

        let num_frames_in = input.len() / 2;
        if num_frames_in == 0 {
            return;
        }

        while self.phase < num_frames_in as f64 {
            let idx = self.phase.floor() as usize;
            let frac = (self.phase - idx as f64) as f32;

            let (s0_l, s0_r) = if idx == 0 {
                (self.prev_samples[0], self.prev_samples[1])
            } else {
                (input[(idx - 1) * 2], input[(idx - 1) * 2 + 1])
            };

            let s1_l = input[idx * 2];
            let s1_r = input[idx * 2 + 1];

            let (s2_l, s2_r) = if idx + 1 < num_frames_in {
                (input[(idx + 1) * 2], input[(idx + 1) * 2 + 1])
            } else {
                (s1_l, s1_r)
            };

            let (s3_l, s3_r) = if idx + 2 < num_frames_in {
                (input[(idx + 2) * 2], input[(idx + 2) * 2 + 1])
            } else {
                (s2_l, s2_r)
            };

            // Cubic Catmull-Rom interpolation
            let out_l = cubic_interp(s0_l, s1_l, s2_l, s3_l, frac);
            let out_r = cubic_interp(s0_r, s1_r, s2_r, s3_r, frac);

            output.push(out_l);
            output.push(out_r);

            self.phase += self.ratio;
        }

        // Store last frame for seamless continuity across chunks
        if num_frames_in > 0 {
            self.prev_samples[0] = input[(num_frames_in - 1) * 2];
            self.prev_samples[1] = input[(num_frames_in - 1) * 2 + 1];
        }

        self.phase -= num_frames_in as f64;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.prev_samples = [0.0, 0.0];
    }
}

#[inline(always)]
fn cubic_interp(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let b = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c = -0.5 * p0 + 0.5 * p2;
    let d = p1;

    a * t * t * t + b * t * t + c * t + d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_resampler() {
        let mut resampler = AudioResampler::new(44100, 44100);
        assert!(resampler.is_identity());

        let input = vec![0.1, 0.2, 0.3, 0.4];
        let mut output = Vec::new();
        resampler.process_interleaved_stereo(&input, &mut output);

        assert_eq!(input, output);
    }

    #[test]
    fn test_upsample_44100_to_48000() {
        let mut resampler = AudioResampler::new(44100, 48000);
        assert!(!resampler.is_identity());

        let num_input_frames = 4410;
        let mut input = Vec::with_capacity(num_input_frames * 2);
        for i in 0..num_input_frames {
            let sample = (i as f32 * 0.01).sin();
            input.push(sample);
            input.push(sample);
        }

        let mut output = Vec::new();
        resampler.process_interleaved_stereo(&input, &mut output);

        let num_output_frames = output.len() / 2;
        // Expected ~4800 frames
        assert!((num_output_frames as i32 - 4800).abs() <= 2);
    }
}
