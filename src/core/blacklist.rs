use crate::core::types::Track;
use crate::db::DatabaseRepository;
use std::collections::HashSet;
use std::sync::Arc;

/// High-performance Blacklist Engine with in-memory caching and SQLite persistence
pub struct BlacklistEngine {
    repo: Option<Arc<DatabaseRepository>>,
    blocked_track_ids: HashSet<String>,
    blocked_artists: HashSet<String>,
    blocked_genres: HashSet<String>,
    stopwords: Vec<String>,
    block_explicit: bool,
}

impl BlacklistEngine {
    pub fn new(repo: Option<Arc<DatabaseRepository>>) -> Self {
        let mut engine = Self {
            repo,
            blocked_track_ids: HashSet::new(),
            blocked_artists: HashSet::new(),
            blocked_genres: HashSet::new(),
            stopwords: Vec::new(),
            block_explicit: false,
        };

        if engine.repo.is_some() {
            let _ = engine.reload_from_db();
        }

        // Add standard built-in stopword heuristics for clean Wave experience
        if engine.stopwords.is_empty() {
            engine.add_default_stopwords();
        }

        engine
    }

    fn add_default_stopwords(&mut self) {
        let defaults = ["remix", "sped up", "slowed", "karaoke", "tribute", "nightcore", "instrumental"];
        for word in defaults {
            if !self.stopwords.contains(&word.to_string()) {
                self.stopwords.push(word.to_string());
            }
        }
    }

    /// Reload all blacklist rules from the SQLite database into high-speed memory cache
    pub fn reload_from_db(&mut self) -> Result<(), rusqlite::Error> {
        let repo = match &self.repo {
            Some(r) => r.clone(),
            None => return Ok(()),
        };

        let rules = repo.list_blacklist_rules()?;
        self.blocked_track_ids.clear();
        self.blocked_artists.clear();
        self.blocked_genres.clear();
        self.stopwords.clear();
        self.block_explicit = false;

        for rule in rules {
            match rule.rule_type.as_str() {
                "track" => {
                    self.blocked_track_ids.insert(rule.pattern.to_lowercase());
                }
                "artist" => {
                    self.blocked_artists.insert(rule.pattern.to_lowercase());
                }
                "genre" => {
                    self.blocked_genres.insert(rule.pattern.to_lowercase());
                }
                "stopword" => {
                    self.stopwords.push(rule.pattern.to_lowercase());
                }
                "explicit" => {
                    self.block_explicit = true;
                }
                _ => {}
            }
        }

        if self.stopwords.is_empty() {
            self.add_default_stopwords();
        }

        Ok(())
    }

    pub fn block_artist(&mut self, artist: &str, reason: Option<&str>) {
        let lower = artist.trim().to_lowercase();
        self.blocked_artists.insert(lower.clone());
        if let Some(ref repo) = self.repo {
            let _ = repo.add_blacklist_rule("artist", &lower, reason);
        }
    }

    pub fn block_track(&mut self, track_id: &str, reason: Option<&str>) {
        let lower = track_id.trim().to_lowercase();
        self.blocked_track_ids.insert(lower.clone());
        if let Some(ref repo) = self.repo {
            let _ = repo.add_blacklist_rule("track", &lower, reason);
        }
    }

    pub fn block_stopword(&mut self, stopword: &str) {
        let lower = stopword.trim().to_lowercase();
        if !self.stopwords.contains(&lower) {
            self.stopwords.push(lower.clone());
            if let Some(ref repo) = self.repo {
                let _ = repo.add_blacklist_rule("stopword", &lower, None);
            }
        }
    }

    pub fn block_genre(&mut self, genre: &str) {
        let lower = genre.trim().to_lowercase();
        self.blocked_genres.insert(lower.clone());
        if let Some(ref repo) = self.repo {
            let _ = repo.add_blacklist_rule("genre", &lower, None);
        }
    }

    pub fn set_block_explicit(&mut self, block: bool) {
        self.block_explicit = block;
        if let Some(ref repo) = self.repo {
            if block {
                let _ = repo.add_blacklist_rule("explicit", "true", Some("Block explicit lyrics"));
            }
        }
    }

    pub fn is_explicit_blocked(&self) -> bool {
        self.block_explicit
    }

    /// Check if a track violates any blacklist rule and return the rejection reason
    pub fn is_track_blacklisted(&self, track: &Track) -> Option<String> {
        // 1. Check exact track ID
        if self.blocked_track_ids.contains(&track.id.to_lowercase()) {
            return Some(format!("Track ID '{}' is blacklisted", track.id));
        }

        // 2. Check explicit flag
        if self.block_explicit && track.explicit {
            return Some("Track contains explicit content".to_string());
        }

        // 3. Check artists (including featured artists in collaborations)
        for artist in &track.artists {
            let lower = artist.to_lowercase();
            if self.blocked_artists.contains(&lower) {
                return Some(format!("Artist '{}' is blacklisted", artist));
            }
        }

        // 4. Check stop words in track title (e.g. remix, sped up, karaoke)
        let title_lower = track.title.to_lowercase();
        for sw in &self.stopwords {
            if title_lower.contains(sw) {
                return Some(format!("Title contains blocked stopword '{}'", sw));
            }
        }

        None
    }

