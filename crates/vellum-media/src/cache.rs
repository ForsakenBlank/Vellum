//! the media cache: thumbnails, filmstrips, waveforms, probe results and frame indexes,
//! kept on disk so a file shows up instantly the second time.
//!
//! an sqlite index (`index.sqlite`) maps a key to an optional blob file in `blobs/` plus
//! optional json. a key is the file's path, its size and modified time, what was made
//! (`kind`) and with which settings (`params`). editing a file changes its size or time,
//! so stale entries are never served; they are replaced when the file is processed again
//! or evicted as the least recently used once the cache is over its size limit.
//!
//! the cache is disposable. an index that cannot be opened is moved aside and a fresh one
//! started, so a broken cache never stops vellum from starting.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{MediaError, MediaResult};

const SCHEMA_VERSION: i64 = 1;
const INDEX_FILE: &str = "index.sqlite";
const BLOB_DIR: &str = "blobs";
/// last-used times are only written when older than this, so browsing a folder of
/// thumbnails does not turn every read into a write
const TOUCH_EVERY_MS: i64 = 60_000;

/// what identifies one cached thing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub path: String,
    pub size: u64,
    /// modified time in nanoseconds since the unix epoch
    pub modified_ns: i64,
    /// what was made, eg "thumb", "waveform", "probe"
    pub kind: String,
    /// the settings it was made with, eg "320"
    pub params: String,
}

