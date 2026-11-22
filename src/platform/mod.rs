pub mod discord;
pub mod ducking;
pub mod export;

pub use discord::{DiscordRpcService, DiscordTrackActivity};

pub use ducking::{
    start_voice_activity_monitor, SessionMeterInfo, SessionMonitorError, WindowsSessionMonitor,
};
pub use export::{export_track_with_metadata, ExportConfig, ExportError};
