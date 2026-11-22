use crate::core::types::{AudioOutputMode, RepeatMode, ShuffleMode, Track, VibeSettings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CoreCommand {
    Play,
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
    Seek(u64), // milliseconds
    SetVolume(f32),
    ToggleMute,
    SetRepeatMode(RepeatMode),
    SetShuffleMode(ShuffleMode),
    SetOutputMode(AudioOutputMode),
    PlayTrack(Track),
    QueueTrack(Track),
    ClearQueue,
    ToggleLike(String),
    DislikeTrack(String),
    SetVibe(VibeSettings),
    SetEqPreset(String),
    SetEqCustomGain { band_idx: usize, gain_db: f32 },
    ToggleEq(bool),
    ToggleSlowedReverb(bool),
    ToggleAudioDucking(bool),
    ToggleLufsNormalization(bool),
    SetCrossfade(u32),
    BlacklistArtist(String),
    BlacklistTrack(String),
    StartAuthDeviceFlow,
    ExportTrack { track_id: String, format: String, destination_dir: String },
    Shutdown,
}
