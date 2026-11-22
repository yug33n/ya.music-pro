use crate::core::types::{AudioOutputMode, PlaybackState, RepeatMode, ShuffleMode, Track, VibeSettings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub current_track: Option<Track>,
    pub playback_state: PlaybackState,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub is_muted: bool,
    pub repeat_mode: RepeatMode,
    pub shuffle_mode: ShuffleMode,
    pub output_mode: AudioOutputMode,
    pub queue: Vec<Track>,
    pub current_queue_index: Option<usize>,
    pub active_eq_preset: String,
    pub eq_enabled: bool,
    pub eq_gains: [f32; 10],
    pub ducking_enabled: bool,
    pub slowed_reverb_enabled: bool,
    pub lufs_normalization_enabled: bool,
    pub crossfade_seconds: u32,
    pub gapless_enabled: bool,
    pub active_vibe: VibeSettings,
    pub is_authorized: bool,
    pub user_name: Option<String>,
    pub dynamic_accent_color: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            current_track: None,
            playback_state: PlaybackState::Stopped,
            position_ms: 0,
            duration_ms: 0,
            volume: 0.8,
            is_muted: false,
            repeat_mode: RepeatMode::Off,
            shuffle_mode: ShuffleMode::Off,
            output_mode: AudioOutputMode::Shared,
            queue: Vec::new(),
            current_queue_index: None,
            active_eq_preset: "Flat".to_string(),
            eq_enabled: false,
            eq_gains: [0.0; 10],
            ducking_enabled: true,
            slowed_reverb_enabled: false,
            lufs_normalization_enabled: true,
            crossfade_seconds: 3,
            gapless_enabled: true,
            active_vibe: VibeSettings::default(),
            is_authorized: false,
            user_name: None,
            dynamic_accent_color: "#FFCC00".to_string(), // Default Yandex Yellow
        }
    }
}

impl AppState {
    pub fn set_track(&mut self, track: Track) {
        self.duration_ms = track.duration_ms;
        self.position_ms = 0;
        self.current_track = Some(track);
        self.playback_state = PlaybackState::Playing;
    }

    pub fn set_playback_state(&mut self, state: PlaybackState) {
        self.playback_state = state;
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if self.volume > 0.0 {
            self.is_muted = false;
        }
    }

    pub fn toggle_mute(&mut self) {
        self.is_muted = !self.is_muted;
    }

    pub fn set_position(&mut self, position_ms: u64) {
        self.position_ms = position_ms.min(self.duration_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::QualityBadge;

    #[test]
    fn test_app_state_default() {
        let state = AppState::default();
        assert_eq!(state.playback_state, PlaybackState::Stopped);
        assert_eq!(state.volume, 0.8);
        assert!(!state.is_muted);
        assert_eq!(state.dynamic_accent_color, "#FFCC00");
    }

    #[test]
    fn test_app_state_track_set() {
        let mut state = AppState::default();
        let track = Track {
            id: "123".to_string(),
            title: "Test Track".to_string(),
            artists: vec!["Test Artist".to_string()],
            album: Some("Test Album".to_string()),
            duration_ms: 180_000,
            cover_uri: None,
            explicit: false,
            is_liked: false,
            quality: QualityBadge::FLAC,
        };
        state.set_track(track.clone());
        assert_eq!(state.current_track, Some(track));
        assert_eq!(state.playback_state, PlaybackState::Playing);
        assert_eq!(state.duration_ms, 180_000);
        assert_eq!(state.position_ms, 0);
    }

    #[test]
    fn test_volume_clamp_and_mute() {
        let mut state = AppState::default();
        state.set_volume(1.5);
        assert_eq!(state.volume, 1.0);
        state.set_volume(-0.5);
        assert_eq!(state.volume, 0.0);
        state.toggle_mute();
        assert!(state.is_muted);
        state.set_volume(0.5);
        assert!(!state.is_muted);
    }
}