impl CacheKey {
    /// a key for `path` as it is on disk now
    pub fn for_file(path: &Path, kind: &str, params: &str) -> MediaResult<Self> {
        let meta = fs::metadata(path).map_err(|err| MediaError::io(path, err))?;
        let modified_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
        // the same file reached two ways (relative, symlinked) shares one entry
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        Ok(Self {
            path: path.display().to_string(),
            size: meta.len(),
            modified_ns,
            kind: kind.to_owned(),
            params: params.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    /// the blob, if this entry has one
    pub file: Option<PathBuf>,
    /// json stored with it
    pub meta: Option<String>,
}

impl CacheEntry {
    pub fn meta_as<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_str(self.meta.as_deref()?).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CacheStats {
    #[ts(type = "number")]
    pub entries: u64,
    #[ts(type = "number")]
    pub bytes: u64,
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
    /// opens or creates the cache in `dir`
    pub fn open(dir: &Path, limit_bytes: u64) -> MediaResult<Self> {
        fs::create_dir_all(dir.join(BLOB_DIR)).map_err(|err| MediaError::io(dir, err))?;
        let index = dir.join(INDEX_FILE);
        let conn = match connect(&index) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!(%err, "the media cache index is unreadable, starting a new one");
                let aside = dir.join(format!("{INDEX_FILE}.broken-{}", now_ms()));
                let _ = fs::rename(&index, aside);
                for suffix in ["-wal", "-shm"] {
                    let _ = fs::remove_file(dir.join(format!("{INDEX_FILE}{suffix}")));
                }
                // the blobs belong to the old index, nothing can find them any more
                let _ = fs::remove_dir_all(dir.join(BLOB_DIR));
                fs::create_dir_all(dir.join(BLOB_DIR)).map_err(|err| MediaError::io(dir, err))?;
                connect(&index)?
            }
        };
        let cache = Self {
            dir: dir.to_owned(),
            conn: Mutex::new(conn),
            limit: AtomicU64::new(limit_bytes),
        };
        cache.remove_orphans();
        Ok(cache)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// where blobs live, for letting the webview show thumbnails
    pub fn blob_dir(&self) -> PathBuf {
        self.dir.join(BLOB_DIR)
    }

    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    /// changes the size limit and evicts at once if the cache is now over it
    pub fn set_limit(&self, bytes: u64) -> MediaResult<()> {
        self.limit.store(bytes, Ordering::Relaxed);
        let conn = self.conn.lock();
        self.evict(&conn)
    }

    /// the entry for `key`, if the file has not changed since it was made
    pub fn get(&self, key: &CacheKey) -> MediaResult<Option<CacheEntry>> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT id, size, modified_ns, file, meta, used_ms FROM entries
                 WHERE path = ?1 AND kind = ?2 AND params = ?3",
                params![key.path, key.kind, key.params],
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
        let Some((id, size, modified_ns, file, meta, used_ms)) = row else {
            return Ok(None);
        };
        let file = file.map(|name| self.blob_dir().join(name));
        let fresh = size == to_i64(key.size) && modified_ns == key.modified_ns;
        let blob_ok = file.as_ref().is_none_or(|f| f.is_file());
        if !fresh || !blob_ok {
            delete_rows(&conn, &self.blob_dir(), &[id])?;
            return Ok(None);
        }
        let now = now_ms();
        if now - used_ms > TOUCH_EVERY_MS {
            conn.execute(
                "UPDATE entries SET used_ms = ?1 WHERE id = ?2",
                params![now, id],
            )?;
        }
        Ok(Some(CacheEntry { file, meta }))
    }

    /// the json stored for `key`, read as `T`
    pub fn get_meta<T: DeserializeOwned>(&self, key: &CacheKey) -> Option<T> {
        match self.get(key) {
            Ok(entry) => entry?.meta_as(),
            Err(err) => {
                tracing::warn!(%err, "media cache read failed");
                None
            }
        }
    }

    /// stores json only, for things like probe results
    pub fn put_meta<T: Serialize>(&self, key: &CacheKey, value: &T) -> MediaResult<CacheEntry> {
        let meta =
            serde_json::to_string(value).map_err(|err| MediaError::Cache(err.to_string()))?;
        let conn = self.conn.lock();
        self.insert(&conn, key, None, 0, Some(meta))
    }

    /// stores a blob. `write` fills the temp file it is given; the entry only appears once
    /// it has succeeded, so a failed or cancelled job never leaves half a thumbnail.
    pub fn put_file(
        &self,
        key: &CacheKey,
        ext: &str,
        meta: Option<String>,
        write: impl FnOnce(&Path) -> MediaResult<()>,
    ) -> MediaResult<CacheEntry> {
        let name = blob_name(&key.kind, ext);
        let blob = self.blob_dir().join(&name);
        let temp = blob.with_extension(format!("{ext}.tmp"));
        if let Err(err) = write(&temp) {
            let _ = fs::remove_file(&temp);
            return Err(err);
        }
        let bytes = fs::metadata(&temp)
            .map_err(|err| MediaError::io(&temp, err))?
            .len();
        fs::rename(&temp, &blob).map_err(|err| MediaError::io(&blob, err))?;
        let conn = self.conn.lock();
        match self.insert(&conn, key, Some(&name), bytes, meta) {
            Ok(entry) => Ok(entry),
            Err(err) => {
                let _ = fs::remove_file(&blob);
                Err(err)
            }
        }
    }

    fn insert(
        &self,
        conn: &Connection,
        key: &CacheKey,
        file: Option<&str>,
        blob_bytes: u64,
        meta: Option<String>,
    ) -> MediaResult<CacheEntry> {
        // an older entry for the same file, kind and params is replaced, blob and all
        let old: Option<i64> = conn
            .query_row(
                "SELECT id FROM entries WHERE path = ?1 AND kind = ?2 AND params = ?3",
                params![key.path, key.kind, key.params],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(old) = old {
            delete_rows(conn, &self.blob_dir(), &[old])?;
        }
        let bytes = blob_bytes + meta.as_ref().map_or(0, |m| m.len() as u64);
        let now = now_ms();
        conn.execute(
            "INSERT INTO entries (path, size, modified_ns, kind, params, file, bytes, meta, created_ms, used_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                key.path,
                to_i64(key.size),
                key.modified_ns,
                key.kind,
                key.params,
                file,
                to_i64(bytes),
                meta,
                now
            ],
        )?;
        self.evict(conn)?;
        Ok(CacheEntry {
            file: file.map(|name| self.blob_dir().join(name)),
            meta,
        })
    }

    /// drops everything cached for one file, eg after it was edited in place
    pub fn forget(&self, path: &Path) -> MediaResult<()> {
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let conn = self.conn.lock();
        let ids = ids_where(&conn, "path = ?1", params![path.display().to_string()])?;
        delete_rows(&conn, &self.blob_dir(), &ids)
    }

    pub fn stats(&self) -> MediaResult<CacheStats> {
        let conn = self.conn.lock();
        let (entries, bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM entries",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(CacheStats {
            entries: u64::try_from(entries).unwrap_or(0),
            bytes: u64::try_from(bytes).unwrap_or(0),
            limit_bytes: self.limit(),
            path: self.dir.display().to_string(),
        })
    }

    /// empties the whole cache
    pub fn clear(&self) -> MediaResult<CacheStats> {
        {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM entries", [])?;
            let blobs = self.blob_dir();
            if let Ok(dir) = fs::read_dir(&blobs) {
                for entry in dir.flatten() {
                    let _ = fs::remove_file(entry.path());
                }
            }
            // give the space back to the disk too
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;");
        }
        self.stats()
    }

    /// removes least recently used entries until the cache fits its limit
    fn evict(&self, conn: &Connection) -> MediaResult<()> {
        let limit = to_i64(self.limit());
        loop {
            let total: i64 =
                conn.query_row("SELECT COALESCE(SUM(bytes), 0) FROM entries", [], |row| {
                    row.get(0)
                })?;
            if total <= limit {
                return Ok(());
            }
            let mut stmt = conn
                .prepare("SELECT id, bytes FROM entries ORDER BY used_ms ASC, id ASC LIMIT 64")?;
            let rows: Vec<(i64, i64)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            if rows.is_empty() {
                return Ok(());
            }
            let mut over = total - limit;
            let mut doomed = Vec::new();
            for (id, bytes) in rows {
                doomed.push(id);
                over -= bytes;
                if over <= 0 {
                    break;
                }
            }
            delete_rows(conn, &self.blob_dir(), &doomed)?;
        }
    }

    /// deletes blob files no entry points at (left by a crash between writing a blob and
    /// recording it) and stray temp files
    fn remove_orphans(&self) {
        let known: Vec<String> = {
            let conn = self.conn.lock();
            let stmt = conn.prepare("SELECT file FROM entries WHERE file IS NOT NULL");
            let Ok(mut stmt) = stmt else { return };
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        };
        let Ok(dir) = fs::read_dir(self.blob_dir()) else {
            return;
        };
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !known.contains(&name) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

fn connect(index: &Path) -> MediaResult<Connection> {
    let conn = Connection::open(index)?;
    conn.busy_timeout(Duration::from_secs(2))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(MediaError::Cache(format!(
            "the cache was made by a newer vellum (schema {version})"
        )));
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entries (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL,
            size INTEGER NOT NULL,
            modified_ns INTEGER NOT NULL,
            kind TEXT NOT NULL,
            params TEXT NOT NULL,
            file TEXT,
            bytes INTEGER NOT NULL,
            meta TEXT,
            created_ms INTEGER NOT NULL,
            used_ms INTEGER NOT NULL,
            UNIQUE (path, kind, params)
        );
        CREATE INDEX IF NOT EXISTS entries_used ON entries (used_ms);
        PRAGMA user_version = 1;",
    )?;
    // a quick look at the file catches a truncated or overwritten index now rather than
    // halfway through a job
    let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(MediaError::Cache(format!(
            "the cache index is damaged: {check}"
        )));
    }
    Ok(conn)
}

fn ids_where(
    conn: &Connection,
    filter: &str,
    args: impl rusqlite::Params,
) -> MediaResult<Vec<i64>> {
    let mut stmt = conn.prepare(&format!("SELECT id FROM entries WHERE {filter}"))?;
    let ids = stmt
        .query_map(args, |row| row.get(0))?
        .collect::<Result<Vec<i64>, _>>()?;
    Ok(ids)
}

fn delete_rows(conn: &Connection, blobs: &Path, ids: &[i64]) -> MediaResult<()> {
    for id in ids {
        let file: Option<Option<String>> = conn
            .query_row(
                "SELECT file FROM entries WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        conn.execute("DELETE FROM entries WHERE id = ?1", params![id])?;
        if let Some(Some(name)) = file {
            let _ = fs::remove_file(blobs.join(name));
        }
    }
    Ok(())
}

/// a unique blob file name, eg `thumb-1732140000123-7.jpg`
fn blob_name(kind: &str, ext: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let safe: String = kind
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{safe}-{}-{}-{n}.{ext}", now_ms(), std::process::id())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;

    use super::*;

    fn source(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn write_bytes(bytes: usize) -> impl FnOnce(&Path) -> MediaResult<()> {
        move |path: &Path| fs::write(path, vec![7u8; bytes]).map_err(|e| MediaError::io(path, e))
    }

    #[test]
    fn blobs_and_json_come_back_for_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 1 << 30).unwrap();
        let file = source(dir.path(), "a.png", b"pixels");
        let key = CacheKey::for_file(&file, "thumb", "320").unwrap();
        assert!(cache.get(&key).unwrap().is_none());

        let stored = cache
            .put_file(&key, "jpg", Some("{\"w\":3}".into()), write_bytes(10))
            .unwrap();
        let blob = stored.file.clone().unwrap();
        assert!(blob.is_file());
        let found = cache.get(&key).unwrap().unwrap();
        assert_eq!(found, stored);

        let probe = CacheKey::for_file(&file, "probe", "").unwrap();
        cache.put_meta(&probe, &vec![1, 2, 3]).unwrap();
        assert_eq!(cache.get_meta::<Vec<i32>>(&probe), Some(vec![1, 2, 3]));

        let stats = cache.stats().unwrap();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.bytes, 10 + 7 + 7);
    }

