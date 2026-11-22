use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum QualityBadge {
    #[default]
    Standard,
    HQ,
    FLAC,
    BitPerfect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
    Buffering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    Track,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ShuffleMode {
    #[default]
    Off,
    On,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioOutputMode {
    #[default]
    Shared,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub cover_uri: Option<String>,
    pub explicit: bool,
    pub is_liked: bool,
    pub quality: QualityBadge,
}

impl Track {
    pub fn artists_display(&self) -> String {
        self.artists.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsLine {
    pub timestamp_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VibeSettings {
    pub character: String, // e.g. "energetic", "calm", "favorite", "unfamiliar"
    pub mood: String,      // e.g. "joyful", "sad", "energetic"
    pub language: String,  // e.g. "russian", "foreign", "any"
}

impl Default for VibeSettings {
    fn default() -> Self {
        Self {
            character: "favorite".to_string(),
            mood: "joyful".to_string(),
            language: "any".to_string(),
        }
    }
}
