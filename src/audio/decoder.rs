use std::io::Cursor;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DecoderError {
    #[error("Symphonia audio decoding error: {0}")]
    Symphonia(#[from] SymphoniaError),

    #[error("No supported audio track found in media source")]
    NoTrackFound,

    #[error("Decoder initialization error: {0}")]
    Init(String),
}

pub struct SymphoniaDecoder {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: usize,
    time_base: Option<TimeBase>,
    duration_ms: u64,
    sample_buf: Option<SampleBuffer<f32>>,
}

impl SymphoniaDecoder {
    pub fn from_bytes(bytes: Vec<u8>, hint_ext: Option<&str>) -> Result<Self, DecoderError> {
        let cursor = Box::new(Cursor::new(bytes));
        Self::from_media_source(cursor, hint_ext)
    }

    pub fn from_media_source(
        source: Box<dyn MediaSource>,
        hint_ext: Option<&str>,
    ) -> Result<Self, DecoderError> {
        let mss = MediaSourceStream::new(source, Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = hint_ext {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(DecoderError::Symphonia)?;

        let format_reader = probed.format;

        // Find the first supported audio track
        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or(DecoderError::NoTrackFound)?;

        let track_id = track.id;
        let time_base = track.codec_params.time_base;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
        let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

        let duration_ms = if let (Some(tb), Some(n_frames)) = (time_base, track.codec_params.n_frames) {
            let time = tb.calc_time(n_frames);
            (time.seconds * 1000) + ((time.frac * 1000.0) as u64)
        } else {
            0
        };

        let decoder_opts = DecoderOptions::default();
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(DecoderError::Symphonia)?;

        Ok(Self {
            format_reader,
            decoder,
            track_id,
            sample_rate,
            channels,
            time_base,
            duration_ms,
            sample_buf: None,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Read and decode the next packet into interleaved stereo f32 samples
    pub fn next_packet_stereo(&mut self, output: &mut Vec<f32>) -> Result<bool, DecoderError> {
        loop {
            let packet = match self.format_reader.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(ref err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false); // End of stream
                }
                Err(err) => return Err(DecoderError::Symphonia(err)),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let channels = spec.channels.count();

                    if self.sample_buf.is_none() {
                        self.sample_buf = Some(SampleBuffer::<f32>::new(
                            decoded.capacity() as u64,
                            spec,
                        ));
                    }

                    if let Some(ref mut buf) = self.sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        let samples = buf.samples();

                        if channels == 1 {
                            // Mono -> Stereo
                            for &s in samples {
                                output.push(s);
                                output.push(s);
                            }
                        } else if channels == 2 {
                            // Already stereo
                            output.extend_from_slice(samples);
                        } else if channels > 2 {
                            // Downmix or take first 2 channels
                            for frame in samples.chunks(channels) {
                                output.push(frame[0]);
                                output.push(frame[1]);
                            }
                        }
                    }

                    return Ok(true);
                }
                Err(SymphoniaError::DecodeError(err)) => {
                    log::warn!("Decode error, attempting to recover: {}", err);
                    continue;
                }
                Err(err) => return Err(DecoderError::Symphonia(err)),
            }
        }
    }

    /// Seek to a specific timestamp in milliseconds
    pub fn seek_ms(&mut self, time_ms: u64) -> Result<(), DecoderError> {
        let seconds = time_ms / 1000;
        let frac = (time_ms % 1000) as f64 / 1000.0;

        let seek_to = if let Some(tb) = self.time_base {
            let ts = tb.calc_timestamp(Time::new(seconds, frac));
            SeekTo::TimeStamp {
                ts,
                track_id: self.track_id,
            }
        } else {
            SeekTo::Time {
                time: Time::new(seconds, frac),
                track_id: Some(self.track_id),
            }
        };

        self.format_reader
            .seek(SeekMode::Accurate, seek_to)
            .map_err(DecoderError::Symphonia)?;

        self.decoder.reset();
        Ok(())
    }
}
