use rusqlite::{Connection, Result};

pub const SCHEMA_MIGRATIONS: &[&str] = &[
    // Migration 1: Configuration table
    r#"
    CREATE TABLE IF NOT EXISTS config (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );
    "#,

    // Migration 2: Blacklist rules table
    r#"
    CREATE TABLE IF NOT EXISTS blacklist (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        rule_type TEXT NOT NULL,
        pattern TEXT NOT NULL,
        reason TEXT,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_blacklist_type_pattern ON blacklist(rule_type, pattern);
    "#,

    // Migration 3: Cache entries for predictive LRU cache
    r#"
    CREATE TABLE IF NOT EXISTS cache_entries (
        track_id TEXT PRIMARY KEY NOT NULL,
        file_path TEXT NOT NULL,
        file_size INTEGER NOT NULL,
        format TEXT NOT NULL,
        bitrate INTEGER,
        last_accessed_at INTEGER NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_cache_last_accessed ON cache_entries(last_accessed_at ASC);
    "#,

    // Migration 4: Equalizer presets
    r#"
    CREATE TABLE IF NOT EXISTS eq_presets (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT UNIQUE NOT NULL,
        is_builtin INTEGER NOT NULL DEFAULT 0,
        gains_json TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    "#,
];

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at INTEGER NOT NULL
        );",
        [],
    )?;

    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version;",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let now = chrono::Utc::now().timestamp();

    for (idx, migration_sql) in SCHEMA_MIGRATIONS.iter().enumerate() {
        let version = (idx + 1) as i64;
        if version > current_version {
            conn.execute_batch(migration_sql)?;
            conn.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2);",
                rusqlite::params![version, now],
            )?;
        }
    }

    seed_builtin_presets(conn, now)?;

    Ok(())
}

fn seed_builtin_presets(conn: &Connection, now: i64) -> Result<()> {
    let presets: &[(&str, &[f32; 10])] = &[
        ("Flat", &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        ("Bass Boost", &[5.0, 4.0, 3.0, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        ("Vocal", &[-1.0, -1.0, 0.0, 2.0, 3.5, 3.0, 2.0, 0.5, 0.0, -1.0]),
        ("Rock", &[4.5, 3.0, 1.5, 0.0, -1.0, 1.0, 2.5, 3.5, 4.0, 4.0]),
        ("Electronic", &[4.0, 3.5, 1.0, 0.0, -1.0, 1.5, 0.0, 2.0, 3.5, 4.0]),
        ("Classical", &[3.0, 2.5, 2.0, 1.5, -0.5, -0.5, 0.0, 1.5, 2.5, 3.0]),
    ];

    for (name, gains) in presets {
        let gains_json = serde_json::to_string(gains).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT OR IGNORE INTO eq_presets (name, is_builtin, gains_json, created_at)
             VALUES (?1, 1, ?2, ?3);",
            rusqlite::params![name, gains_json, now],
        )?;
    }

    Ok(())
}
