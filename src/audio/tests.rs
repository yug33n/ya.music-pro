#[cfg(test)]
mod tests {
    use crate::audio::decoder::SymphoniaDecoder;
    use crate::audio::device::DeviceManager;
    use crate::audio::resampler::AudioResampler;

    #[test]
    fn test_device_manager_query() {
        let mgr = DeviceManager::new();
        let devices = mgr.list_output_devices();
        println!("Available audio devices count: {}", devices.len());
    }

    #[test]
    fn test_resampler_streaming_stereo() {
        let mut resampler = AudioResampler::new(44100, 48000);
        let mut input = Vec::new();
        // 1000 frames of 440 Hz sine wave
        for i in 0..1000 {
            let s = (i as f32 * 0.1).sin();
            input.push(s);     // Left
            input.push(s * 0.5); // Right
        }

        let mut output = Vec::new();
        resampler.process_interleaved_stereo(&input, &mut output);

        assert!(!output.is_empty());
        // Verify stereo pairs
        assert_eq!(output.len() % 2, 0);
    }

    #[test]
    fn test_decoder_invalid_bytes_graceful_handling() {
        let dummy_bytes = vec![0u8; 128];
        let res = SymphoniaDecoder::from_bytes(dummy_bytes, Some("mp3"));
        assert!(res.is_err());
    }
}
