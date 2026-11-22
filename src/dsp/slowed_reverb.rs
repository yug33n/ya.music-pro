use rubato::{FastFixedIn, PolynomialDegree, Resampler};

const STEREO_SPREAD: usize = 23;
const COMB_TUNING_L: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING_L: [usize; 4] = [556, 441, 341, 225];

struct CombFilter {
    buffer: Vec<f32>,
    idx: usize,
    filter_store: f32,
    damp1: f32,
    damp2: f32,
    feedback: f32,
}

impl CombFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size.max(1)],
            idx: 0,
            filter_store: 0.0,
            damp1: 0.2,
            damp2: 0.8,
            feedback: 0.8,
        }
    }

    fn set_damp(&mut self, val: f32) {
        self.damp1 = val;
        self.damp2 = 1.0 - val;
    }

    fn set_feedback(&mut self, val: f32) {
        self.feedback = val;
    }

    #[inline(always)]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.idx];
        self.filter_store = output * self.damp2 + self.filter_store * self.damp1;
        self.buffer[self.idx] = input + self.filter_store * self.feedback;
        self.idx += 1;
        if self.idx >= self.buffer.len() {
            self.idx = 0;
        }
        output
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.filter_store = 0.0;
        self.idx = 0;
    }
}

struct AllPassFilter {
    buffer: Vec<f32>,
    idx: usize,
    feedback: f32,
}

impl AllPassFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size.max(1)],
            idx: 0,
            feedback: 0.5,
        }
    }

    #[inline(always)]
    fn process(&mut self, input: f32) -> f32 {
        let buf_out = self.buffer[self.idx];
        let output = -input + buf_out;
        self.buffer[self.idx] = input + buf_out * self.feedback;
        self.idx += 1;
        if self.idx >= self.buffer.len() {
            self.idx = 0;
        }
        output
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.idx = 0;
    }
}

pub struct Freeverb {
    combs_l: Vec<CombFilter>,
    combs_r: Vec<CombFilter>,
    allpasses_l: Vec<AllPassFilter>,
    allpasses_r: Vec<AllPassFilter>,
    room_size: f32,
    damp: f32,
    wet: f32,
    dry: f32,
    width: f32,
}

impl Freeverb {
    pub fn new(sample_rate: u32) -> Self {
        let scale = sample_rate as f32 / 44100.0;
        let mut combs_l = Vec::with_capacity(8);
        let mut combs_r = Vec::with_capacity(8);
        for &tuning in &COMB_TUNING_L {
            combs_l.push(CombFilter::new((tuning as f32 * scale) as usize));
            combs_r.push(CombFilter::new(((tuning + STEREO_SPREAD) as f32 * scale) as usize));
        }

        let mut allpasses_l = Vec::with_capacity(4);
        let mut allpasses_r = Vec::with_capacity(4);
        for &tuning in &ALLPASS_TUNING_L {
            allpasses_l.push(AllPassFilter::new((tuning as f32 * scale) as usize));
            allpasses_r.push(AllPassFilter::new(((tuning + STEREO_SPREAD) as f32 * scale) as usize));
        }

        let mut verb = Self {
            combs_l,
            combs_r,
            allpasses_l,
            allpasses_r,
            room_size: 0.8,
            damp: 0.3,
            wet: 0.4,
            dry: 0.85,
            width: 1.0,
        };
        verb.update_parameters();
        verb
    }

    fn update_parameters(&mut self) {
        let feedback = self.room_size.clamp(0.0, 0.98);
        let damp = self.damp.clamp(0.0, 1.0);
        for comb in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            comb.set_feedback(feedback);
            comb.set_damp(damp);
        }
    }

    pub fn set_room_size(&mut self, val: f32) {
        self.room_size = val;
        self.update_parameters();
    }

    pub fn set_damp(&mut self, val: f32) {
        self.damp = val;
        self.update_parameters();
    }

    pub fn set_wet(&mut self, val: f32) {
        self.wet = val;
    }

    pub fn set_dry(&mut self, val: f32) {
        self.dry = val;
    }

    pub fn process_stereo_frame(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let input_mono = (in_l + in_r) * 0.5 * 0.015; // gain scale into combs

        let mut out_l = 0.0;
        let mut out_r = 0.0;

        for comb in &mut self.combs_l {
            out_l += comb.process(input_mono);
        }
        for comb in &mut self.combs_r {
            out_r += comb.process(input_mono);
        }

        for ap in &mut self.allpasses_l {
            out_l = ap.process(out_l);
        }
        for ap in &mut self.allpasses_r {
            out_r = ap.process(out_r);
        }

        let wet1 = self.wet * (self.width * 0.5 + 0.5);
        let wet2 = self.wet * ((1.0 - self.width) * 0.5);

        let final_l = in_l * self.dry + out_l * wet1 + out_r * wet2;
        let final_r = in_r * self.dry + out_r * wet1 + out_l * wet2;

        (final_l, final_r)
    }

    pub fn clear(&mut self) {
        for c in &mut self.combs_l {
            c.clear();
        }
        for c in &mut self.combs_r {
            c.clear();
        }
        for a in &mut self.allpasses_l {
            a.clear();
        }
        for a in &mut self.allpasses_r {
            a.clear();
        }
    }
}

