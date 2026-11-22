use crate::pro_api::error::ApiError;
use crate::pro_api::models::YandexApiResponse;
use crate::pro_api::YandexApiClient;
use crate::core::types::{QualityBadge, Track as CoreTrack, VibeSettings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YandexArtist {
    #[serde(default)]
    pub id: serde_json::Value,
    pub name: String,
}

impl YandexArtist {
    pub fn id_string(&self) -> String {
        match &self.id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YandexAlbum {
    #[serde(default)]
    pub id: serde_json::Value,
    pub title: String,
    pub year: Option<u32>,
    pub genre: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YandexTrack {
    #[serde(default)]
    pub id: serde_json::Value,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<YandexArtist>,
    #[serde(default)]
    pub albums: Vec<YandexAlbum>,
    pub duration_ms: Option<u64>,
    pub cover_uri: Option<String>,
    pub og_image: Option<String>,
    pub content_warning: Option<String>,
    pub available: Option<bool>,
}

impl YandexTrack {
    pub fn id_string(&self) -> String {
        match &self.id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            _ => String::new(),
        }
    }

    pub fn to_core_track(&self, is_liked: bool) -> CoreTrack {
        let artists_list: Vec<String> = self.artists.iter().map(|a| a.name.clone()).collect();
        let album_title = self.albums.first().map(|a| a.title.clone());

        let cover_url = self
            .cover_uri
            .as_ref()
            .or(self.og_image.as_ref())
            .map(|uri| {
                if uri.contains("%%") {
                    format!("https://{}", uri.replace("%%", "400x400"))
                } else if uri.starts_with("//") {
                    format!("https:{}", uri)
                } else if !uri.starts_with("http") {
                    format!("https://{}", uri)
                } else {
                    uri.clone()
                }
            });

        let explicit = self
            .content_warning
            .as_deref()
            .map(|cw| cw.eq_ignore_ascii_case("explicit"))
            .unwrap_or(false);

        CoreTrack {
            id: self.id_string(),
            title: self.title.clone(),
            artists: artists_list,
            album: album_title,
            duration_ms: self.duration_ms.unwrap_or(0),
            cover_uri: cover_url,
            explicit,
            is_liked,
            quality: QualityBadge::HQ,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YandexPlaylistTrackItem {
    pub id: Option<serde_json::Value>,
    pub track: Option<YandexTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YandexPlaylist {
    pub uid: serde_json::Value,
    pub kind: serde_json::Value,
    pub title: String,
    pub description: Option<String>,
    pub track_count: Option<usize>,
    pub cover_uri: Option<String>,
    pub tracks: Option<Vec<YandexPlaylistTrackItem>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikedTrackShort {
    pub id: serde_json::Value,
    pub album_id: Option<serde_json::Value>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikesLibrary {
    pub uid: Option<serde_json::Value>,
    #[serde(default)]
    pub tracks: Vec<LikedTrackShort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotorTrackItem {
    pub track: YandexTrack,
    pub liked: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotorStationTracksResponse {
    pub station: Option<serde_json::Value>,
    #[serde(default)]
    pub sequence: Vec<RotorTrackItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultTracks {
    #[serde(default)]
    pub items: Vec<YandexTrack>,
    pub total: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultAlbums {
    #[serde(default)]
    pub items: Vec<YandexAlbum>,
    pub total: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultArtists {
    #[serde(default)]
    pub items: Vec<YandexArtist>,
    pub total: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultPlaylists {
    #[serde(default)]
    pub items: Vec<YandexPlaylist>,
    pub total: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YandexSearchResponse {
    pub text: Option<String>,
    pub tracks: Option<SearchResultTracks>,
    pub albums: Option<SearchResultAlbums>,
    pub artists: Option<SearchResultArtists>,
    pub playlists: Option<SearchResultPlaylists>,
}

// Catalog operations for YandexApiClient
impl YandexApiClient {
    /// Get list of liked track IDs
    pub async fn get_liked_track_ids(&self, user_id: &str) -> Result<Vec<String>, ApiError> {
        let url = format!("{}/users/{}/likes/tracks", super::client::API_BASE_URL, user_id);
        let headers = self.auth_headers().await?;

        let resp = self.http().get(&url).headers(headers).send().await?.error_for_status()?;
        let api_resp: YandexApiResponse<LikesLibrary> = resp.json().await?;

        let library = api_resp.result.ok_or_else(|| ApiError::Yandex("No likes library in response".to_string()))?;
        let ids = library
            .tracks
            .into_iter()
            .map(|t| match t.id {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s,
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect();

        Ok(ids)
    }

    /// Fetch detailed track objects by IDs
    pub async fn get_tracks_by_ids(&self, track_ids: &[String]) -> Result<Vec<CoreTrack>, ApiError> {
        if track_ids.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/tracks", super::client::API_BASE_URL);
        let headers = self.auth_headers().await?;

        // Yandex accepts form track-ids=1,2,3...
        let ids_joined = track_ids.join(",");
        let params = [("track-ids", ids_joined.as_str())];

        let resp = self
            .http()
            .post(&url)
            .headers(headers)
            .form(&params)
            .send()
            .await?
            .error_for_status()?;

        let api_resp: YandexApiResponse<Vec<YandexTrack>> = resp.json().await?;
        let tracks = api_resp.result.unwrap_or_default();

        Ok(tracks.into_iter().map(|t| t.to_core_track(false)).collect())
    }

    /// Fetch user's playlists
    pub async fn get_user_playlists(&self, user_id: &str) -> Result<Vec<YandexPlaylist>, ApiError> {
        let url = format!("{}/users/{}/playlists/list", super::client::API_BASE_URL, user_id);
        let headers = self.auth_headers().await?;

        let resp = self.http().get(&url).headers(headers).send().await?.error_for_status()?;
        let api_resp: YandexApiResponse<Vec<YandexPlaylist>> = resp.json().await?;

        Ok(api_resp.result.unwrap_or_default())
    }

    /// Fetch detailed playlist with tracks
    pub async fn get_playlist(&self, user_id: &str, kind: &str) -> Result<YandexPlaylist, ApiError> {
        let url = format!("{}/users/{}/playlists/{}", super::client::API_BASE_URL, user_id, kind);
        let headers = self.auth_headers().await?;

        let resp = self.http().get(&url).headers(headers).send().await?.error_for_status()?;
        let api_resp: YandexApiResponse<YandexPlaylist> = resp.json().await?;

        api_resp.result.ok_or_else(|| ApiError::Yandex("Playlist not found".to_string()))
    }

    /// Perform catalog search
    pub async fn search_catalog(&self, text: &str) -> Result<YandexSearchResponse, ApiError> {
        let url = format!("{}/search", super::client::API_BASE_URL);
        let headers = self.auth_headers().await?;
        let query = [("text", text), ("type", "all"), ("page", "0")];

        let resp = self
            .http()
            .get(&url)
            .headers(headers)
            .query(&query)
            .send()
            .await?
            .error_for_status()?;

        let api_resp: YandexApiResponse<YandexSearchResponse> = resp.json().await?;
        api_resp.result.ok_or_else(|| ApiError::Yandex("Search error".to_string()))
    }

    /// Fetch My Wave / Rotor station tracks with optional vibe settings
    pub async fn get_my_wave_tracks(
        &self,
        station_id: &str,
        vibe: &VibeSettings,
    ) -> Result<Vec<CoreTrack>, ApiError> {
        let url = format!("{}/rotor/station/{}/tracks", super::client::API_BASE_URL, station_id);
        let headers = self.auth_headers().await?;

        let query = [
            ("mood", vibe.mood.as_str()),
            ("character", vibe.character.as_str()),
            ("language", vibe.language.as_str()),
        ];

        let resp = self
            .http()
            .get(&url)
            .headers(headers)
            .query(&query)
            .send()
            .await?
            .error_for_status()?;

        let api_resp: YandexApiResponse<RotorStationTracksResponse> = resp.json().await?;
        let rotor_resp = api_resp
            .result
            .ok_or_else(|| ApiError::Yandex("No tracks in My Wave station".to_string()))?;

        let tracks = rotor_resp
            .sequence
            .into_iter()
            .map(|item| {
                let liked = item.liked.unwrap_or(false);
                item.track.to_core_track(liked)
            })
            .collect();

        Ok(tracks)
    }

    /// Like or dislike track
    pub async fn set_track_like(&self, user_id: &str, track_id: &str, like: bool) -> Result<(), ApiError> {
        let action = if like { "add-multiple" } else { "remove" };
        let url = format!("{}/users/{}/likes/tracks/{}", super::client::API_BASE_URL, user_id, action);
        let headers = self.auth_headers().await?;
        let params = [("track-ids", track_id)];

        self.http()
            .post(&url)
            .headers(headers)
            .form(&params)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yandex_track_to_core_conversion() {
        let y_track = YandexTrack {
            id: serde_json::json!(123456),
            title: "Test Track".to_string(),
            artists: vec![
                YandexArtist {
                    id: serde_json::json!(101),
                    name: "Artist One".to_string(),
                },
                YandexArtist {
                    id: serde_json::json!(102),
                    name: "Artist Two".to_string(),
                },
            ],
            albums: vec![YandexAlbum {
                id: serde_json::json!(501),
                title: "Awesome Album".to_string(),
                year: Some(2024),
                genre: Some("Electronic".to_string()),
            }],
            duration_ms: Some(215_000),
            cover_uri: Some("avatars.yandex.net/get-music-content/123/%%".to_string()),
            og_image: None,
            content_warning: Some("explicit".to_string()),
            available: Some(true),
        };

        let core_track = y_track.to_core_track(true);
        assert_eq!(core_track.id, "123456");
        assert_eq!(core_track.title, "Test Track");
        assert_eq!(core_track.artists, vec!["Artist One", "Artist Two"]);
        assert_eq!(core_track.album, Some("Awesome Album".to_string()));
        assert_eq!(core_track.duration_ms, 215_000);
        assert_eq!(
            core_track.cover_uri,
            Some("https://avatars.yandex.net/get-music-content/123/400x400".to_string())
        );
        assert!(core_track.explicit);
        assert!(core_track.is_liked);
    }
}
