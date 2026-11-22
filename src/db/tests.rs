#[cfg(test)]
mod tests {
    use crate::db::repository::{CacheEntry, DatabaseRepository};

    #[test]
    fn test_migrations_and_builtin_presets() {
        let repo = DatabaseRepository::in_memory().expect("Failed to create in-memory DB");
        let presets = repo.list_eq_presets().expect("Failed to list EQ presets");
        assert!(presets.len() >= 6);

        let flat = repo.get_eq_preset("Flat").expect("Failed to get Flat preset");
        assert!(flat.is_some());
        let flat = flat.unwrap();
        assert!(flat.is_builtin);
        assert_eq!(flat.gains, [0.0; 10]);
    }

    #[test]
    fn test_config_crud() {
        let repo = DatabaseRepository::in_memory().expect("Failed to create DB");

        assert_eq!(repo.get_config("token").unwrap(), None);

        repo.set_config("token", "secret123").unwrap();
        assert_eq!(
            repo.get_config("token").unwrap(),
            Some("secret123".to_string())
        );

        // Update existing key
        repo.set_config("token", "secret456").unwrap();
        assert_eq!(
            repo.get_config("token").unwrap(),
            Some("secret456".to_string())
        );

        assert!(repo.remove_config("token").unwrap());
        assert_eq!(repo.get_config("token").unwrap(), None);
    }

    #[test]
    fn test_blacklist_rules_and_matching() {
        let repo = DatabaseRepository::in_memory().expect("Failed to create DB");

        // Rule 1: Block artist by ID
        repo.add_blacklist_rule("artist", "1001", Some("Unwanted artist"))
            .unwrap();
        // Rule 2: Block stopword "remix"
        repo.add_blacklist_rule("stopword", "remix", Some("No remixes"))
            .unwrap();
        // Rule 3: Block explicit content
        repo.add_blacklist_rule("explicit", "1", Some("Clean mode"))
            .unwrap();

        // 1. Should be blocked due to artist 1001
        let blocked_artist = repo
            .is_track_blacklisted("t1", "Good Song", &["1001".to_string()], false)
            .unwrap();
        assert!(blocked_artist);

        // 2. Should be blocked due to stopword "remix" in title
        let blocked_remix = repo
            .is_track_blacklisted("t2", "Awesome Song (Club Remix)", &["2002".to_string()], false)
            .unwrap();
        assert!(blocked_remix);

        // 3. Should be blocked due to explicit
        let blocked_explicit = repo
            .is_track_blacklisted("t3", "Explicit Track", &["2002".to_string()], true)
            .unwrap();
        assert!(blocked_explicit);

        // 4. Clean track should pass
        let allowed = repo
            .is_track_blacklisted("t4", "Clean Original Track", &["2002".to_string()], false)
            .unwrap();
        assert!(!allowed);
    }

    #[test]
    fn test_cache_lru_operations() {
        let repo = DatabaseRepository::in_memory().expect("Failed to create DB");

        let entry1 = CacheEntry {
            track_id: "track_1".to_string(),
            file_path: "/cache/1.mp3".to_string(),
            file_size: 5_000_000,
            format: "mp3".to_string(),
            bitrate: Some(320),
            last_accessed_at: 100,
            created_at: 100,
        };

        let entry2 = CacheEntry {
            track_id: "track_2".to_string(),
            file_path: "/cache/2.flac".to_string(),
            file_size: 25_000_000,
            format: "flac".to_string(),
            bitrate: Some(1411),
            last_accessed_at: 200,
            created_at: 200,
        };

        repo.insert_or_update_cache(&entry1).unwrap();
        repo.insert_or_update_cache(&entry2).unwrap();

        assert_eq!(repo.get_total_cache_size().unwrap(), 30_000_000);

        let lru = repo.list_lru_cache_entries().unwrap();
        assert_eq!(lru.len(), 2);
        assert_eq!(lru[0].track_id, "track_1"); // accessed at 100 comes first

        // Touch track 1 to make it newer
        repo.touch_cache_entry("track_1").unwrap();
        let lru_after_touch = repo.list_lru_cache_entries().unwrap();
        assert_eq!(lru_after_touch[0].track_id, "track_2"); // track 2 is now the oldest

        repo.delete_cache_entry("track_2").unwrap();
        assert_eq!(repo.get_total_cache_size().unwrap(), 5_000_000);
    }

    #[test]
    fn test_custom_eq_preset() {
        let repo = DatabaseRepository::in_memory().expect("Failed to create DB");

        let gains = [1.0, 2.0, 3.0, 4.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0];
        repo.save_eq_preset("MyCustom", &gains).unwrap();

        let preset = repo.get_eq_preset("MyCustom").unwrap().unwrap();
        assert_eq!(preset.name, "MyCustom");
        assert!(!preset.is_builtin);
        assert_eq!(preset.gains, gains);

        // Builtin cannot be deleted
        assert!(!repo.delete_eq_preset("Flat").unwrap());

        // Custom can be deleted
        assert!(repo.delete_eq_preset("MyCustom").unwrap());
        assert_eq!(repo.get_eq_preset("MyCustom").unwrap(), None);
    }
}
