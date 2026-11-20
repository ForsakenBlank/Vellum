//! settings stored as json on disk, versioned, with migrations.
//!
//! the file is a json object with a `version` field next to the settings themselves:
//!
//! ```json
//! { "version": 1, "appearance": { "theme": "dark" }, "jobs": { "maxCpuJobs": 2 } }
//! ```
//!
//! loading runs the raw json through every migration from the file's version up to the
//! current one, then deserialises it. missing fields fall back to defaults and unknown
//! fields are ignored, so most changes need no migration at all. migrations are only
//! needed when a field is renamed, moved or changes meaning.
//!
//! the store never loses the user's file: a migrated or newer file is backed up before the
//! first write, an unreadable file is moved aside instead of overwritten, and every write
//! goes to a temp file that is then renamed over the real one.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::events::{Event, EventBus};
use crate::shortcuts::{self, ShortcutSettings};

const VERSION_KEY: &str = "version";

/// upgrades the raw json of a settings file from `from` to `from + 1`.
#[derive(Clone, Copy)]
pub struct Migration {
    pub from: u32,
    pub run: fn(&mut Map<String, Value>) -> Result<(), String>,
}

/// a settings type the store can load, migrate and save.
pub trait SettingsSchema:
    Serialize + DeserializeOwned + Default + Clone + PartialEq + Send + Sync + 'static
{
    /// bump this whenever a migration is added
    const VERSION: u32;
    /// event name sent when the settings change, eg `settings:changed`
    const CHANGED_EVENT: &'static str;

    /// one migration per version step, `from` 1 up to `VERSION - 1`
    fn migrations() -> Vec<Migration> {
        Vec::new()
    }

    /// clamps or resets values that are out of range. runs after every load and update.
    fn sanitize(&mut self) {}
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("could not read or write the settings file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("settings could not be converted: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no migration from settings version {0}")]
    MissingMigration(u32),
    #[error("settings migration from version {from} failed: {message}")]
    MigrationFailed { from: u32, message: String },
    #[error("settings must be a json object")]
    NotAnObject,
}

/// what happened when the settings were opened, so the app can tell the user if needed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// no file yet, defaults were used
    Fresh,
    Loaded,
    /// the file was upgraded, the original is kept at `backup`
    Migrated {
        from: u32,
        backup: PathBuf,
    },
    /// the file came from a newer vellum. it was read as well as possible and the original
    /// is kept at `backup` so going back to the newer version loses nothing.
    FromNewerVersion {
        version: u32,
        backup: PathBuf,
    },
    /// the file could not be read or migrated and was moved to `moved_to`, defaults were used
    Recovered {
        moved_to: PathBuf,
        reason: String,
    },
}

/// sent on the bus whenever the settings change
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsChanged<S> {
    pub settings: S,
}

impl<S: SettingsSchema> Event for SettingsChanged<S> {
    const NAME: &'static str = S::CHANGED_EVENT;
}

pub struct SettingsStore<S> {
    path: PathBuf,
    current: RwLock<S>,
    /// serialises writers so two updates cannot interleave their disk writes
    write_lock: Mutex<()>,
    bus: Option<EventBus>,
}