    #[test]
    fn a_changed_file_is_a_miss_and_its_old_blob_goes() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 1 << 30).unwrap();
        let file = source(dir.path(), "a.png", b"one");
        let key = CacheKey::for_file(&file, "thumb", "320").unwrap();
        let blob = cache
            .put_file(&key, "jpg", None, write_bytes(5))
            .unwrap()
            .file
            .unwrap();

        fs::write(&file, b"two plus more").unwrap();
        let new_key = CacheKey::for_file(&file, "thumb", "320").unwrap();
        assert!(cache.get(&new_key).unwrap().is_none());
        assert!(!blob.exists());
        assert_eq!(cache.stats().unwrap().entries, 0);
    }

    #[test]
    fn the_least_recently_used_entries_go_first_when_over_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 250).unwrap();
        let keys: Vec<CacheKey> = (0..3)
            .map(|i| {
                let file = source(dir.path(), &format!("{i}.png"), b"x");
                CacheKey::for_file(&file, "thumb", "320").unwrap()
            })
            .collect();
        cache
            .put_file(&keys[0], "jpg", None, write_bytes(100))
            .unwrap();
        sleep(Duration::from_millis(5));
        cache
            .put_file(&keys[1], "jpg", None, write_bytes(100))
            .unwrap();
        // touch the first so the second becomes the oldest. touches are throttled, so
        // write the time directly.
        cache
            .conn
            .lock()
            .execute(
                "UPDATE entries SET used_ms = ?1 WHERE path = ?2",
                params![now_ms() + 10, keys[0].path],
            )
            .unwrap();
        cache
            .put_file(&keys[2], "jpg", None, write_bytes(100))
            .unwrap();
        assert!(cache.get(&keys[0]).unwrap().is_some());
        assert!(cache.get(&keys[1]).unwrap().is_none());
        assert!(cache.get(&keys[2]).unwrap().is_some());
        assert!(cache.stats().unwrap().bytes <= 250);

        cache.set_limit(120).unwrap();
        assert_eq!(cache.stats().unwrap().entries, 1);
    }

    #[test]
    fn a_failed_write_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 1 << 30).unwrap();
        let file = source(dir.path(), "a.png", b"x");
        let key = CacheKey::for_file(&file, "thumb", "320").unwrap();
        let err = cache.put_file(&key, "jpg", None, |path| {
            fs::write(path, b"half").unwrap();
            Err(MediaError::Cancelled)
        });
        assert!(matches!(err, Err(MediaError::Cancelled)));
        assert!(cache.get(&key).unwrap().is_none());
        assert_eq!(fs::read_dir(cache.blob_dir()).unwrap().count(), 0);
    }

    #[test]
    fn clearing_and_forgetting_remove_blobs_too() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(&dir.path().join("cache"), 1 << 30).unwrap();
        let a = source(dir.path(), "a.png", b"x");
        let b = source(dir.path(), "b.png", b"y");
        for (file, kind) in [(&a, "thumb"), (&a, "waveform"), (&b, "thumb")] {
            let key = CacheKey::for_file(file, kind, "").unwrap();
            cache.put_file(&key, "bin", None, write_bytes(3)).unwrap();
        }
        cache.forget(&a).unwrap();
        assert_eq!(cache.stats().unwrap().entries, 1);
        let stats = cache.clear().unwrap();
        assert_eq!((stats.entries, stats.bytes), (0, 0));
        assert_eq!(fs::read_dir(cache.blob_dir()).unwrap().count(), 0);
    }

    #[test]
    fn a_broken_index_is_replaced_and_orphans_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        {
            let cache = MediaCache::open(&root, 1 << 30).unwrap();
            let file = source(dir.path(), "a.png", b"x");
            let key = CacheKey::for_file(&file, "thumb", "").unwrap();
            cache.put_file(&key, "jpg", None, write_bytes(3)).unwrap();
        }
        // a blob nobody knows about, as a crash would leave
        fs::write(root.join(BLOB_DIR).join("stray.jpg"), b"?").unwrap();
        let cache = MediaCache::open(&root, 1 << 30).unwrap();
        assert_eq!(cache.stats().unwrap().entries, 1);
        assert!(!root.join(BLOB_DIR).join("stray.jpg").exists());
        drop(cache);

        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(root.join(format!("{INDEX_FILE}{suffix}")));
        }
        fs::write(
            root.join(INDEX_FILE),
            b"this is not a database, it is a sandwich",
        )
        .unwrap();
        let cache = MediaCache::open(&root, 1 << 30).unwrap();
        assert_eq!(cache.stats().unwrap().entries, 0);
        let moved = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".broken-"));
        assert!(moved);
    }
}
