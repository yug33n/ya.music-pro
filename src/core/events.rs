use crate::core::types::{LyricsLine, PlaybackState, Track};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CoreEvent {
    TrackChanged(Option<Track>),
    PlaybackStateChanged(PlaybackState),
    PositionChanged {
        position_ms: u64,
        duration_ms: u64,
    },
    VolumeChanged {
        volume: f32,
        muted: bool,
    },
    QueueUpdated(Vec<Track>),
    LikeStatusChanged {
        track_id: String,
        is_liked: bool,
    },
    AuthStatusChanged {
        authorized: bool,
        user_name: Option<String>,
    },
    DeviceCodeReceived {
        user_code: String,
        verification_url: String,
    },
    DynamicColorExtracted(String),
    LyricsLoaded(Vec<LyricsLine>),
    CacheProgress {
        track_id: String,
        progress: f32,
    },
    AudioDuckingTriggered(bool),
    Notification {
        title: String,
        message: String,
    },
    Error(String),
}