impl<S: SettingsSchema> SettingsStore<S> {
    /// loads settings from `path`, creating nothing on disk until the first change.
    pub fn open(
        path: impl Into<PathBuf>,
        bus: Option<EventBus>,
    ) -> Result<(Self, LoadOutcome), SettingsError> {
        let path = path.into();
        let (settings, outcome) = load::<S>(&path)?;
        let store = Self {
            path,
            current: RwLock::new(settings),
            write_lock: Mutex::new(()),
            bus,
        };
        Ok((store, outcome))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> S {
        self.current.read().clone()
    }

    pub fn read<R>(&self, f: impl FnOnce(&S) -> R) -> R {
        f(&self.current.read())
    }

    /// changes the settings, saves them and tells everyone. if saving fails nothing changes.
    pub fn update(&self, f: impl FnOnce(&mut S)) -> Result<S, SettingsError> {
        let _guard = self.write_lock.lock();
        let mut next = self.current.read().clone();
        f(&mut next);
        self.commit(next)
    }

    /// applies a json merge patch (rfc 7396) from the ui, eg `{ "appearance": { "theme": "light" } }`
    pub fn apply_patch(&self, patch: &Value) -> Result<S, SettingsError> {
        let _guard = self.write_lock.lock();
        let mut value = serde_json::to_value(&*self.current.read())?;
        merge_patch(&mut value, patch);
        let next: S = serde_json::from_value(value)?;
        self.commit(next)
    }

    pub fn reset(&self) -> Result<S, SettingsError> {
        let _guard = self.write_lock.lock();
        self.commit(S::default())
    }

    fn commit(&self, mut next: S) -> Result<S, SettingsError> {
        next.sanitize();
        if *self.current.read() == next && self.path.exists() {
            return Ok(next);
        }
        save(&self.path, &next)?;
        *self.current.write() = next.clone();
        if let Some(bus) = &self.bus {
            bus.emit(SettingsChanged {
                settings: next.clone(),
            });
        }
        Ok(next)
    }
}

fn load<S: SettingsSchema>(path: &Path) -> Result<(S, LoadOutcome), SettingsError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok((S::default(), LoadOutcome::Fresh));
        }
        Err(source) => {
            return Err(SettingsError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };

    match parse::<S>(&text) {
        Ok((mut settings, version)) => {
            settings.sanitize();
            let outcome = if version == S::VERSION {
                LoadOutcome::Loaded
            } else {
                let backup = sibling(path, &format!("v{version}.bak"));
                copy(path, &backup)?;
                save(path, &settings)?;
                if version < S::VERSION {
                    LoadOutcome::Migrated {
                        from: version,
                        backup,
                    }
                } else {
                    LoadOutcome::FromNewerVersion { version, backup }
                }
            };
            Ok((settings, outcome))
        }
        Err(err) => {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default();
            let moved_to = sibling(path, &format!("corrupt-{stamp}"));
            fs::rename(path, &moved_to).map_err(|source| SettingsError::Io {
                path: path.to_owned(),
                source,
            })?;
            tracing::warn!(?moved_to, %err, "settings file was unreadable, using defaults");
            Ok((
                S::default(),
                LoadOutcome::Recovered {
                    moved_to,
                    reason: err.to_string(),
                },
            ))
        }
    }
}

/// parses and migrates file contents, returning the settings and the file's version
fn parse<S: SettingsSchema>(text: &str) -> Result<(S, u32), SettingsError> {
    let Value::Object(mut map) = serde_json::from_str::<Value>(text)? else {
        return Err(SettingsError::NotAnObject);
    };
    // files without a version predate versioning, which means version 1
    let version = match map.remove(VERSION_KEY) {
        Some(value) => value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(SettingsError::NotAnObject)?,
        None => 1,
    };
    migrate::<S>(&mut map, version)?;
    let settings = serde_json::from_value(Value::Object(map))?;
    Ok((settings, version))
}

fn migrate<S: SettingsSchema>(
    map: &mut Map<String, Value>,
    from: u32,
) -> Result<(), SettingsError> {
    let migrations = S::migrations();
    let mut version = from;
    while version < S::VERSION {
        let step = migrations
            .iter()
            .find(|m| m.from == version)
            .ok_or(SettingsError::MissingMigration(version))?;
        (step.run)(map).map_err(|message| SettingsError::MigrationFailed {
            from: version,
            message,
        })?;
        version += 1;
    }
    Ok(())
}

