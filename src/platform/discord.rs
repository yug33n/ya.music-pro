use crate::core::types::QualityBadge;
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const DEFAULT_DISCORD_APP_ID: &str = "1093126982977450005"; // Yandex Music Desktop Client RPC ID

#[derive(Error, Debug)]
pub enum DiscordError {
    #[error("Discord IPC communication error: {0}")]
    Ipc(String),

    #[error("Discord client not connected")]
    NotConnected,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscordTrackActivity {
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub quality: QualityBadge,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub is_playing: bool,
}

pub struct DiscordRpcService {
    client: Option<DiscordIpcClient>,
    app_id: String,
    is_connected: bool,
    is_enabled: bool,
}

impl DiscordRpcService {
    pub fn new(app_id: Option<&str>) -> Self {
        Self {
            client: None,
            app_id: app_id.unwrap_or(DEFAULT_DISCORD_APP_ID).to_string(),
            is_connected: false,
            is_enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.is_enabled = enabled;
        if !enabled {
            let _ = self.clear_activity();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }

    /// Try to connect to Discord desktop IPC socket
    pub fn connect(&mut self) -> Result<(), DiscordError> {
        if self.is_connected {
            return Ok(());
        }

        let mut client = DiscordIpcClient::new(&self.app_id)
            .map_err(|e| DiscordError::Ipc(format!("Failed to create IPC client: {}", e)))?;

        match client.connect() {
            Ok(()) => {
                self.client = Some(client);
                self.is_connected = true;
                log::info!("Connected to Discord Rich Presence IPC");
                Ok(())
            }
            Err(e) => {
                self.is_connected = false;
                Err(DiscordError::Ipc(format!("Failed to connect to Discord IPC: {}", e)))
            }
        }
    }

    /// Format quality badge for Discord small asset
    pub fn quality_badge_asset(quality: QualityBadge) -> (&'static str, &'static str) {
        match quality {
            QualityBadge::BitPerfect => ("badge_bitperfect", "Bit-Perfect (WASAPI Exclusive)"),
            QualityBadge::FLAC => ("badge_flac", "FLAC (Lossless Audio)"),
            QualityBadge::HQ => ("badge_hq", "HQ Audio (320 kbps)"),
            QualityBadge::Standard => ("badge_standard", "Standard Quality"),
        }
    }

    /// Build Activity payload for Discord RPC
    pub fn build_activity<'a>(track: &'a DiscordTrackActivity) -> activity::Activity<'a> {
        let (badge_icon, badge_text) = Self::quality_badge_asset(track.quality);

        let cover_key = track
            .cover_url
            .as_deref()
            .unwrap_or("yandex_music_logo");

        let assets = activity::Assets::new()
            .large_image(cover_key)
            .large_text(&track.title)
            .small_image(badge_icon)
            .small_text(badge_text);

        let mut act = activity::Activity::new()
            .details(&track.title)
            .state(&track.artists)
            .assets(assets);

        if track.is_playing && track.duration_ms > 0 {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            let elapsed_secs = (track.position_ms / 1000) as i64;
            let start_time = now.saturating_sub(elapsed_secs);
            let total_secs = (track.duration_ms / 1000) as i64;
            let end_time = start_time + total_secs;

            let timestamps = activity::Timestamps::new()
                .start(start_time)
                .end(end_time);

            act = act.timestamps(timestamps);
        }

        act
    }

    /// Update current playing activity in Discord
    pub fn update_activity(&mut self, track: &DiscordTrackActivity) -> Result<(), DiscordError> {
        if !self.is_enabled {
            return Ok(());
        }

        if !self.is_connected {
            let _ = self.connect();
        }

        let client = match self.client.as_mut() {
            Some(c) => c,
            None => return Err(DiscordError::NotConnected),
        };

        let act = Self::build_activity(track);
        match client.set_activity(act) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.is_connected = false;
                self.client = None;
                Err(DiscordError::Ipc(format!("Failed to set activity: {}", e)))
            }
        }
    }

    /// Clear current activity
    pub fn clear_activity(&mut self) -> Result<(), DiscordError> {
        if let Some(client) = self.client.as_mut() {
            let _ = client.clear_activity();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quality_badge_assets() {
        let (icon_bp, text_bp) = DiscordRpcService::quality_badge_asset(QualityBadge::BitPerfect);
        assert_eq!(icon_bp, "badge_bitperfect");
        assert!(text_bp.contains("Bit-Perfect"));

        let (icon_flac, _) = DiscordRpcService::quality_badge_asset(QualityBadge::FLAC);
        assert_eq!(icon_flac, "badge_flac");

        let (icon_hq, _) = DiscordRpcService::quality_badge_asset(QualityBadge::HQ);
        assert_eq!(icon_hq, "badge_hq");
    }

    #[test]
    fn test_build_activity_structure() {
        let track = DiscordTrackActivity {
            title: "Stereo Love".to_string(),
            artists: "Edward Maya, Vika Jigulina".to_string(),
            album: Some("Stereo Love Show".to_string()),
            cover_url: Some("https://avatars.yandex.net/get-music-content/123/cover/400x400".to_string()),
            quality: QualityBadge::BitPerfect,
            position_ms: 30_000,
            duration_ms: 240_000,
            is_playing: true,
        };

        let act = DiscordRpcService::build_activity(&track);
        let _ = act; // Successfully constructed without panic
    }

    #[test]
    fn test_discord_service_enabled_toggle() {
        let mut service = DiscordRpcService::new(None);
        assert!(service.is_enabled());
        assert!(!service.is_connected());

        service.set_enabled(false);
        assert!(!service.is_enabled());
    }
}
