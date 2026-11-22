use rusqlite::{Connection, OptionalExtension, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlacklistRule {
    pub id: i64,
    pub rule_type: String, // "artist", "track", "stopword", "genre", "explicit"
    pub pattern: String,
    pub reason: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub track_id: String,
    pub file_path: String,
    pub file_size: u64,
    pub format: String,
    pub bitrate: Option<u32>,
    pub last_accessed_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqPreset {
    pub id: i64,
    pub name: String,
    pub is_builtin: bool,
    pub gains: [f32; 10],
    pub created_at: i64,
}

#[derive(Clone)]
pub struct DatabaseRepository {
    conn: Arc<Mutex<Connection>>,
}

impl DatabaseRepository {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        super::schema::run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        super::schema::run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // Config operations
    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        conn.query_row(
            "SELECT value FROM config WHERE key = ?1;",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .optional()
    }

    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO config (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at;",
            rusqlite::params![key, value, now],
        )?;
        Ok(())
    }

    pub fn remove_config(&self, key: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let rows = conn.execute("DELETE FROM config WHERE key = ?1;", rusqlite::params![key])?;
        Ok(rows > 0)
    }

    // Blacklist operations
    pub fn add_blacklist_rule(
        &self,
        rule_type: &str,
        pattern: &str,
        reason: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO blacklist (rule_type, pattern, reason, created_at)
             VALUES (?1, ?2, ?3, ?4);",
            rusqlite::params![rule_type, pattern.to_lowercase(), reason, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn remove_blacklist_rule(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let rows = conn.execute("DELETE FROM blacklist WHERE id = ?1;", rusqlite::params![id])?;
        Ok(rows > 0)
    }

    pub fn list_blacklist_rules(&self) -> Result<Vec<BlacklistRule>> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let mut stmt = conn.prepare(
            "SELECT id, rule_type, pattern, reason, created_at FROM blacklist ORDER BY id ASC;",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(BlacklistRule {
                id: row.get(0)?,
                rule_type: row.get(1)?,
                pattern: row.get(2)?,
                reason: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn is_track_blacklisted(
        &self,
        track_id: &str,
        title: &str,
        artist_ids: &[String],
        is_explicit: bool,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");

        // 1. Check track ID
        let track_blocked: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM blacklist WHERE rule_type = 'track' AND pattern = ?1);",
                rusqlite::params![track_id.to_lowercase()],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if track_blocked {
            return Ok(true);
        }

        // 2. Check explicit
        if is_explicit {
            let explicit_blocked: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM blacklist WHERE rule_type = 'explicit');",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if explicit_blocked {
                return Ok(true);
            }
        }

        // 3. Check artist IDs
        for artist_id in artist_ids {
            let artist_blocked: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM blacklist WHERE rule_type = 'artist' AND pattern = ?1);",
                    rusqlite::params![artist_id.to_lowercase()],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if artist_blocked {
                return Ok(true);
            }
        }

        // 4. Check stopwords in title
        let mut stmt = conn.prepare("SELECT pattern FROM blacklist WHERE rule_type = 'stopword';")?;
        let title_lower = title.to_lowercase();
        let stopwords = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for stopword in stopwords {
            let sw = stopword?;
            if title_lower.contains(&sw) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    // Cache operations (LRU)
    pub fn insert_or_update_cache(&self, entry: &CacheEntry) -> Result<()> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        conn.execute(
            "INSERT INTO cache_entries (track_id, file_path, file_size, format, bitrate, last_accessed_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(track_id) DO UPDATE SET
                file_path = excluded.file_path,
                file_size = excluded.file_size,
                format = excluded.format,
                bitrate = excluded.bitrate,
                last_accessed_at = excluded.last_accessed_at;",
            rusqlite::params![
                entry.track_id,
                entry.file_path,
                entry.file_size as i64,
                entry.format,
                entry.bitrate,
                entry.last_accessed_at,
                entry.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_cache_entry(&self, track_id: &str) -> Result<Option<CacheEntry>> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        conn.query_row(
            "SELECT track_id, file_path, file_size, format, bitrate, last_accessed_at, created_at
             FROM cache_entries WHERE track_id = ?1;",
            rusqlite::params![track_id],
            |row| {
                let size_i64: i64 = row.get(2)?;
                Ok(CacheEntry {
                    track_id: row.get(0)?,
                    file_path: row.get(1)?,
                    file_size: size_i64 as u64,
                    format: row.get(3)?,
                    bitrate: row.get(4)?,
                    last_accessed_at: row.get(5)?,
                    created_at: row.get(6)?,
                })
            },
        )
        .optional()
    }

    pub fn touch_cache_entry(&self, track_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let now = chrono::Utc::now().timestamp();
        let rows = conn.execute(
            "UPDATE cache_entries SET last_accessed_at = ?1 WHERE track_id = ?2;",
            rusqlite::params![now, track_id],
        )?;
        Ok(rows > 0)
    }

    pub fn get_total_cache_size(&self) -> Result<u64> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(file_size), 0) FROM cache_entries;",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(total as u64)
    }

    pub fn list_lru_cache_entries(&self) -> Result<Vec<CacheEntry>> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let mut stmt = conn.prepare(
            "SELECT track_id, file_path, file_size, format, bitrate, last_accessed_at, created_at
             FROM cache_entries ORDER BY last_accessed_at ASC;",
        )?;
        let rows = stmt.query_map([], |row| {
            let size_i64: i64 = row.get(2)?;
            Ok(CacheEntry {
                track_id: row.get(0)?,
                file_path: row.get(1)?,
                file_size: size_i64 as u64,
                format: row.get(3)?,
                bitrate: row.get(4)?,
                last_accessed_at: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn delete_cache_entry(&self, track_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let rows = conn.execute(
            "DELETE FROM cache_entries WHERE track_id = ?1;",
            rusqlite::params![track_id],
        )?;
        Ok(rows > 0)
    }

    // EQ Presets
    pub fn list_eq_presets(&self) -> Result<Vec<EqPreset>> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let mut stmt = conn.prepare(
            "SELECT id, name, is_builtin, gains_json, created_at FROM eq_presets ORDER BY is_builtin DESC, name ASC;",
        )?;
        let rows = stmt.query_map([], |row| {
            let gains_json: String = row.get(3)?;
            let gains: [f32; 10] = serde_json::from_str(&gains_json).unwrap_or([0.0; 10]);
            let is_builtin_int: i32 = row.get(2)?;
            Ok(EqPreset {
                id: row.get(0)?,
                name: row.get(1)?,
                is_builtin: is_builtin_int != 0,
                gains,
                created_at: row.get(4)?,
            })
        })?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn get_eq_preset(&self, name: &str) -> Result<Option<EqPreset>> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        conn.query_row(
            "SELECT id, name, is_builtin, gains_json, created_at FROM eq_presets WHERE name = ?1;",
            rusqlite::params![name],
            |row| {
                let gains_json: String = row.get(3)?;
                let gains: [f32; 10] = serde_json::from_str(&gains_json).unwrap_or([0.0; 10]);
                let is_builtin_int: i32 = row.get(2)?;
                Ok(EqPreset {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    is_builtin: is_builtin_int != 0,
                    gains,
                    created_at: row.get(4)?,
                })
            },
        )
        .optional()
    }

    pub fn save_eq_preset(&self, name: &str, gains: &[f32; 10]) -> Result<()> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let gains_json = serde_json::to_string(gains).unwrap_or_else(|_| "[]".to_string());
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO eq_presets (name, is_builtin, gains_json, created_at)
             VALUES (?1, 0, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET gains_json = excluded.gains_json WHERE is_builtin = 0;",
            rusqlite::params![name, gains_json, now],
        )?;
        Ok(())
    }

    pub fn delete_eq_preset(&self, name: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("Failed to lock DB connection");
        let rows = conn.execute(
            "DELETE FROM eq_presets WHERE name = ?1 AND is_builtin = 0;",
            rusqlite::params![name],
        )?;
        Ok(rows > 0)
    }
}