fn save<S: SettingsSchema>(path: &Path, settings: &S) -> Result<(), SettingsError> {
    let mut value = serde_json::to_value(settings)?;
    let map = value.as_object_mut().ok_or(SettingsError::NotAnObject)?;
    map.insert(VERSION_KEY.to_owned(), Value::from(S::VERSION));
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

/// writes to a temp file next to the target, flushes it, then renames it into place so a
/// crash mid write can never leave a half written settings file
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SettingsError> {
    let io_err = |source| SettingsError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let tmp = sibling(path, "tmp");
    let mut file = fs::File::create(&tmp).map_err(io_err)?;
    file.write_all(bytes).map_err(io_err)?;
    file.sync_all().map_err(io_err)?;
    drop(file);
    fs::rename(&tmp, path).map_err(io_err)
}

fn copy(from: &Path, to: &Path) -> Result<(), SettingsError> {
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|source| SettingsError::Io {
            path: to.to_owned(),
            source,
        })
}

/// `settings.json` plus `bak` gives `settings.json.bak`
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

/// rfc 7396 json merge patch: objects merge, `null` removes, anything else replaces
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    if let Value::Object(target) = target {
        for (key, value) in patch {
            if value.is_null() {
                target.remove(key);
            } else {
                merge_patch(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    HighContrast,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct AppearanceSettings {
    pub theme: Theme,
    /// `#rrggbb`
    pub accent: String,
    pub ui_scale: f32,
    pub reduced_motion: bool,
    /// tighter rows and padding for small screens
    pub compact: bool,
}

/// the default accent, a warm safelight amber that reads well on the darkroom greys
pub const DEFAULT_ACCENT: &str = "#f2a93b";

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            accent: DEFAULT_ACCENT.to_owned(),
            ui_scale: 1.0,
            reduced_motion: false,
            compact: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct AutosaveSettings {
    pub enabled: bool,
    pub interval_secs: u32,
}

impl Default for AutosaveSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct JobSettings {
    pub max_cpu_jobs: u32,
    pub max_gpu_jobs: u32,
    pub max_io_jobs: u32,
}

impl Default for JobSettings {
    fn default() -> Self {
        // encoders and ai models already use many threads each, so a couple of jobs at a
        // time keeps the machine responsive. one gpu job avoids fighting over vram.
        Self {
            max_cpu_jobs: 2,
            max_gpu_jobs: 1,
            max_io_jobs: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct HistorySettings {
    pub max_steps: u32,
    pub max_memory_mb: u32,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self {
            max_steps: 500,
            max_memory_mb: 1024,
        }
    }
}

/// how vellum reads media files. used by `vellum-media`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct MediaSettings {
    /// the most disk space thumbnails, waveforms and other cached files may take
    pub cache_limit_mb: u32,
    /// a folder holding ffmpeg and ffprobe (or the ffmpeg program itself) to use instead
    /// of searching for them. empty searches.
    pub ffmpeg_path: String,
    /// use the graphics card's video encoders and decoders when they work
    pub hardware_acceleration: bool,
    /// the most memory one image may take when opened
    pub max_image_mb: u32,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            cache_limit_mb: 2048,
            ffmpeg_path: String::new(),
            hardware_acceleration: true,
            max_image_mb: 2048,
        }
    }
}

/// one group of panels shown as tabs inside a dock
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct DockGroup {
    pub id: String,
    /// panel ids in tab order. the ui owns the list of panels and drops ids it does not know.
    pub panels: Vec<String>,
    pub active: String,
    /// share of the dock this group gets, relative to the other groups
    pub weight: f32,
}

impl Default for DockGroup {
    fn default() -> Self {
        Self {
            id: String::new(),
            panels: Vec::new(),
            active: String::new(),
            weight: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct DockArea {
    /// width for the side docks, height for the bottom one, in css pixels at 100% scale
    pub size: u32,
    pub collapsed: bool,
    pub groups: Vec<DockGroup>,
}

impl Default for DockArea {
    fn default() -> Self {
        Self {
            size: 280,
            collapsed: false,
            groups: Vec::new(),
        }
    }
}

/// where every panel sits around the document area
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct DockLayout {
    pub left: DockArea,
    pub right: DockArea,
    pub bottom: DockArea,
}

impl Default for DockLayout {
    fn default() -> Self {
        let group = |id: &str, panels: &[&str]| DockGroup {
            id: id.to_owned(),
            panels: panels.iter().map(|p| (*p).to_owned()).collect(),
            active: panels.first().map(|p| (*p).to_owned()).unwrap_or_default(),
            weight: 1.0,
        };
        Self {
            left: DockArea {
                size: 260,
                ..DockArea::default()
            },
            right: DockArea {
                size: 300,
                collapsed: false,
                groups: vec![
                    group("right-top", &["info", "history"]),
                    group("right-bottom", &["jobs"]),
                ],
            },
            bottom: DockArea {
                size: 220,
                ..DockArea::default()
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedLayout {
    pub name: String,
    pub layout: DockLayout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct LayoutSettings {
    /// the layout on screen now, saved as the user drags and resizes
    pub current: DockLayout,
    /// name of the built in or saved layout the current one started from
    pub active: String,
    /// layouts the user saved by name
    pub saved: Vec<SavedLayout>,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            current: DockLayout::default(),
            active: "default".to_owned(),
            saved: Vec::new(),
        }
    }
}

const MAX_SAVED_LAYOUTS: usize = 20;

impl LayoutSettings {
    fn sanitize(&mut self) {
        sanitize_dock(&mut self.current);
        let mut names: Vec<String> = Vec::new();
        self.saved.retain_mut(|saved| {
            saved.name = saved.name.trim().chars().take(60).collect();
            let keep = !saved.name.is_empty() && !names.contains(&saved.name);
            names.push(saved.name.clone());
            sanitize_dock(&mut saved.layout);
            keep
        });
        self.saved.truncate(MAX_SAVED_LAYOUTS);
    }
}

fn sanitize_dock(layout: &mut DockLayout) {
    let areas = [
        (&mut layout.left, 160, 640),
        (&mut layout.right, 160, 640),
        (&mut layout.bottom, 100, 600),
    ];
    for (area, min, max) in areas {
        area.size = area.size.clamp(min, max);
        area.groups.retain(|group| !group.panels.is_empty());
        for group in &mut area.groups {
            if !group.weight.is_finite() {
                group.weight = 1.0;
            }
            group.weight = group.weight.clamp(0.1, 10.0);
            if !group.panels.contains(&group.active) {
                group.active = group.panels.first().cloned().unwrap_or_default();
            }
        }
    }
}

/// the app wide settings. later phases add sections here, bumping `VERSION` and adding a
/// migration only when an existing field changes shape.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub autosave: AutosaveSettings,
    pub jobs: JobSettings,
    pub history: HistorySettings,
    pub shortcuts: ShortcutSettings,
    pub layout: LayoutSettings,
    pub media: MediaSettings,
}

impl SettingsSchema for AppSettings {
    const VERSION: u32 = 1;
    const CHANGED_EVENT: &'static str = "settings:changed";

    fn sanitize(&mut self) {
        let defaults = Self::default();
        let look = &mut self.appearance;
        if !look.ui_scale.is_finite() {
            look.ui_scale = defaults.appearance.ui_scale;
        }
        look.ui_scale = look.ui_scale.clamp(0.5, 3.0);
        if !is_hex_colour(&look.accent) {
            look.accent = defaults.appearance.accent;
        }
        self.autosave.interval_secs = self.autosave.interval_secs.clamp(5, 3600);
        self.jobs.max_cpu_jobs = self.jobs.max_cpu_jobs.clamp(1, 64);
        self.jobs.max_gpu_jobs = self.jobs.max_gpu_jobs.clamp(1, 16);
        self.jobs.max_io_jobs = self.jobs.max_io_jobs.clamp(1, 64);
        self.history.max_steps = self.history.max_steps.clamp(10, 10_000);
        self.history.max_memory_mb = self.history.max_memory_mb.clamp(64, 64 * 1024);
        shortcuts::sanitize(&mut self.shortcuts);
        self.layout.sanitize();
        let media = &mut self.media;
        media.cache_limit_mb = media.cache_limit_mb.clamp(100, 1024 * 1024);
        media.max_image_mb = media.max_image_mb.clamp(256, 64 * 1024);
        media.ffmpeg_path = media.ffmpeg_path.trim().to_owned();
    }
}

fn is_hex_colour(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn temp() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        (dir, path)
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    /// a schema on version 3 that renamed `name` to `title` (1 to 2) and turned `size`
    /// from a string into a number (2 to 3)
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct Toy {
        title: String,
        size: u32,
    }

    impl Default for Toy {
        fn default() -> Self {
            Self {
                title: "untitled".into(),
                size: 10,
            }
        }
    }

    impl SettingsSchema for Toy {
        const VERSION: u32 = 3;
        const CHANGED_EVENT: &'static str = "toy:changed";

        fn migrations() -> Vec<Migration> {
            vec![
                Migration {
                    from: 1,
                    run: |map| {
                        if let Some(name) = map.remove("name") {
                            map.insert("title".into(), name);
                        }
                        Ok(())
                    },
                },
                Migration {
                    from: 2,
                    run: |map| {
                        if let Some(Value::String(size)) = map.get("size") {
                            let size: u32 = size.parse().map_err(|_| "size is not a number")?;
                            map.insert("size".into(), Value::from(size));
                        }
                        Ok(())
                    },
                },
            ]
        }
    }

    #[test]
    fn missing_file_gives_defaults_and_writes_nothing() {
        let (_dir, path) = temp();
        let (store, outcome) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        assert_eq!(outcome, LoadOutcome::Fresh);
        assert_eq!(store.get(), AppSettings::default());
        assert!(!path.exists());
    }

    #[test]
    fn update_saves_with_version_and_reloads() {
        let (_dir, path) = temp();
        let (store, _) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        store
            .update(|s| s.appearance.theme = Theme::HighContrast)
            .unwrap();

        let json = read_json(&path);
        assert_eq!(json["version"], 1);
        assert_eq!(json["appearance"]["theme"], "high-contrast");
        assert_eq!(json["jobs"]["maxCpuJobs"], 2);

        let (again, outcome) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(again.get().appearance.theme, Theme::HighContrast);
        assert!(!path.with_file_name("settings.json.tmp").exists());
    }

    #[test]
    fn missing_fields_use_defaults_and_unknown_fields_are_ignored() {
        let (_dir, path) = temp();
        fs::write(
            &path,
            r#"{ "version": 1, "appearance": { "uiScale": 1.5 }, "fromTheFuture": true }"#,
        )
        .unwrap();
        let (store, outcome) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        assert_eq!(outcome, LoadOutcome::Loaded);
        let settings = store.get();
        assert_eq!(settings.appearance.ui_scale, 1.5);
        assert_eq!(settings.appearance.theme, Theme::Dark);
        assert_eq!(settings.jobs, JobSettings::default());
    }

    #[test]
    fn migrations_run_in_order_and_keep_a_backup() {
        let (_dir, path) = temp();
        let original = r#"{ "version": 1, "name": "holiday", "size": "42" }"#;
        fs::write(&path, original).unwrap();

        let (store, outcome) = SettingsStore::<Toy>::open(&path, None).unwrap();
        let backup = path.with_file_name("settings.json.v1.bak");
        assert_eq!(
            outcome,
            LoadOutcome::Migrated {
                from: 1,
                backup: backup.clone()
            }
        );
        assert_eq!(
            store.get(),
            Toy {
                title: "holiday".into(),
                size: 42
            }
        );
        assert_eq!(fs::read_to_string(backup).unwrap(), original);
        assert_eq!(read_json(&path)["version"], 3);
    }

    #[test]
    fn files_without_a_version_count_as_version_one() {
        let (_dir, path) = temp();
        fs::write(&path, r#"{ "name": "old" }"#).unwrap();
        let (store, outcome) = SettingsStore::<Toy>::open(&path, None).unwrap();
        assert!(matches!(outcome, LoadOutcome::Migrated { from: 1, .. }));
        assert_eq!(store.get().title, "old");
    }

    #[test]
    fn failed_migration_keeps_the_original_file() {
        let (_dir, path) = temp();
        let original = r#"{ "version": 2, "size": "huge" }"#;
        fs::write(&path, original).unwrap();
        let (store, outcome) = SettingsStore::<Toy>::open(&path, None).unwrap();
        let LoadOutcome::Recovered { moved_to, reason } = outcome else {
            panic!("expected recovery, got {outcome:?}");
        };
        assert!(reason.contains("version 2"), "{reason}");
        assert_eq!(fs::read_to_string(moved_to).unwrap(), original);
        assert_eq!(store.get(), Toy::default());
    }

    #[test]
    fn newer_files_are_backed_up_before_being_rewritten() {
        let (_dir, path) = temp();
        let original = r#"{ "version": 9, "title": "future", "size": 3, "shiny": true }"#;
        fs::write(&path, original).unwrap();
        let (store, outcome) = SettingsStore::<Toy>::open(&path, None).unwrap();
        let backup = path.with_file_name("settings.json.v9.bak");
        assert_eq!(
            outcome,
            LoadOutcome::FromNewerVersion {
                version: 9,
                backup: backup.clone()
            }
        );
        assert_eq!(store.get().title, "future");
        assert_eq!(fs::read_to_string(backup).unwrap(), original);
    }

    #[test]
    fn corrupt_files_are_moved_aside() {
        let (dir, path) = temp();
        fs::write(&path, "{ this is not json").unwrap();
        let (store, outcome) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        let LoadOutcome::Recovered { moved_to, .. } = outcome else {
            panic!("expected recovery, got {outcome:?}");
        };
        assert_eq!(store.get(), AppSettings::default());
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&moved_to).unwrap(), "{ this is not json");
        assert_eq!(moved_to.parent(), Some(dir.path()));
    }

    #[test]
    fn wrong_types_count_as_corrupt() {
        let (_dir, path) = temp();
        fs::write(
            &path,
            r#"{ "version": 1, "jobs": { "maxCpuJobs": "many" } }"#,
        )
        .unwrap();
        let (_, outcome) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        assert!(matches!(outcome, LoadOutcome::Recovered { .. }));
    }

    #[test]
    fn sanitize_clamps_bad_values() {
        let (_dir, path) = temp();
        let (store, _) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        let settings = store
            .update(|s| {
                s.appearance.ui_scale = 40.0;
                s.appearance.accent = "red".into();
                s.jobs.max_cpu_jobs = 0;
                s.autosave.interval_secs = 1;
                s.media.cache_limit_mb = 1;
                s.media.ffmpeg_path = "  /opt/ffmpeg  ".into();
            })
            .unwrap();
        assert_eq!(settings.appearance.ui_scale, 3.0);
        assert_eq!(settings.appearance.accent, DEFAULT_ACCENT);
        assert_eq!(settings.jobs.max_cpu_jobs, 1);
        assert_eq!(settings.autosave.interval_secs, 5);
        assert_eq!(settings.media.cache_limit_mb, 100);
        assert_eq!(settings.media.ffmpeg_path, "/opt/ffmpeg");
    }

    #[test]
    fn sanitize_cleans_layouts_and_shortcuts() {
        let (_dir, path) = temp();
        let (store, _) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        let settings = store
            .update(|s| {
                s.layout.current.right.size = 5000;
                s.layout.current.left.groups.push(DockGroup {
                    id: "empty".into(),
                    ..DockGroup::default()
                });
                s.layout.current.right.groups[0].weight = f32::NAN;
                s.layout.current.right.groups[0].active = "gone".into();
                for name in ["  Editing ", "Editing", ""] {
                    s.layout.saved.push(SavedLayout {
                        name: name.into(),
                        layout: DockLayout::default(),
                    });
                }
                s.shortcuts
                    .overrides
                    .insert("file.save".into(), vec!["ctrl+alt+s".into()]);
            })
            .unwrap();
        let layout = &settings.layout;
        assert_eq!(layout.current.right.size, 640);
        assert!(layout.current.left.groups.is_empty());
        assert_eq!(layout.current.right.groups[0].weight, 1.0);
        assert_eq!(layout.current.right.groups[0].active, "info");
        let names: Vec<_> = layout.saved.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Editing"]);
        assert_eq!(settings.shortcuts.overrides["file.save"], ["Ctrl+Alt+S"]);
    }

    #[test]
    fn phase_0_files_load_without_a_migration() {
        let (_dir, path) = temp();
        fs::write(
            &path,
            r##"{ "version": 1, "appearance": { "theme": "light", "accent": "#7c5cff" } }"##,
        )
        .unwrap();
        let (store, outcome) = SettingsStore::<AppSettings>::open(&path, None).unwrap();
        assert_eq!(outcome, LoadOutcome::Loaded);
        let settings = store.get();
        assert_eq!(settings.appearance.accent, "#7c5cff");
        assert!(!settings.appearance.compact);
        assert_eq!(settings.layout, LayoutSettings::default());
        assert_eq!(settings.shortcuts, ShortcutSettings::default());
    }

    #[test]
    fn patches_from_the_ui_merge_and_emit() {
        let (_dir, path) = temp();
        let bus = EventBus::new();
        let changes = bus.subscribe::<SettingsChanged<AppSettings>>();
        let (store, _) = SettingsStore::<AppSettings>::open(&path, Some(bus)).unwrap();

        let settings = store
            .apply_patch(&json!({ "appearance": { "theme": "light" } }))
            .unwrap();
        assert_eq!(settings.appearance.theme, Theme::Light);
        assert_eq!(settings.appearance.accent, DEFAULT_ACCENT);
        assert_eq!(
            changes.try_recv().map(|c| c.settings.appearance.theme),
            Some(Theme::Light)
        );

        // a no-op change does not write or emit again
        store
            .apply_patch(&json!({ "appearance": { "theme": "light" } }))
            .unwrap();
        assert!(changes.try_recv().is_none());

        assert!(
            store
                .apply_patch(&json!({ "appearance": { "theme": "purple" } }))
                .is_err()
        );
        assert_eq!(store.get().appearance.theme, Theme::Light);

        store.reset().unwrap();
        assert_eq!(store.get(), AppSettings::default());
    }

    #[test]
    fn failed_save_keeps_the_old_settings() {
        let dir = tempfile::tempdir().unwrap();
        // a directory where the file should be makes the rename fail
        let path = dir.path().join("settings.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "x").unwrap();
        let store = SettingsStore::<AppSettings> {
            path: path.clone(),
            current: RwLock::new(AppSettings::default()),
            write_lock: Mutex::new(()),
            bus: None,
        };
        assert!(store.update(|s| s.appearance.theme = Theme::Light).is_err());
        assert_eq!(store.get().appearance.theme, Theme::Dark);
    }

    #[test]
    fn merge_patch_follows_the_rfc() {
        let mut doc = json!({ "a": { "b": 1, "c": 2 }, "d": [1, 2] });
        merge_patch(&mut doc, &json!({ "a": { "b": null, "e": 3 }, "d": [3] }));
        assert_eq!(doc, json!({ "a": { "c": 2, "e": 3 }, "d": [3] }));
    }

    #[test]
    fn every_schema_version_has_a_migration() {
        fn check<S: SettingsSchema>() {
            let froms: Vec<u32> = S::migrations().iter().map(|m| m.from).collect();
            let expected: Vec<u32> = (1..S::VERSION).collect();
            assert_eq!(froms, expected);
        }
        check::<AppSettings>();
        check::<Toy>();
    }
}
