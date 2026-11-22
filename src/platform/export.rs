use crate::core::types::Track;
use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag, TagType};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ExportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Lofty metadata tagging error: {0}")]
    Lofty(#[from] lofty::error::LoftyError),

    #[error("Invalid export configuration: {0}")]
    Config(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    pub destination_dir: PathBuf,
    pub format: String, // "mp3" or "flac"
    pub embed_cover: bool,
    pub save_lrc: bool,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            destination_dir: PathBuf::from("./exports"),
            format: "mp3".to_string(),
            embed_cover: true,
            save_lrc: true,
        }
    }
}

/// Sanitize file name for Windows / Linux / macOS file systems
pub fn sanitize_filename(name: &str) -> String {
    let invalid_chars = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut clean: String = name
        .chars()
        .map(|c| if invalid_chars.contains(&c) { '_' } else { c })
        .collect();

    // Trim trailing periods and spaces (invalid on Windows)
    while clean.ends_with('.') || clean.ends_with(' ') {
        clean.pop();
    }

    if clean.is_empty() {
        "unnamed_track".to_string()
    } else {
        clean
    }
}

/// Export audio file with full Lofty ID3v2/Vorbis tags, embedded high-res cover, and synced .lrc file
pub fn export_track_with_metadata(
    track: &Track,
    audio_bytes: &[u8],
    cover_bytes: Option<&[u8]>,
    lrc_content: Option<&str>,
    config: &ExportConfig,
) -> Result<(PathBuf, Option<PathBuf>), ExportError> {
    fs::create_dir_all(&config.destination_dir)?;

    let base_name = sanitize_filename(&format!("{} - {}", track.artists_display(), track.title));
    let ext = config.format.to_lowercase();
    let audio_path = config.destination_dir.join(format!("{}.{}", base_name, ext));

    // 1. Write audio stream bytes to disk
    fs::write(&audio_path, audio_bytes)?;

    // 2. Tag with Lofty metadata
    let tag_type = match ext.as_str() {
        "flac" => TagType::VorbisComments,
        _ => TagType::Id3v2,
    };

    let mut tag = Tag::new(tag_type);
    tag.insert_text(ItemKey::TrackTitle, track.title.clone());
    tag.insert_text(ItemKey::TrackArtist, track.artists_display());

    if let Some(ref album) = track.album {
        tag.insert_text(ItemKey::AlbumTitle, album.clone());
    }

    // Embed Hi-Res cover art if provided
    if config.embed_cover {
        if let Some(cover_data) = cover_bytes {
            let mime = if cover_data.starts_with(b"\x89PNG") {
                MimeType::Png
            } else {
                MimeType::Jpeg
            };

            let pic = Picture::new_unchecked(
                PictureType::CoverFront,
                Some(mime),
                None,
                cover_data.to_vec(),
            );
            tag.push_picture(pic);
        }
    }

    // Save tag directly to the newly written audio file
    tag.save_to_path(&audio_path, WriteOptions::default())?;

    // 3. Save accompanying synchronized .lrc file if requested
    let mut lrc_path = None;
    if config.save_lrc {
        if let Some(lrc) = lrc_content {
            if !lrc.trim().is_empty() {
                let target_lrc = config.destination_dir.join(format!("{}.lrc", base_name));
                fs::write(&target_lrc, lrc)?;
                lrc_path = Some(target_lrc);
            }
        }
    }

    Ok((audio_path, lrc_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::QualityBadge;
    use lofty::config::ParseOptions;
    use lofty::probe::Probe;

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("yamusic_test_exports").join(name);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn create_dummy_track() -> Track {
        Track {
            id: "12345".to_string(),
            title: "Stereo Love".to_string(),
            artists: vec!["Edward Maya".to_string(), "Vika Jigulina".to_string()],
            album: Some("The Stereo Love Show".to_string()),
            duration_ms: 240_000,
            cover_uri: None,
            explicit: false,
            is_liked: true,
            quality: QualityBadge::FLAC,
        }
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("AC/DC: Back in Black?"), "AC_DC_ Back in Black_");
        assert_eq!(sanitize_filename("Track 1..."), "Track 1");
        assert_eq!(sanitize_filename(""), "unnamed_track");
    }

    #[test]
    fn test_export_track_with_id3_and_lrc() {
        let dir = temp_test_dir("export_track");
        let config = ExportConfig {
            destination_dir: dir.clone(),
            format: "mp3".to_string(),
            embed_cover: true,
            save_lrc: true,
        };

        let track = create_dummy_track();
        // Valid MPEG-1 Layer III frames (128 kbps, 44.1 kHz, frame size = 417 bytes)
        let mut frame = vec![0xFF, 0xFB, 0x90, 0x64];
        frame.resize(417, 0x55);
        let mut audio_bytes = Vec::new();
        for _ in 0..5 {
            audio_bytes.extend_from_slice(&frame);
        }

        let cover_bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46]; // JPEG header
        let lrc_content = "[00:12.00] When you gonna call me\n[00:15.50] I'm waitin' for you";

        let (audio_path, lrc_path) = export_track_with_metadata(
            &track,
            &audio_bytes,
            Some(&cover_bytes),
            Some(lrc_content),
            &config,
        )
        .unwrap();

        assert!(audio_path.exists());
        assert!(lrc_path.is_some());
        let lrc_file = lrc_path.unwrap();
        assert!(lrc_file.exists());

        // Verify LRC contents
        let read_lrc = fs::read_to_string(&lrc_file).unwrap();
        assert_eq!(read_lrc, lrc_content);

        // Verify tagged metadata read back via Lofty Probe with read_properties(false)
        let tagged_file = Probe::open(&audio_path)
            .unwrap()
            .options(ParseOptions::new().read_properties(false))
            .read()
            .unwrap();

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.tags().first())
            .expect("A tag should exist on the exported file");

        assert_eq!(tag.title().as_deref(), Some("Stereo Love"));
        assert_eq!(tag.artist().as_deref(), Some("Edward Maya, Vika Jigulina"));
        assert_eq!(tag.album().as_deref(), Some("The Stereo Love Show"));
        assert_eq!(tag.pictures().len(), 1);

        let _ = fs::remove_dir_all(dir);
    }
}
