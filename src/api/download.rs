use crate::api::error::ApiError;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

pub const YANDEX_MUSIC_SALT: &str = "XGRSTTXSSKIYSWS051AgYZ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfoItem {
    pub codec: String,
    pub bitrate_in_kbps: Option<u32>,
    pub download_info_url: String,
    pub direct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename = "download-info")]
pub struct XmlDownloadInfo {
    pub host: String,
    pub path: String,
    pub ts: String,
    pub s: String,
}

/// Parse download-info XML returned by Yandex storage
pub fn parse_download_info_xml(xml: &str) -> Result<XmlDownloadInfo, ApiError> {
    quick_xml::de::from_str::<XmlDownloadInfo>(xml).map_err(ApiError::from)
}

/// Sign download-info parameters using MD5 and Yandex salt to produce a direct stream URL
pub fn sign_download_url(info: &XmlDownloadInfo) -> String {
    let stripped_path = info.path.strip_prefix('/').unwrap_or(&info.path);
    let to_sign = format!("{}{}{}", YANDEX_MUSIC_SALT, stripped_path, info.s);

    let mut hasher = Md5::new();
    hasher.update(to_sign.as_bytes());
    let sign = format!("{:x}", hasher.finalize());

    let normalized_path = if info.path.starts_with('/') {
        info.path.clone()
    } else {
        format!("/{}", info.path)
    };

    format!(
        "https://{}/get-mp3/{}/{}{}",
        info.host, sign, info.ts, normalized_path
    )
}

/// Pick the best download info item based on preference (FLAC > 320kbps MP3 > 192kbps MP3)
pub fn select_best_download_item<'a>(
    items: &'a [DownloadInfoItem],
    prefer_flac: bool,
) -> Option<&'a DownloadInfoItem> {
    if prefer_flac {
        if let Some(flac) = items.iter().find(|i| i.codec.eq_ignore_ascii_case("flac")) {
            return Some(flac);
        }
    }

    // Otherwise sort/find MP3 with highest bitrate
    items
        .iter()
        .filter(|i| i.codec.eq_ignore_ascii_case("mp3"))
        .max_by_key(|i| i.bitrate_in_kbps.unwrap_or(0))
        .or_else(|| items.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xml_download_info() {
        let sample_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<download-info>
    <host>s128sas.storage.yandex.net</host>
    <path>/get-mp3/b12345/data.mp3</path>
    <ts>1732000000</ts>
    <s>0123456789abcdef0123456789abcdef</s>
</download-info>"#;

        let parsed = parse_download_info_xml(sample_xml).expect("Failed to parse XML");
        assert_eq!(parsed.host, "s128sas.storage.yandex.net");
        assert_eq!(parsed.path, "/get-mp3/b12345/data.mp3");
        assert_eq!(parsed.ts, "1732000000");
        assert_eq!(parsed.s, "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn test_sign_download_url_reproducible() {
        let info = XmlDownloadInfo {
            host: "s128sas.storage.yandex.net".to_string(),
            path: "/get-mp3/b12345/data.mp3".to_string(),
            ts: "1732000000".to_string(),
            s: "0123456789abcdef0123456789abcdef".to_string(),
        };

        let signed_url = sign_download_url(&info);
        assert!(signed_url.starts_with("https://s128sas.storage.yandex.net/get-mp3/"));
        assert!(signed_url.contains("/1732000000/get-mp3/b12345/data.mp3"));

        // Verify MD5 hash match
        let raw_to_hash = format!(
            "{}{}{}",
            YANDEX_MUSIC_SALT, "get-mp3/b12345/data.mp3", "0123456789abcdef0123456789abcdef"
        );
        let mut hasher = Md5::new();
        hasher.update(raw_to_hash.as_bytes());
        let expected_sign = format!("{:x}", hasher.finalize());

        assert!(signed_url.contains(&expected_sign));
    }

    #[test]
    fn test_select_best_download_item() {
        let items = vec![
            DownloadInfoItem {
                codec: "mp3".to_string(),
                bitrate_in_kbps: Some(192),
                download_info_url: "http://example.com/192".to_string(),
                direct: false,
            },
            DownloadInfoItem {
                codec: "mp3".to_string(),
                bitrate_in_kbps: Some(320),
                download_info_url: "http://example.com/320".to_string(),
                direct: false,
            },
            DownloadInfoItem {
                codec: "flac".to_string(),
                bitrate_in_kbps: Some(1411),
                download_info_url: "http://example.com/flac".to_string(),
                direct: true,
            },
        ];

        // When preferring FLAC
        let best_flac = select_best_download_item(&items, true).unwrap();
        assert_eq!(best_flac.codec, "flac");

        // When not preferring FLAC
        let best_mp3 = select_best_download_item(&items, false).unwrap();
        assert_eq!(best_mp3.codec, "mp3");
        assert_eq!(best_mp3.bitrate_in_kbps, Some(320));
    }
}