    /// Instant in-place filtering of queue: removes all blacklisted tracks before playback
    pub fn filter_queue(&self, queue: &mut Vec<Track>) {
        queue.retain(|track| self.is_track_blacklisted(track).is_none());
    }

    /// Search for the next playable track in queue/wave starting from current_idx, skipping blacklisted items
    pub fn find_next_allowed_track<'a>(
        &self,
        queue: &'a [Track],
        start_index: usize,
    ) -> Option<(usize, &'a Track)> {
        queue
            .iter()
            .enumerate()
            .skip(start_index)
            .find(|(_, track)| self.is_track_blacklisted(track).is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::QualityBadge;

    fn create_dummy_track(id: &str, title: &str, artists: Vec<&str>, explicit: bool) -> Track {
        Track {
            id: id.to_string(),
            title: title.to_string(),
            artists: artists.into_iter().map(|s| s.to_string()).collect(),
            album: Some("Test Album".to_string()),
            duration_ms: 180_000,
            cover_uri: None,
            explicit,
            is_liked: false,
            quality: QualityBadge::HQ,
        }
    }

    #[test]
    fn test_blacklist_artist_and_collab_filtering() {
        let mut engine = BlacklistEngine::new(None);
        engine.block_artist("Morgenshtern", None);

        let clean_track = create_dummy_track("1", "Nice Song", vec!["Good Artist"], false);
        assert!(engine.is_track_blacklisted(&clean_track).is_none());

        // Solo track
        let banned_solo = create_dummy_track("2", "Loud Song", vec!["morgenshtern"], false);
        assert!(engine.is_track_blacklisted(&banned_solo).is_some());

        // Collab / feat
        let banned_feat = create_dummy_track("3", "Hit", vec!["Eminem", "Morgenshtern"], false);
        assert!(engine.is_track_blacklisted(&banned_feat).is_some());
    }

    #[test]
    fn test_blacklist_stopwords() {
        let engine = BlacklistEngine::new(None);
        let remix = create_dummy_track("1", "Cool Track (Radio Remix)", vec!["DJ"], false);
        let sped_up = create_dummy_track("2", "Night Song - Sped Up", vec!["Artist"], false);
        let normal = create_dummy_track("3", "Original Track", vec!["Artist"], false);

        assert!(engine.is_track_blacklisted(&remix).is_some());
        assert!(engine.is_track_blacklisted(&sped_up).is_some());
        assert!(engine.is_track_blacklisted(&normal).is_none());
    }

    #[test]
    fn test_blacklist_explicit_toggle() {
        let mut engine = BlacklistEngine::new(None);
        let explicit_track = create_dummy_track("1", "Explicit Track", vec!["Rapper"], true);

        // Allowed by default
        assert!(engine.is_track_blacklisted(&explicit_track).is_none());

        // Blocked when enabled
        engine.set_block_explicit(true);
        assert!(engine.is_track_blacklisted(&explicit_track).is_some());
    }

    #[test]
    fn test_filter_queue_in_place() {
        let mut engine = BlacklistEngine::new(None);
        engine.block_artist("BadArtist", None);

        let mut queue = vec![
            create_dummy_track("1", "Track 1", vec!["GoodArtist"], false),
            create_dummy_track("2", "Track 2", vec!["BadArtist"], false),
            create_dummy_track("3", "Track 3 (Remix)", vec!["GoodArtist"], false),
            create_dummy_track("4", "Track 4", vec!["GoodArtist"], false),
        ];

        engine.filter_queue(&mut queue);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].id, "1");
        assert_eq!(queue[1].id, "4");
    }

    #[test]
    fn test_find_next_allowed_track() {
        let mut engine = BlacklistEngine::new(None);
        engine.block_artist("BadArtist", None);

        let queue = vec![
            create_dummy_track("1", "Track 1", vec!["BadArtist"], false),
            create_dummy_track("2", "Track 2 (Karaoke)", vec!["GoodArtist"], false),
            create_dummy_track("3", "Track 3", vec!["GoodArtist"], false),
        ];

        let next = engine.find_next_allowed_track(&queue, 0);
        assert!(next.is_some());
        let (idx, track) = next.unwrap();
        assert_eq!(idx, 2);
        assert_eq!(track.id, "3");
    }

    #[test]
    fn test_blacklist_sqlite_sync() {
        let repo = Arc::new(DatabaseRepository::in_memory().unwrap());
        repo.add_blacklist_rule("artist", "BadBand", Some("Annoying")).unwrap();

        let mut engine = BlacklistEngine::new(Some(repo.clone()));
        let track = create_dummy_track("1", "Song", vec!["BadBand"], false);
        assert!(engine.is_track_blacklisted(&track).is_some());

        // Block another artist via engine and verify DB contains it
        engine.block_artist("AnotherBadArtist", None);
        let rules = repo.list_blacklist_rules().unwrap();
        assert!(rules.iter().any(|r| r.pattern == "anotherbadartist"));
    }
}
