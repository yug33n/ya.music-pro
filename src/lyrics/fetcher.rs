use crate::pro_api::error::ApiError;
use crate::pro_api::models::YandexApiResponse;
use crate::pro_api::YandexApiClient;
use crate::core::types::{LyricsLine, Track};
use crate::lyrics::parser::parse_lrc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YandexSupplementResponse {
    pub lyrics: Option<YandexLyricsInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YandexLyricsInfo {
    pub id: Option<serde_json::Value>,
    pub sync_type: Option<String>,
    pub lyrics: Option<String>,
    pub full_lyrics: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LrclibResponse {
    pub id: Option<u64>,
    pub track_name: Option<String>,
    pub artist_name: Option<String>,
    pub album_name: Option<String>,
    pub duration: Option<f64>,
    pub instrumental: Option<bool>,
    pub plain_lyrics: Option<String>,
    pub synced_lyrics: Option<String>,
}

pub struct LyricsFetcher;

impl LyricsFetcher {
    /// Fetch synced lyrics for a track, checking Yandex Music supplement first, then falling back to LRCLIB
    pub async fn fetch_lyrics(
        client: &YandexApiClient,
        track: &Track,
    ) -> Result<Vec<LyricsLine>, ApiError> {
        // Source 1: Yandex Music Supplement API
        if let Ok(Some(lrc_text)) = Self::fetch_yandex_supplement(client, &track.id).await {
            let parsed = parse_lrc(&lrc_text);
            if !parsed.is_empty() {
                log::info!("Loaded {} synced lyric lines from Yandex supplement", parsed.len());
                return Ok(parsed);
            }
        }

        // Source 2: LRCLIB Fallback API
        let first_artist = track.artists.first().map(|s| s.as_str()).unwrap_or("");
        if let Ok(Some(lrclib_text)) = Self::fetch_lrclib(
            client.http(),
            &track.title,
            first_artist,
            track.album.as_deref(),
            track.duration_ms / 1000,
        )
        .await
        {
            let parsed = parse_lrc(&lrclib_text);
            if !parsed.is_empty() {
                log::info!("Loaded {} synced lyric lines from LRCLIB fallback", parsed.len());
                return Ok(parsed);
            }
        }

        Ok(Vec::new())
    }

    /// Query Yandex Music track supplement
    pub async fn fetch_yandex_supplement(
        client: &YandexApiClient,
        track_id: &str,
    ) -> Result<Option<String>, ApiError> {
        let url = format!("{}/tracks/{}/supplement", crate::pro_api::client::API_BASE_URL, track_id);
        let headers = client.auth_headers().await?;

        let resp = client.http().get(&url).headers(headers).send().await?;
        if !resp.status().is_success() {
            return Ok(None);
        }

        let api_resp: YandexApiResponse<YandexSupplementResponse> = resp.json().await?;
        if let Some(supp) = api_resp.result {
            if let Some(lyrics_info) = supp.lyrics {
                if let Some(text) = lyrics_info.lyrics.or(lyrics_info.full_lyrics) {
                    if !text.trim().is_empty() {
                        return Ok(Some(text));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Query LRCLIB public lyrics service
    pub async fn fetch_lrclib(
        http: &reqwest::Client,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_sec: u64,
    ) -> Result<Option<String>, ApiError> {
        let url = "https://lrclib.net/api/get";
        let mut query = vec![
            ("track_name", title.to_string()),
            ("artist_name", artist.to_string()),
            ("duration", duration_sec.to_string()),
        ];
        if let Some(alb) = album {
            query.push(("album_name", alb.to_string()));
        }

        let resp = http.get(url).query(&query).send().await?;
        if !resp.status().is_success() {
            return Ok(None);
        }

        let lrclib: LrclibResponse = resp.json().await?;
        if let Some(synced) = lrclib.synced_lyrics {
            if !synced.trim().is_empty() {
                return Ok(Some(synced));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_lrclib_response() {
        let json_str = r#"{
            "id": 10582,
            "trackName": "Sample Track",
            "artistName": "Artist",
            "albumName": "Album",
            "duration": 210.5,
            "instrumental": false,
            "syncedLyrics": "[00:10.50] Hello world\n[00:20.00] Second line\n"
        }"#;

        let res: LrclibResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(res.id, Some(10582));
        assert!(res.synced_lyrics.is_some());
        let lrc = parse_lrc(res.synced_lyrics.as_deref().unwrap());
        assert_eq!(lrc.len(), 2);
        assert_eq!(lrc[0].timestamp_ms, 10_500);
    }
}