pub struct SlowedReverb {
    _sample_rate: u32,
    speed_rate: f32,
    is_enabled: bool,
    reverb: Freeverb,
    resampler: Option<FastFixedIn<f32>>,
    chunk_size: usize,
    in_buf_l: Vec<f32>,
    in_buf_r: Vec<f32>,
}

impl SlowedReverb {
    pub const DEFAULT_SPEED: f32 = 0.85;

    pub fn new(sample_rate: u32) -> Self {
        let chunk_size = 512;
        let resample_ratio = (1.0 / Self::DEFAULT_SPEED) as f64;
        let resampler = FastFixedIn::<f32>::new(
            resample_ratio,
            1.5,
            PolynomialDegree::Cubic,
            chunk_size,
            2,
        ).ok();

        Self {
            _sample_rate: sample_rate,
            speed_rate: Self::DEFAULT_SPEED,
            is_enabled: false,
            reverb: Freeverb::new(sample_rate),
            resampler,
            chunk_size,
            in_buf_l: Vec::with_capacity(chunk_size * 2),
            in_buf_r: Vec::with_capacity(chunk_size * 2),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.is_enabled = enabled;
        if !enabled {
            self.reverb.clear();
            self.in_buf_l.clear();
            self.in_buf_r.clear();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    pub fn set_speed(&mut self, speed: f32) {
        let speed = speed.clamp(0.7, 1.0);
        self.speed_rate = speed;
        let resample_ratio = (1.0 / speed) as f64;
        self.resampler = FastFixedIn::<f32>::new(
            resample_ratio,
            1.5,
            PolynomialDegree::Cubic,
            self.chunk_size,
            2,
        ).ok();
    }

    pub fn speed(&self) -> f32 {
        self.speed_rate
    }

    pub fn freeverb_mut(&mut self) -> &mut Freeverb {
        &mut self.reverb
    }

    /// Process interleaved stereo input into output buffer
    pub fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        if !self.is_enabled {
            output.clear();
            output.extend_from_slice(input);
            return;
        }

        output.clear();
        // Deinterleave into channel buffers
        let (chunks, _) = input.as_chunks::<2>();
        for &[l, r] in chunks {
            self.in_buf_l.push(l);
            self.in_buf_r.push(r);
        }

        if let Some(ref mut resampler) = self.resampler {
            let req_frames = resampler.input_frames_next();
            while self.in_buf_l.len() >= req_frames {
                let slice_l: Vec<f32> = self.in_buf_l.drain(..req_frames).collect();
                let slice_r: Vec<f32> = self.in_buf_r.drain(..req_frames).collect();

                let waves_in = [slice_l, slice_r];
                if let Ok(resampled) = resampler.process(&waves_in, None) {
                    for (&sl, &sr) in resampled[0].iter().zip(resampled[1].iter()) {
                        let (vl, vr) = self.reverb.process_stereo_frame(sl, sr);
                        output.push(vl);
                        output.push(vr);
                    }
                }
            }
        } else {
            // Fallback if no resampler: apply reverb only
            for (sl, sr) in self.in_buf_l.drain(..).zip(self.in_buf_r.drain(..)) {
                let (vl, vr) = self.reverb.process_stereo_frame(sl, sr);
                output.push(vl);
                output.push(vr);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freeverb_stereo_processing() {
        let mut verb = Freeverb::new(48000);
        let (out_l, out_r) = verb.process_stereo_frame(0.5, 0.5);
        // Direct output should have audio content
        assert!(out_l.abs() > 0.0);
        assert!(out_r.abs() > 0.0);
    }

    #[test]
    fn test_slowed_reverb_bypass() {
        let mut sr = SlowedReverb::new(48000);
        sr.set_enabled(false);
        let input = vec![0.1, 0.2, 0.3, 0.4];
        let mut output = Vec::new();
        sr.process(&input, &mut output);
        assert_eq!(output, input);
    }

    #[test]
    fn test_slowed_reverb_processing() {
        let mut sr = SlowedReverb::new(48000);
        sr.set_enabled(true);
        // Push 1024 frames of 440Hz sine wave stereo
        let mut input = Vec::with_capacity(2048);
        for i in 0..1024 {
            let s = ((i as f32) * 0.05).sin() * 0.5;
            input.push(s);
            input.push(s);
        }
        let mut output = Vec::new();
        sr.process(&input, &mut output);
        // Output should be generated
        assert!(!output.is_empty());
    }
}
