use crate::audio::decoder::{DecoderError, SymphoniaDecoder};
use crate::audio::resampler::AudioResampler;

pub struct DecoderSlot {
    pub decoder: SymphoniaDecoder,
    pub resampler: AudioResampler,
    pub raw_buffer: Vec<f32>,
    pub resampled_buffer: Vec<f32>,
    pub buffer_offset: usize,
    pub duration_ms: u64,
    pub is_eof: bool,
}

impl DecoderSlot {
    pub fn new(decoder: SymphoniaDecoder, target_sample_rate: u32) -> Self {
        let duration_ms = decoder.duration_ms();
        let sample_rate = decoder.sample_rate();
        let resampler = AudioResampler::new(sample_rate, target_sample_rate);

        Self {
            decoder,
            resampler,
            raw_buffer: Vec::with_capacity(4096),
            resampled_buffer: Vec::with_capacity(4096),
            buffer_offset: 0,
            duration_ms,
            is_eof: false,
        }
    }

    /// Read next chunk of decoded and resampled stereo samples
    pub fn read_next_chunk(&mut self) -> Result<bool, DecoderError> {
        self.raw_buffer.clear();
        self.resampled_buffer.clear();
        self.buffer_offset = 0;

        match self.decoder.next_packet_stereo(&mut self.raw_buffer) {
            Ok(true) => {
                self.resampler
                    .process_interleaved_stereo(&self.raw_buffer, &mut self.resampled_buffer);
                Ok(true)
            }
            Ok(false) => {
                self.is_eof = true;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
}

pub struct DualBufferPipeline {
    active_slot: Option<DecoderSlot>,
    queued_slot: Option<DecoderSlot>,
    target_sample_rate: u32,
    pub gapless_swapped: bool,
}

impl DualBufferPipeline {
    pub fn new(target_sample_rate: u32) -> Self {
        Self {
            active_slot: None,
            queued_slot: None,
            target_sample_rate,
            gapless_swapped: false,
        }
    }

    pub fn set_active(&mut self, decoder: SymphoniaDecoder) {
        self.active_slot = Some(DecoderSlot::new(decoder, self.target_sample_rate));
        self.gapless_swapped = false;
    }

    pub fn queue_next(&mut self, decoder: SymphoniaDecoder) {
        self.queued_slot = Some(DecoderSlot::new(decoder, self.target_sample_rate));
    }

    pub fn has_queued(&self) -> bool {
        self.queued_slot.is_some()
    }

    pub fn clear(&mut self) {
        self.active_slot = None;
        self.queued_slot = None;
        self.gapless_swapped = false;
    }

    /// Read stereo samples into `output`. Seamlessly switches to queued slot on active EOF.
    pub fn read_stereo_samples(
        &mut self,
        output: &mut Vec<f32>,
        desired_frames: usize,
    ) -> Result<bool, DecoderError> {
        self.gapless_swapped = false;
        let mut frames_collected = 0;

        while frames_collected < desired_frames {
            if let Some(ref mut active) = self.active_slot {
                // Check if buffer has remaining samples
                if active.buffer_offset < active.resampled_buffer.len() {
                    let available_samples = active.resampled_buffer.len() - active.buffer_offset;
                    let needed_samples = (desired_frames - frames_collected) * 2;
                    let take = available_samples.min(needed_samples);

                    output.extend_from_slice(
                        &active.resampled_buffer[active.buffer_offset..active.buffer_offset + take],
                    );
                    active.buffer_offset += take;
                    frames_collected += take / 2;
                    continue;
                }

                // Buffer is exhausted, decode next packet
                if !active.is_eof {
                    match active.read_next_chunk()? {
                        true => continue,
                        false => {
                            // Active slot reached EOF, switch to queued next track immediately!
                        }
                    }
                }
            }

            // Active reached EOF or None, check if queued slot is ready
            if let Some(next) = self.queued_slot.take() {
                self.active_slot = Some(next);
                self.gapless_swapped = true;
                log::info!("Gapless transition: switched to pre-buffered next track without gap");
                continue;
            }

            // Both active and queued are exhausted
            break;
        }

        Ok(frames_collected > 0)
    }

    pub fn active_duration_ms(&self) -> u64 {
        self.active_slot.as_ref().map(|s| s.duration_ms).unwrap_or(0)
    }
}
