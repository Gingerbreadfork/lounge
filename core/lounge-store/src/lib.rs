//! Durable state: one SQLite database owned by the core.

use lounge_graph::{Item, Layout};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("serialisation: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

impl StoreError {
    /// SQLite cannot read the file as a database at all, so only starting a new one helps.
    pub fn is_corrupt(&self) -> bool {
        matches!(
            self,
            StoreError::Db(rusqlite::Error::SqliteFailure(e, _))
                if matches!(e.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
        )
    }
}

/// An item already serialised for the `items` table.
pub struct EncodedItem {
    pub id: String,
    pub kind: String,
    pub json: String,
}

pub fn encode_items(items: &[Item]) -> Result<Vec<EncodedItem>> {
    items
        .iter()
        .map(|it| Ok(EncodedItem { id: it.id.clone(), kind: format!("{:?}", it.kind), json: serde_json::to_string(it)? }))
        .collect()
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS items (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    kind TEXT NOT NULL,
    json TEXT NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS items_source ON items(source);
CREATE TABLE IF NOT EXISTS layout (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS app_usage (
    package TEXT PRIMARY KEY,
    last_used_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS artwork (
    src_path TEXT PRIMARY KEY,
    src_len INTEGER NOT NULL,
    src_mtime_ms INTEGER NOT NULL,
    out_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS snapshot (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL,
    json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS plugins (
    id TEXT PRIMARY KEY,
    manifest TEXT NOT NULL,
    publisher TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0,
    built_in INTEGER NOT NULL DEFAULT 0,
    installed_ms INTEGER NOT NULL,
    last_refresh_ms INTEGER,
    last_error TEXT
);
CREATE TABLE IF NOT EXISTS plugin_kv (
    plugin TEXT NOT NULL,
    key TEXT NOT NULL,
    value BLOB NOT NULL,
    PRIMARY KEY (plugin, key)
);
CREATE TABLE IF NOT EXISTS plugin_settings (
    plugin TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (plugin, key)
);
CREATE TABLE IF NOT EXISTS usage_hours (
    package TEXT NOT NULL,
    hour INTEGER NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (package, hour)
);
"#;

const SCHEMA_VERSION: i64 = 4;

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

/// One installed plugin as the store remembers it.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginRow {
    pub id: String,
    pub manifest_json: String,
    pub publisher: String,
    pub enabled: bool,
    pub built_in: bool,
    pub installed_ms: i64,
    pub last_refresh_ms: Option<i64>,
    pub last_error: Option<String>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        let store = Store {
            conn,
            path: path.to_path_buf(),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Moves the database at `path` and its journal files to `<path>.broken`, replacing an earlier
    /// copy, and returns where it went. The next `open` starts an empty database.
    pub fn set_aside(path: &Path) -> Result<PathBuf> {
        let with = |p: &Path, suffix: &str| {
            let mut s = p.as_os_str().to_owned();
            s.push(suffix);
            PathBuf::from(s)
        };
        let aside = with(path, ".broken");
        // An earlier copy goes first, whatever it is: a rename cannot replace a file with a directory.
        let _ = std::fs::remove_file(&aside);
        let _ = std::fs::remove_dir_all(&aside);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(with(&aside, suffix));
        }
        std::fs::rename(path, &aside)?;
        for suffix in ["-wal", "-shm"] {
            let journal = with(path, suffix);
            if journal.exists() {
                std::fs::rename(&journal, with(&aside, suffix))?;
            }
        }
        Ok(aside)
    }

    pub fn open_in_memory() -> Result<Store> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        let store = Store {
            conn,
            path: PathBuf::from(":memory:"),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let current: i64 = self
            .meta("schema_version")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if current < 4 {
            // Prepared artwork changed (JPEGs at 2, the baked Stage vignette at 3, the lowered
            // Stage's light at 4); let it re-prepare.
            self.conn.execute("DELETE FROM artwork", [])?;
        }
        if current < SCHEMA_VERSION {
            self.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;
        }
        Ok(())
    }

    pub fn db_bytes(&self) -> u64 {
        if self.path.as_os_str() == ":memory:" {
            return 0;
        }
        let mut total = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        for suffix in ["-wal", "-shm"] {
            let mut p = self.path.as_os_str().to_owned();
            p.push(suffix);
            total += std::fs::metadata(PathBuf::from(p)).map(|m| m.len()).unwrap_or(0);
        }
        total
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// The saved layout, or the default one when none is saved or it cannot be read.
    pub fn load_layout(&self) -> Result<Layout> {
        let json: Option<String> = self
            .conn
            .query_row("SELECT json FROM layout WHERE id = 1", [], |r| r.get(0))
            .optional()?;
        let Some(json) = json else { return Ok(Layout::default()) };
        match serde_json::from_str(&json) {
            Ok(layout) => Ok(layout),
            Err(e) => {
                // Kept in meta so the next saved change does not lose it for good.
                tracing::warn!("stored layout is unreadable, using the default: {e}");
                let _ = self.set_meta("unreadable_layout", &json);
                Ok(Layout::default())
            }
        }
    }

    pub fn save_layout(&self, layout: &Layout) -> Result<()> {
        let json = serde_json::to_string(layout)?;
        self.conn.execute(
            "INSERT INTO layout(id, json) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![json],
        )?;
        Ok(())
    }

    pub fn replace_items(&mut self, source: &str, items: &[Item], now_ms: i64) -> Result<()> {
        let rows = encode_items(items)?;
        self.replace_items_encoded(source, &rows, now_ms)
    }

    /// As `replace_items`, with the JSON already encoded so the encoding happens before the store's
    /// lock is taken.
    pub fn replace_items_encoded(&mut self, source: &str, rows: &[EncodedItem], now_ms: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM items WHERE source = ?1", params![source])?;
        {
            // A repeated id keeps its first row rather than failing the whole write.
            let mut stmt = tx.prepare_cached(
                "INSERT INTO items(id, source, kind, json, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id) DO NOTHING",
            )?;
            for r in rows {
                stmt.execute(params![r.id, source, r.kind, r.json, now_ms])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert or update items without touching the rest of their source.
    pub fn upsert_items(&mut self, items: &[Item], now_ms: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO items(id, source, kind, json, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET source = excluded.source, kind = excluded.kind, json = excluded.json, updated_ms = excluded.updated_ms",
            )?;
            for it in items {
                stmt.execute(params![
                    it.id,
                    it.source,
                    format!("{:?}", it.kind),
                    serde_json::to_string(it)?,
                    now_ms
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Keep only the `keep` most recently updated items of a source; returns the ids removed.
    pub fn prune_source(&mut self, source: &str, keep: usize) -> Result<Vec<String>> {
        let mut removed = Vec::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM items WHERE source = ?1 ORDER BY updated_ms DESC LIMIT -1 OFFSET ?2",
            )?;
            let rows = stmt.query_map(params![source, keep as i64], |r| r.get::<_, String>(0))?;
            for r in rows {
                removed.push(r?);
            }
        }
        for id in &removed {
            self.conn.execute("DELETE FROM items WHERE id = ?1", params![id])?;
        }
        Ok(removed)
    }

    /// Every stored item this build can read. A row it cannot read, such as one a newer version
    /// wrote, is skipped rather than failing the load.
    pub fn load_items(&self) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare("SELECT id, json FROM items")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        let mut unreadable = 0;
        for row in rows {
            let (id, json) = row?;
            match serde_json::from_str(&json) {
                Ok(item) => out.push(item),
                Err(e) => {
                    unreadable += 1;
                    if unreadable == 1 {
                        tracing::warn!("stored item {id} is unreadable: {e}");
                    }
                }
            }
        }
        if unreadable > 1 {
            tracing::warn!("{unreadable} stored items were unreadable and skipped");
        }
        Ok(out)
    }

    /// Record when apps were last used. A use newer than the one stored also counts towards the
    /// app's habit at `local_hour(ms)`; each package and hour counted is returned.
    pub fn set_usage(&mut self, usage: &[(String, i64)], local_hour: impl Fn(i64) -> u32) -> Result<Vec<(String, u32)>> {
        let tx = self.conn.transaction()?;
        let mut counted = Vec::new();
        {
            let mut prev = tx.prepare("SELECT last_used_ms FROM app_usage WHERE package = ?1")?;
            let mut stmt = tx.prepare(
                "INSERT INTO app_usage(package, last_used_ms) VALUES (?1, ?2)
                 ON CONFLICT(package) DO UPDATE SET last_used_ms = MAX(last_used_ms, excluded.last_used_ms)",
            )?;
            let mut habit = tx.prepare(
                "INSERT INTO usage_hours(package, hour, count) VALUES (?1, ?2, 1)
                 ON CONFLICT(package, hour) DO UPDATE SET count = count + 1",
            )?;
            for (pkg, ms) in usage {
                let before: Option<i64> = prev.query_row(params![pkg], |r| r.get(0)).optional()?;
                // Uses less than ten minutes apart are one sitting, not two.
                if before.is_none_or(|b| *ms > b + 600_000) {
                    let hour = local_hour(*ms) % 24;
                    habit.execute(params![pkg, hour])?;
                    counted.push((pkg.clone(), hour));
                }
                stmt.execute(params![pkg, ms])?;
            }
        }
        tx.commit()?;
        Ok(counted)
    }

    /// How many times each app was opened in each local hour.
    pub fn load_habits(&self) -> Result<std::collections::HashMap<String, [u32; 24]>> {
        let mut stmt = self.conn.prepare("SELECT package, hour, count FROM usage_hours")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        let mut out: std::collections::HashMap<String, [u32; 24]> = std::collections::HashMap::new();
        for r in rows {
            let (pkg, hour, count) = r?;
            if (0..24).contains(&hour) {
                out.entry(pkg).or_insert([0; 24])[hour as usize] = count.max(0) as u32;
            }
        }
        Ok(out)
    }

    pub fn load_usage(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare("SELECT package, last_used_ms FROM app_usage")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// The remembered result for a source as stored, unparsed: a prepared picture's JSON, or
    /// `null` for a source that could not be prepared.
    pub fn artwork_cached_json(&self, src_path: &str, src_len: u64, src_mtime_ms: i64) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare_cached("SELECT src_len, src_mtime_ms, out_json FROM artwork WHERE src_path = ?1")?;
        let row: Option<(i64, i64, String)> = stmt
            .query_row(params![src_path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        Ok(row.filter(|(len, mtime, _)| *len == src_len as i64 && *mtime == src_mtime_ms).map(|(_, _, json)| json))
    }

    /// Every remembered artwork row, as its source key and JSON.
    pub fn artwork_rows(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT src_path, out_json FROM artwork")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn artwork_forget(&mut self, keys: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached("DELETE FROM artwork WHERE src_path = ?1")?;
            for k in keys {
                stmt.execute(params![k])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Remember several prepared pictures in one transaction: (source path key, length, mtime, JSON).
    pub fn artwork_put_many(&self, rows: &[(String, u64, i64, String)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO artwork(src_path, src_len, src_mtime_ms, out_json) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(src_path) DO UPDATE SET src_len = excluded.src_len, src_mtime_ms = excluded.src_mtime_ms, out_json = excluded.out_json",
            )?;
            for (path, len, mtime, json) in rows {
                stmt.execute(params![path, *len as i64, mtime, json])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn plugin_upsert(&self, row: &PluginRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO plugins(id, manifest, publisher, enabled, built_in, installed_ms, last_refresh_ms, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET manifest = excluded.manifest, publisher = excluded.publisher, built_in = excluded.built_in,
             last_refresh_ms = NULL, last_error = NULL",
            params![row.id, row.manifest_json, row.publisher, row.enabled as i64, row.built_in as i64, row.installed_ms, row.last_refresh_ms, row.last_error],
        )?;
        Ok(())
    }

    pub fn plugins(&self) -> Result<Vec<PluginRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, manifest, publisher, enabled, built_in, installed_ms, last_refresh_ms, last_error FROM plugins ORDER BY installed_ms",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PluginRow {
                id: r.get(0)?,
                manifest_json: r.get(1)?,
                publisher: r.get(2)?,
                enabled: r.get::<_, i64>(3)? != 0,
                built_in: r.get::<_, i64>(4)? != 0,
                installed_ms: r.get(5)?,
                last_refresh_ms: r.get(6)?,
                last_error: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn plugin_remove(&mut self, id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM plugins WHERE id = ?1", params![id])?;
        tx.execute("DELETE FROM plugin_kv WHERE plugin = ?1", params![id])?;
        tx.execute("DELETE FROM plugin_settings WHERE plugin = ?1", params![id])?;
        tx.execute("DELETE FROM items WHERE source = ?1", params![format!("plugin:{id}")])?;
        tx.commit()?;
        Ok(())
    }

    pub fn plugin_set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.conn.execute("UPDATE plugins SET enabled = ?2 WHERE id = ?1", params![id, enabled as i64])?;
        Ok(())
    }

    pub fn plugin_set_refresh(&self, id: &str, at_ms: i64, error: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE plugins SET last_refresh_ms = ?2, last_error = ?3 WHERE id = ?1", params![id, at_ms, error])?;
        Ok(())
    }

    pub fn plugin_kv_get(&self, plugin: &str, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM plugin_kv WHERE plugin = ?1 AND key = ?2", params![plugin, key], |r| r.get(0))
            .optional()?)
    }

    pub fn plugin_kv_set(&self, plugin: &str, key: &str, value: &[u8], max_keys: usize) -> Result<()> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM plugin_kv WHERE plugin = ?1", params![plugin], |r| r.get(0))?;
        let exists: bool = self
            .conn
            .query_row("SELECT 1 FROM plugin_kv WHERE plugin = ?1 AND key = ?2", params![plugin, key], |_| Ok(()))
            .optional()?
            .is_some();
        if !exists && count as usize >= max_keys {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO plugin_kv(plugin, key, value) VALUES (?1, ?2, ?3) ON CONFLICT(plugin, key) DO UPDATE SET value = excluded.value",
            params![plugin, key, value],
        )?;
        Ok(())
    }

    pub fn plugin_kv_delete(&self, plugin: &str, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM plugin_kv WHERE plugin = ?1 AND key = ?2", params![plugin, key])?;
        Ok(())
    }

    pub fn plugin_setting(&self, plugin: &str, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM plugin_settings WHERE plugin = ?1 AND key = ?2", params![plugin, key], |r| r.get(0))
            .optional()?)
    }

    pub fn plugin_settings(&self, plugin: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM plugin_settings WHERE plugin = ?1")?;
        let rows = stmt.query_map(params![plugin], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn plugin_setting_set(&self, plugin: &str, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(v) => {
                self.conn.execute(
                    "INSERT INTO plugin_settings(plugin, key, value) VALUES (?1, ?2, ?3) ON CONFLICT(plugin, key) DO UPDATE SET value = excluded.value",
                    params![plugin, key, v],
                )?;
            }
            None => {
                self.conn.execute("DELETE FROM plugin_settings WHERE plugin = ?1 AND key = ?2", params![plugin, key])?;
            }
        }
        Ok(())
    }

    pub fn save_snapshot(&self, revision: u64, json: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO snapshot(id, revision, json) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET revision = excluded.revision, json = excluded.json",
            params![revision as i64, json],
        )?;
        Ok(())
    }

    pub fn load_snapshot(&self) -> Result<Option<(u64, String)>> {
        Ok(self
            .conn
            .query_row("SELECT revision, json FROM snapshot WHERE id = 1", [], |r| {
                Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
            })
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lounge_graph::Item;

    #[test]
    fn layout_roundtrip() {
        let s = Store::open_in_memory().unwrap();
        let mut l = Layout::default();
        l.pinned_apps.push("com.netflix.ninja".into());
        l.hidden.insert("app:x".into());
        s.save_layout(&l).unwrap();
        assert_eq!(s.load_layout().unwrap(), l);
    }

    #[test]
    fn an_unreadable_layout_falls_back_and_is_kept() {
        let s = Store::open_in_memory().unwrap();
        s.conn.execute("INSERT INTO layout(id, json) VALUES (1, '{\"hidden\": 3}')", []).unwrap();
        assert_eq!(s.load_layout().unwrap(), Layout::default());
        assert_eq!(s.meta("unreadable_layout").unwrap().as_deref(), Some("{\"hidden\": 3}"));
    }

    #[test]
    fn a_file_that_is_not_a_database_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lounge.db");
        std::fs::write(&path, vec![0x5a; 8192]).unwrap();
        let err = Store::open(&path).err().expect("a file of noise must not open");
        assert!(err.is_corrupt(), "{err}");
        assert_eq!(Store::set_aside(&path).unwrap(), dir.path().join("lounge.db.broken"));
        assert!(!path.exists());
        let s = Store::open(&path).unwrap();
        assert_eq!(s.load_layout().unwrap(), Layout::default());
    }

    #[test]
    fn setting_aside_takes_the_journal_and_replaces_an_earlier_copy() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| dir.path().join(name);
        for name in ["lounge.db", "lounge.db-wal", "lounge.db.broken", "lounge.db.broken-shm"] {
            std::fs::write(file(name), name).unwrap();
        }
        Store::set_aside(&file("lounge.db")).unwrap();
        assert_eq!(std::fs::read_to_string(file("lounge.db.broken")).unwrap(), "lounge.db");
        assert_eq!(std::fs::read_to_string(file("lounge.db.broken-wal")).unwrap(), "lounge.db-wal");
        assert!(!file("lounge.db.broken-shm").exists());
        assert!(!file("lounge.db").exists() && !file("lounge.db-wal").exists());
    }

    #[test]
    fn setting_aside_replaces_an_earlier_copy_of_another_kind() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("lounge.db")).unwrap();
        std::fs::write(dir.path().join("lounge.db.broken"), b"older").unwrap();
        Store::set_aside(&dir.path().join("lounge.db")).unwrap();
        assert!(dir.path().join("lounge.db.broken").is_dir());
        assert!(Store::open(&dir.path().join("lounge.db")).is_ok());
    }

    #[test]
    fn items_replace_by_source() {
        let mut s = Store::open_in_memory().unwrap();
        s.replace_items("apps", &[Item::app("a", "A", "A"), Item::app("b", "B", "B")], 1)
            .unwrap();
        let mut c = Item::app("c", "C", "C");
        c.source = "tif".into();
        s.replace_items("tif", &[c], 1).unwrap();
        assert_eq!(s.load_items().unwrap().len(), 3);
        s.replace_items("apps", &[], 2).unwrap();
        let left = s.load_items().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].source, "tif");
    }

    #[test]
    fn upsert_and_prune_by_recency() {
        let mut s = Store::open_in_memory().unwrap();
        let mut items = Vec::new();
        for i in 0..5 {
            let mut it = Item::app(&format!("p{i}"), "A", "A");
            it.source = "sessions".into();
            items.push(it);
        }
        for (i, it) in items.iter().enumerate() {
            s.upsert_items(std::slice::from_ref(it), i as i64).unwrap();
        }
        let removed = s.prune_source("sessions", 2).unwrap();
        assert_eq!(removed.len(), 3);
        let left: Vec<String> = s.load_items().unwrap().into_iter().map(|i| i.id).collect();
        assert!(left.contains(&"app:p4".to_string()) && left.contains(&"app:p3".to_string()));
    }

    #[test]
    fn usage_keeps_max() {
        let mut s = Store::open_in_memory().unwrap();
        s.set_usage(&[("p".into(), 10)], |_| 0).unwrap();
        s.set_usage(&[("p".into(), 5)], |_| 0).unwrap();
        assert_eq!(s.load_usage().unwrap(), vec![("p".to_string(), 10)]);
    }

    #[test]
    fn habits_count_each_sitting_once() {
        let mut s = Store::open_in_memory().unwrap();
        let hour = |ms: i64| ((ms / 3_600_000) % 24) as u32;
        let nine = 21 * 3_600_000;
        assert_eq!(s.set_usage(&[("p".into(), nine)], hour).unwrap(), vec![("p".to_string(), 21)]);
        // The same sitting reported again, and a few minutes later, counts once.
        assert!(s.set_usage(&[("p".into(), nine)], hour).unwrap().is_empty());
        assert!(s.set_usage(&[("p".into(), nine + 300_000)], hour).unwrap().is_empty());
        s.set_usage(&[("p".into(), nine + 86_400_000)], hour).unwrap();
        assert_eq!(s.load_habits().unwrap()["p"][21], 2);
    }

    #[test]
    fn a_repeated_id_does_not_fail_the_write() {
        let mut s = Store::open_in_memory().unwrap();
        let mut a = Item::app("a", "A", "First");
        a.source = "plugin:x".into();
        let mut b = a.clone();
        b.title = "Second".into();
        s.replace_items("plugin:x", &[a, b], 1).unwrap();
        let items = s.load_items().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "First");
    }

    #[test]
    fn unreadable_rows_are_skipped() {
        let mut s = Store::open_in_memory().unwrap();
        s.replace_items("apps", &[Item::app("a", "A", "A")], 1).unwrap();
        s.conn.execute("INSERT INTO items(id, source, kind, json, updated_ms) VALUES ('z', 'x', 'Future', '{\"kind\":\"Hologram\"}', 1)", []).unwrap();
        assert_eq!(s.load_items().unwrap().len(), 1);
    }

    #[test]
    fn artwork_cache_invalidates_on_change() {
        let s = Store::open_in_memory().unwrap();
        s.artwork_put_many(&[("x.png".to_string(), 100, 1, "[\"a\"]".to_string())]).unwrap();
        assert_eq!(s.artwork_cached_json("x.png", 100, 1).unwrap().as_deref(), Some("[\"a\"]"));
        assert!(s.artwork_cached_json("x.png", 101, 1).unwrap().is_none());
    }

    #[test]
    fn plugin_tables() {
        let mut s = Store::open_in_memory().unwrap();
        let row = PluginRow { id: "tv.lounge.iptv".into(), manifest_json: "{}".into(), publisher: "ab".into(), enabled: false, built_in: true, installed_ms: 1, last_refresh_ms: None, last_error: None };
        s.plugin_upsert(&row).unwrap();
        s.plugin_set_enabled("tv.lounge.iptv", true).unwrap();
        s.plugin_setting_set("tv.lounge.iptv", "playlist_url", Some("http://x")).unwrap();
        s.plugin_kv_set("tv.lounge.iptv", "token", b"t", 8).unwrap();
        assert!(s.plugins().unwrap()[0].enabled);
        // An update keeps the plugin's place among the others.
        s.plugin_upsert(&PluginRow { installed_ms: 99, ..row.clone() }).unwrap();
        assert_eq!(s.plugins().unwrap()[0].installed_ms, 1);
        assert_eq!(s.plugin_settings("tv.lounge.iptv").unwrap(), vec![("playlist_url".to_string(), "http://x".to_string())]);
        assert_eq!(s.plugin_kv_get("tv.lounge.iptv", "token").unwrap(), Some(b"t".to_vec()));
        s.plugin_remove("tv.lounge.iptv").unwrap();
        assert!(s.plugins().unwrap().is_empty());
        assert!(s.plugin_kv_get("tv.lounge.iptv", "token").unwrap().is_none());
    }

    #[test]
    fn snapshot_roundtrip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("lounge.db");
        let s = Store::open(&path).unwrap();
        s.save_snapshot(7, "{\"ok\":true}").unwrap();
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.load_snapshot().unwrap(), Some((7, "{\"ok\":true}".to_string())));
        assert!(s.db_bytes() > 0);
    }
}
