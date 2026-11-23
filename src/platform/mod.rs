pub mod discord;
pub mod ducking;
pub mod export;
pub mod hotkeys;
pub mod tray;

pub use discord::{DiscordRpcService, DiscordTrackActivity};
pub use ducking::{
    start_voice_activity_monitor, SessionMeterInfo, SessionMonitorError, WindowsSessionMonitor,
};
pub use export::{export_track_with_metadata, ExportConfig, ExportError};
pub use hotkeys::{cyrillic_to_latin, parse_hotkey_string, GlobalHotkeyAction, GlobalHotkeysEngine};
pub use tray::{HeadlessTrayMode, TrayEventAction};