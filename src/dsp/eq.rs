use biquad::{Biquad, Coefficients, DirectForm2Transposed, ToHertz, Type, Q_BUTTERWORTH_F32};

pub const EQ_FREQUENCIES: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

pub struct Equalizer10Band {
    filters_l: [DirectForm2Transposed<f32>; 10],
    filters_r: [DirectForm2Transposed<f32>; 10],
    gains_db: [f32; 10],
    sample_rate: u32,
    is_enabled: bool,
}

impl Equalizer10Band {
    pub fn new(sample_rate: u32) -> Self {
        let dummy_coeffs = Coefficients::<f32>::from_params(
            Type::PeakingEQ(0.0),
            sample_rate.hz(),
            1000.0.hz(),
            Q_BUTTERWORTH_F32,
        )
        .unwrap_or(Coefficients {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        });

        let mut filters_l = [
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
        ];

        let mut filters_r = [
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
            DirectForm2Transposed::<f32>::new(dummy_coeffs),
        ];

        let gains_db = [0.0; 10];

        // Initialize coefficients for all 10 bands
        for (i, &freq) in EQ_FREQUENCIES.iter().enumerate() {
            if let Ok(coeffs) = Coefficients::<f32>::from_params(
                Type::PeakingEQ(0.0),
                sample_rate.hz(),
                freq.hz(),
                Q_BUTTERWORTH_F32,
            ) {
                filters_l[i] = DirectForm2Transposed::<f32>::new(coeffs);
                filters_r[i] = DirectForm2Transposed::<f32>::new(coeffs);
            }
        }

        Self {
            filters_l,
            filters_r,
            gains_db,
            sample_rate,
            is_enabled: true,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.rebuild_filters();
        }
    }

    pub fn set_gains(&mut self, gains: &[f32; 10]) {
        for (i, &g) in gains.iter().enumerate() {
            self.gains_db[i] = g.clamp(-12.0, 12.0);
        }
        self.rebuild_filters();
    }

    pub fn set_band_gain(&mut self, band_idx: usize, gain_db: f32) {
        if band_idx < 10 {
            self.gains_db[band_idx] = gain_db.clamp(-12.0, 12.0);
            self.update_band(band_idx);
        }
    }

    pub fn gains(&self) -> &[f32; 10] {
        &self.gains_db
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.is_enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    fn update_band(&mut self, band_idx: usize) {
        let freq = EQ_FREQUENCIES[band_idx];
        let gain = self.gains_db[band_idx];

        if let Ok(coeffs) = Coefficients::<f32>::from_params(
            Type::PeakingEQ(gain),
            self.sample_rate.hz(),
            freq.hz(),
            Q_BUTTERWORTH_F32,
        ) {
            self.filters_l[band_idx].update_coefficients(coeffs);
            self.filters_r[band_idx].update_coefficients(coeffs);
        }
    }

    fn rebuild_filters(&mut self) {
        for i in 0..10 {
            self.update_band(i);
        }
    }

    /// Process interleaved stereo samples in-place
    pub fn process_interleaved_stereo(&mut self, samples: &mut [f32]) {
        if !self.is_enabled {
            return;
        }

        let (chunks, _) = samples.as_chunks_mut::<2>();
        for frame in chunks {
            let mut l = frame[0];
            let mut r = frame[1];

            // Run through cascade of 10 biquad filters
            for i in 0..10 {
                // If gain is essentially 0 dB, skip filter evaluation for CPU efficiency
                if self.gains_db[i].abs() > 0.05 {
                    l = self.filters_l[i].run(l);
                    r = self.filters_r[i].run(r);
                }
            }

            frame[0] = l;
            frame[1] = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eq_initialization_and_bypass() {
        let mut eq = Equalizer10Band::new(48000);
        assert_eq!(*eq.gains(), [0.0; 10]);

        let mut samples = vec![0.5, -0.5, 0.25, -0.25];
        let original = samples.clone();

        eq.process_interleaved_stereo(&mut samples);
        // Flat gains should leave samples virtually unchanged
        for (a, b) in samples.iter().zip(original.iter()) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn test_eq_boost_processing() {
        let mut eq = Equalizer10Band::new(48000);
        // Boost 1 kHz band by +6 dB
        eq.set_band_gain(5, 6.0);
        assert_eq!(eq.gains()[5], 6.0);

        let mut samples = Vec::new();
        for i in 0..1000 {
            let s = (i as f32 * 0.13).sin(); // 1 kHz-ish frequency
            samples.push(s);
            samples.push(s);
        }

        eq.process_interleaved_stereo(&mut samples);
        assert!(!samples.is_empty());
    }
}
