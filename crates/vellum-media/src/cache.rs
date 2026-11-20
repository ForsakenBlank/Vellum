//! the media cache: thumbnails, filmstrips, waveforms, frame indexes and probe results.
//!
//! an sqlite index (`index.sqlite`) maps a source file plus what was made from it to a file
//! under `files/`. a source is identified by its path, size and modified time, so editing a
//! file outside vellum makes its old previews stale and they are replaced on next use.
//!
//! the cache is disposable. if the index cannot be opened it is thrown away and started
//! again, and if even that fails an in memory index keeps the app working. nothing in here
//! ever stops vellum starting. the size limit is enforced by removing the least recently
//! used entries after every write. ids never repeat, so a regenerated preview always gets a
//! new file name and the webview cannot show a stale copy it cached under the old one.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use ts_rs::TS;

use crate::error::{MediaError, MediaResult};

const SCHEMA_VERSION: i64 = 1;
/// reading an entry only updates its last used time this often, to save writes
const TOUCH_INTERVAL_MS: i64 = 60_000;

/// identifies one version of a source file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceKey {
    pub path: String,
    pub size: u64,
    pub modified_ns: i64,
}

impl SourceKey {
    pub fn of(path: &Path) -> MediaResult<Self> {
        let meta = fs::metadata(path).map_err(|err| MediaError::io(path, err))?;
        let modified_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        // the same file reached through a different relative path or symlink shares entries
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        Ok(Self {
            path: path.display().to_string(),
            size: meta.len(),
            modified_ns,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    /// the cached file, for entries that have one
    pub file: Option<PathBuf>,
    /// small json alongside, eg filmstrip frame times or a probe result
    pub meta: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CacheStats {
    #[ts(type = "number")]
    pub bytes: u64,
    #[ts(type = "number")]
    pub entries: u64,
    #[ts(type = "number")]
    pub limit_bytes: u64,
    pub path: String,
}

pub struct MediaCache {
    dir: PathBuf,
    conn: Mutex<Connection>,
    limit: AtomicU64,
}

impl MediaCache {
    /// opens the cache in `dir`, starting over if the index is damaged
    pub fn open(dir: &Path, limit_bytes: u64) -> Self {
        let conn = match open_index(dir) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!(%err, ?dir, "the media cache was unreadable, starting a new one");
                let _ = fs::remove_dir_all(dir);
                open_index(dir).unwrap_or_else(|err| {
                    tracing::error!(%err, "the media cache cannot be stored on disk this session");
                    memory_index()
                })
            }
        };
        Self {
            dir: dir.to_path_buf(),
            conn: Mutex::new(conn),
            limit: AtomicU64::new(limit_bytes),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn set_limit(&self, bytes: u64) {
        let old = self.limit.swap(bytes, Ordering::Relaxed);
        if bytes < old
            && let Err(err) = self.evict()
        {
            tracing::warn!(%err, "could not shrink the media cache");
        }
    }

    /// a cached result, or none if there is none for this version of the source
    pub fn get(&self, source: &SourceKey, kind: &str, params: &str) -> Option<CacheEntry> {
        self.try_get(source, kind, params).unwrap_or_else(|err| {
            tracing::warn!(%err, "media cache lookup failed");
            None
        })
    }

    fn try_get(
        &self,
        source: &SourceKey,
        kind: &str,
        params: &str,
    ) -> MediaResult<Option<CacheEntry>> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT id, source_size, source_modified, file, meta, accessed_ms FROM entries
                 WHERE source = ?1 AND kind = ?2 AND params = ?3",
                params![source.path, kind, params],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, size, modified, file, meta, accessed)) = row else {
            return Ok(None);
        };
        let file = file.map(|f| self.dir.join(f));
        let stale = size != i64::try_from(source.size).unwrap_or(i64::MAX)
            || modified != source.modified_ns;
        let missing = file.as_ref().is_some_and(|f| !f.is_file());
        if stale || missing {
            delete_rows(&conn, &self.dir, &[id])?;
            return Ok(None);
        }
        let now = now_ms();
        if now - accessed > TOUCH_INTERVAL_MS {
            conn.execute(
                "UPDATE entries SET accessed_ms = ?1 WHERE id = ?2",
                params![now, id],
            )?;
        }
        Ok(Some(CacheEntry { file, meta }))
    }

    /// stores `bytes` as a file for this source and returns where it went
    pub fn put_file(
        &self,
        source: &SourceKey,
        kind: &str,
        params: &str,
        bytes: &[u8],
        extension: &str,
        meta: Option<&str>,
    ) -> MediaResult<PathBuf> {
        let path = {
            let conn = self.conn.lock();
            let id = upsert(&conn, &self.dir, source, kind, params, meta)?;
            let relative = format!("files/{:02x}/{id}.{extension}", id % 256);
            let path = self.dir.join(&relative);
            write_atomic(&path, bytes)?;
            conn.execute(
                "UPDATE entries SET file = ?1, bytes = ?2 WHERE id = ?3",
                params![relative, bytes.len() as i64, id],
            )?;
            path
        };
        self.evict()?;
        Ok(path)
    }

    /// stores only metadata, eg a probe result
    pub fn put_meta(
        &self,
        source: &SourceKey,
        kind: &str,
        params: &str,
        meta: &str,
    ) -> MediaResult<()> {
        let conn = self.conn.lock();
        let id = upsert(&conn, &self.dir, source, kind, params, Some(meta))?;
        conn.execute(
            "UPDATE entries SET bytes = ?1 WHERE id = ?2",
            params![meta.len() as i64, id],
        )?;
        Ok(())
    }

    pub fn stats(&self) -> CacheStats {
        let conn = self.conn.lock();
        let (bytes, entries) = conn
            .query_row(
                "SELECT COALESCE(SUM(bytes), 0), COUNT(*) FROM entries",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap_or((0, 0));
        CacheStats {
            bytes: u64::try_from(bytes).unwrap_or(0),
            entries: u64::try_from(entries).unwrap_or(0),
            limit_bytes: self.limit.load(Ordering::Relaxed),
            path: self.dir.display().to_string(),
        }
    }

    /// removes everything
    pub fn clean(&self) -> MediaResult<CacheStats> {
        {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM entries", [])?;
            let files = self.dir.join("files");
            if files.exists() {
                fs::remove_dir_all(&files).map_err(|err| MediaError::io(&files, err))?;
            }
        }
        Ok(self.stats())
    }

    /// removes least recently used entries until the cache fits its limit
    fn evict(&self) -> MediaResult<()> {
        let limit = i64::try_from(self.limit.load(Ordering::Relaxed)).unwrap_or(i64::MAX);
        let conn = self.conn.lock();
        loop {
            let total: i64 =
                conn.query_row("SELECT COALESCE(SUM(bytes), 0) FROM entries", [], |row| {
                    row.get(0)
                })?;
            if total <= limit {
                return Ok(());
            }
            let mut over = total - limit;
            let mut stmt =
                conn.prepare("SELECT id, bytes FROM entries ORDER BY accessed_ms, id LIMIT 64")?;
            let oldest: Vec<(i64, i64)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            if oldest.is_empty() {
                return Ok(());
            }
            let mut doomed = Vec::new();
            for (id, bytes) in oldest {
                doomed.push(id);
                over -= bytes;
                if over <= 0 {
                    break;
                }
            }
            delete_rows(&conn, &self.dir, &doomed)?;
        }
    }
}

fn open_index(dir: &Path) -> MediaResult<Connection> {
    fs::create_dir_all(dir).map_err(|err| MediaError::io(dir, err))?;
    let conn = Connection::open(dir.join("index.sqlite"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    // wal lets a second vellum window read while another writes
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(MediaError::Cache(format!(
            "made by a newer Vellum (schema {version})"
        )));
    }
    create_schema(&conn)?;
    // a quick check that the file really is a working database
    conn.query_row("SELECT COUNT(*) FROM entries", [], |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(conn)
}

fn memory_index() -> Connection {
    // an in memory database can only fail to open if sqlite itself is broken
    #[allow(clippy::expect_used)]
    let conn = Connection::open_in_memory().expect("sqlite can open an in memory database");
    if let Err(err) = create_schema(&conn) {
        tracing::error!(%err, "even the in memory media cache failed");
    }
    conn
}

fn create_schema(conn: &Connection) -> MediaResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            source_size INTEGER NOT NULL,
            source_modified INTEGER NOT NULL,
            kind TEXT NOT NULL,
            params TEXT NOT NULL,
            file TEXT,
            bytes INTEGER NOT NULL DEFAULT 0,
            meta TEXT,
            created_ms INTEGER NOT NULL,
            accessed_ms INTEGER NOT NULL,
            UNIQUE (source, kind, params)
        );
        CREATE INDEX IF NOT EXISTS entries_accessed ON entries (accessed_ms);",
    )?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// replaces any entry for the same source, kind and params, returning the new row id
fn upsert(
    conn: &Connection,
    dir: &Path,
    source: &SourceKey,
    kind: &str,
    params: &str,
    meta: Option<&str>,
) -> MediaResult<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM entries WHERE source = ?1 AND kind = ?2 AND params = ?3",
            params![source.path, kind, params],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        delete_rows(conn, dir, &[id])?;
    }
    let now = now_ms();
    conn.execute(
        "INSERT INTO entries (source, source_size, source_modified, kind, params, meta, created_ms, accessed_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            source.path,
            i64::try_from(source.size).unwrap_or(i64::MAX),
            source.modified_ns,
            kind,
            params,
            meta,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn delete_rows(conn: &Connection, dir: &Path, ids: &[i64]) -> MediaResult<()> {
    for id in ids {
        let file: Option<String> = conn
            .query_row("SELECT file FROM entries WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .flatten();
        if let Some(file) = file {
            // already gone is fine, the row is what matters
            let _ = fs::remove_file(dir.join(file));
        }
        conn.execute("DELETE FROM entries WHERE id = ?1", [id])?;
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> MediaResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| MediaError::io(parent, err))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|err| MediaError::io(&tmp, err))?;
    fs::rename(&tmp, path).map_err(|err| MediaError::io(path, err))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(dir: &Path, name: &str, contents: &[u8]) -> SourceKey {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        SourceKey::of(&path).unwrap()
    }

    #[test]
    fn files_and_meta_come_back_until_the_source_changes() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 1 << 20);
        let key = source(dir.path(), "a.png", b"one");
        assert!(cache.get(&key, "thumb", "384").is_none());

        let file = cache
            .put_file(
                &key,
                "thumb",
                "384",
                b"jpeg bytes",
                "jpg",
                Some("{\"w\":1}"),
            )
            .unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"jpeg bytes");
        let hit = cache.get(&key, "thumb", "384").unwrap();
        assert_eq!(hit.file.as_deref(), Some(file.as_path()));
        assert_eq!(hit.meta.as_deref(), Some("{\"w\":1}"));
        assert!(cache.get(&key, "thumb", "128").is_none());

