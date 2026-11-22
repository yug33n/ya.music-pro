use crate::api::YandexApiClient;
use crate::core::types::Track;
use crate::db::{CacheEntry, DatabaseRepository};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("API error: {0}")]
    Api(String),
}

/// Standard cache quota presets
pub const QUOTA_5_GB: u64 = 5 * 1024 * 1024 * 1024;
pub const QUOTA_10_GB: u64 = 10 * 1024 * 1024 * 1024;
pub const QUOTA_25_GB: u64 = 25 * 1024 * 1024 * 1024;

/// Predictive audio stream pre-caching engine with LRU disk quota management
pub struct PrecacheManager {
    cache_dir: PathBuf,
    max_quota_bytes: u64,
    db: Arc<DatabaseRepository>,
    api_client: YandexApiClient,
}

impl PrecacheManager {
    pub fn new(
        cache_dir: impl AsRef<Path>,
        max_quota_bytes: u64,
        db: Arc<DatabaseRepository>,
        api_client: YandexApiClient,
    ) -> Self {
        let path = cache_dir.as_ref().to_path_buf();
        let _ = fs::create_dir_all(&path);

        Self {
            cache_dir: path,
            max_quota_bytes,
            db,
            api_client,
        }
    }

    pub fn set_quota(&mut self, quota_bytes: u64) {
        self.max_quota_bytes = quota_bytes;
        let _ = self.enforce_quota();
    }

    pub fn quota(&self) -> u64 {
        self.max_quota_bytes
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Retrieve path to cached track file if available on disk and update LRU timestamp
    pub fn get_cached_track_path(&self, track_id: &str) -> Option<PathBuf> {
        let entry = self.db.get_cache_entry(track_id).ok()??;
        let path = PathBuf::from(&entry.file_path);
        if path.exists() {
            let _ = self.db.touch_cache_entry(track_id);
            Some(path)
        } else {
            // File missing on disk, clean up dangling DB entry
            let _ = self.db.delete_cache_entry(track_id);
            None
        }
    }

    /// Save audio bytes to disk cache and update database entry
    pub fn store_track_bytes(
        &self,
        track_id: &str,
        format: &str,
        bitrate: Option<u32>,
        bytes: &[u8],
    ) -> Result<PathBuf, CacheError> {
        let file_name = format!("{}.{}", track_id, format);
        let target_path = self.cache_dir.join(&file_name);

        fs::write(&target_path, bytes)?;

        let now = chrono::Utc::now().timestamp();
        let entry = CacheEntry {
            track_id: track_id.to_string(),
            file_path: target_path.to_string_lossy().to_string(),
            file_size: bytes.len() as u64,
            format: format.to_string(),
            bitrate,
            last_accessed_at: now,
            created_at: now,
        };

        self.db.insert_or_update_cache(&entry)?;
        self.enforce_quota()?;

        Ok(target_path)
    }

    /// Enforce LRU disk quota: if total cache exceeds quota, evict oldest accessed tracks
    pub fn enforce_quota(&self) -> Result<u64, CacheError> {
        let mut total_size = self.db.get_total_cache_size()?;

        if total_size <= self.max_quota_bytes {
            return Ok(total_size);
        }

        let lru_entries = self.db.list_lru_cache_entries()?;

        for entry in lru_entries {
            if total_size <= (self.max_quota_bytes * 9 / 10) {
                // Reached comfortable buffer headroom (90% of quota)
                break;
            }

            let path = Path::new(&entry.file_path);
            if path.exists() {
                let _ = fs::remove_file(path);
            }

            let _ = self.db.delete_cache_entry(&entry.track_id);
            total_size = total_size.saturating_sub(entry.file_size);
        }

        Ok(total_size)
    }

    /// Pre-cache next N tracks (e.g. 5-10) ahead in the queue or Wave
    pub async fn precache_upcoming_tracks(
        &self,
        upcoming_tracks: &[Track],
        count: usize,
    ) -> Result<usize, CacheError> {
        let to_cache = &upcoming_tracks[..upcoming_tracks.len().min(count)];
        let mut cached_count = 0;

        for track in to_cache {
            // Skip if already cached
            if self.get_cached_track_path(&track.id).is_some() {
                cached_count += 1;
                continue;
            }

            // Resolve direct signed URL from Yandex Music API
            let signed_url = match self.api_client.get_track_direct_url(&track.id, true).await {
                Ok(url) => url,
                Err(e) => {
                    log::debug!("Pre-cache URL resolution failed for {}: {}", track.id, e);
                    continue;
                }
            };

            // Download audio stream bytes
            let response = match reqwest::get(&signed_url).await {
                Ok(resp) => resp,
                Err(e) => {
                    log::debug!("Pre-cache HTTP download failed for {}: {}", track.id, e);
                    continue;
                }
            };

            let bytes = match response.bytes().await {
                Ok(b) => b,
                Err(_) => continue,
            };

            let format = if signed_url.contains(".flac") { "flac" } else { "mp3" };
            if self.store_track_bytes(&track.id, format, Some(320), &bytes).is_ok() {
                cached_count += 1;
            }
        }

        Ok(cached_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::QualityBadge;

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("yamusic_test_cache").join(name);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_store_and_retrieve_cached_track() {
        let dir = temp_test_dir("store_retrieve");
        let db = Arc::new(DatabaseRepository::in_memory().unwrap());
        let api = YandexApiClient::new();
        let manager = PrecacheManager::new(&dir, QUOTA_5_GB, db, api);

        let data = vec![0x49, 0x44, 0x33, 0x04]; // ID3 dummy tag
        let path = manager.store_track_bytes("track_123", "mp3", Some(320), &data).unwrap();

        assert!(path.exists());
        let retrieved = manager.get_cached_track_path("track_123");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), path);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_lru_quota_eviction() {
        let dir = temp_test_dir("lru_eviction");
        let db = Arc::new(DatabaseRepository::in_memory().unwrap());
        let api = YandexApiClient::new();

        // Small 250 byte quota
        let manager = PrecacheManager::new(&dir, 250, db, api);

        let data1 = vec![1u8; 100];
        let data2 = vec![2u8; 100];
        let data3 = vec![3u8; 100];

        let p1 = manager.store_track_bytes("t1", "mp3", Some(320), &data1).unwrap();
        let p2 = manager.store_track_bytes("t2", "mp3", Some(320), &data2).unwrap();

        assert!(p1.exists());
        assert!(p2.exists());

        // Store 3rd track -> total becomes 300 bytes > 250 byte quota -> triggers LRU eviction of t1
        let p3 = manager.store_track_bytes("t3", "mp3", Some(320), &data3).unwrap();

        assert!(p3.exists());
        // Oldest track t1 should have been evicted
        assert!(!p1.exists());
        assert!(manager.get_cached_track_path("t1").is_none());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_quota_presets() {
        assert_eq!(QUOTA_5_GB, 5 * 1024 * 1024 * 1024);
        assert_eq!(QUOTA_10_GB, 10 * 1024 * 1024 * 1024);
        assert_eq!(QUOTA_25_GB, 25 * 1024 * 1024 * 1024);
    }
}