        cache.put_meta(&key, "probe", "", "{}").unwrap();
        assert_eq!(cache.stats().entries, 2);

        // a different size means the user changed the file, so the old preview is stale
        let changed = source(dir.path(), "a.png", b"changed!");
        assert!(cache.get(&changed, "thumb", "384").is_none());
        assert!(!file.exists());
    }

    #[test]
    fn writing_the_same_entry_replaces_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 1 << 20);
        let key = source(dir.path(), "a.mp4", b"video");
        let first = cache
            .put_file(&key, "strip", "8", b"aaaa", "jpg", None)
            .unwrap();
        let second = cache
            .put_file(&key, "strip", "8", b"bb", "jpg", None)
            .unwrap();
        assert!(!first.exists());
        assert!(second.exists());
        assert_eq!(cache.stats().bytes, 2);
    }

    #[test]
    fn the_limit_removes_least_recently_used_first() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 25);
        let keys: Vec<_> = (0..3)
            .map(|i| source(dir.path(), &format!("{i}.png"), format!("{i}").as_bytes()))
            .collect();
        for key in &keys {
            cache
                .put_file(key, "thumb", "", &[0; 10], "jpg", None)
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // three files of 10 bytes do not fit in 25, so the oldest went
        assert!(cache.get(&keys[0], "thumb", "").is_none());
        assert!(cache.get(&keys[1], "thumb", "").is_some());
        assert!(cache.get(&keys[2], "thumb", "").is_some());
        assert!(cache.stats().bytes <= 25);

        cache.set_limit(10);
        assert_eq!(cache.stats().entries, 1);
    }

    #[test]
    fn clean_empties_everything() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 1 << 20);
        let key = source(dir.path(), "a.png", b"x");
        let file = cache
            .put_file(&key, "thumb", "", b"123", "jpg", None)
            .unwrap();
        let stats = cache.clean().unwrap();
        assert_eq!((stats.bytes, stats.entries), (0, 0));
        assert!(!file.exists());
        // and it still works afterwards
        cache
            .put_file(&key, "thumb", "", b"123", "jpg", None)
            .unwrap();
        assert!(cache.get(&key, "thumb", "").is_some());
    }

    #[test]
    fn a_damaged_index_is_replaced_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(
            cache_dir.join("index.sqlite"),
            b"this is not a database at all, not even close",
        )
        .unwrap();
        let cache = MediaCache::open(&cache_dir, 1 << 20);
        let key = source(dir.path(), "a.png", b"x");
        cache
            .put_file(&key, "thumb", "", b"1", "jpg", None)
            .unwrap();
        assert!(cache.get(&key, "thumb", "").is_some());
    }

    #[test]
    fn a_deleted_cache_file_counts_as_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 1 << 20);
        let key = source(dir.path(), "a.png", b"x");
        let file = cache
            .put_file(&key, "thumb", "", b"1", "jpg", None)
            .unwrap();
        fs::remove_file(file).unwrap();
        assert!(cache.get(&key, "thumb", "").is_none());
        assert_eq!(cache.stats().entries, 0);
    }
}
