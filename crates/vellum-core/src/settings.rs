//! app settings stored as versioned json on disk.
//!
//! loading never loses the user's file: a corrupt file is copied aside before defaults are used,
//! and a file written by a newer vellum is loaded read only so we never save over it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::events::{EventBus, VellumEvent};

/// bump this and add a migration to [`MIGRATIONS`] whenever a setting is renamed or reshaped.
pub const SETTINGS_VERSION: u32 = 1;

/// a migration takes the raw json for version `n` and turns it into version `n + 1`.
pub type Migration = fn(&mut Map<String, Value>) -> Result<(), String>;

/// `MIGRATIONS[i]` upgrades version `i + 1` to `i + 2`. empty until the first breaking change.
pub const MIGRATIONS: &[Migration] = &[];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    HighContrast,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct AppearanceSettings {
    pub theme: Theme,
    pub accent: String,
    pub ui_scale: f32,
    pub compact: bool,
    pub reduced_motion: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            accent: "#7c6cff".to_string(),
            ui_scale: 1.0,
            compact: false,
            reduced_motion: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct JobSettings {
    pub max_cpu_jobs: u32,
    pub max_gpu_jobs: u32,
    pub max_io_jobs: u32,
}

impl Default for JobSettings {
    fn default() -> Self {
        Self {
            max_cpu_jobs: 2,
            max_gpu_jobs: 1,
            max_io_jobs: 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct CacheSettings {
    pub limit_mb: u32,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self { limit_mb: 10_240 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
    pub version: u32,
    pub appearance: AppearanceSettings,
    pub autosave: AutosaveSettings,
    pub jobs: JobSettings,
    pub cache: CacheSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            appearance: AppearanceSettings::default(),
            autosave: AutosaveSettings::default(),
            jobs: JobSettings::default(),
            cache: CacheSettings::default(),
        }
    }
}

impl Settings {
    /// pulls hand edited or out of range values back into something the app can use.
    pub fn sanitize(&mut self) {
        let a = &mut self.appearance;
        a.ui_scale = if a.ui_scale.is_finite() {
            a.ui_scale.clamp(0.5, 3.0)
        } else {
            1.0
        };
        self.autosave.interval_secs = self.autosave.interval_secs.clamp(5, 3600);
        self.jobs.max_cpu_jobs = self.jobs.max_cpu_jobs.clamp(1, 64);
        self.jobs.max_gpu_jobs = self.jobs.max_gpu_jobs.clamp(1, 16);
        self.jobs.max_io_jobs = self.jobs.max_io_jobs.clamp(1, 64);
        self.cache.limit_mb = self.cache.limit_mb.max(256);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("could not read or write settings at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("settings file is not valid: {0}")]
    Invalid(String),
    #[error("could not upgrade settings from version {from}: {reason}")]
    Migration { from: u32, reason: String },
    #[error("settings were saved by a newer version of vellum, so changes will not be saved")]
    ReadOnly,
}

/// what happened while loading, so the app can tell the user if something was off.
#[derive(Debug, Default, PartialEq)]
pub enum LoadOutcome {
    #[default]
    Loaded,
    CreatedDefaults,
    Migrated {
        from: u32,
    },
    /// the file could not be read, it was copied to `backup` and defaults are in use
    RecoveredFromCorrupt {
        backup: PathBuf,
        reason: String,
    },
    /// written by a newer version: values we understand are used but nothing is saved
    NewerVersion {
        found: u32,
    },
}

pub struct SettingsStore {
    path: PathBuf,
    settings: Settings,
    read_only: bool,
    bus: Option<EventBus<VellumEvent>>,
}

impl SettingsStore {
    pub fn load(path: impl Into<PathBuf>) -> Result<(Self, LoadOutcome), SettingsError> {
        Self::load_with(path, MIGRATIONS)
    }

    /// same as [`SettingsStore::load`] with a custom migration list, mostly for tests.
    pub fn load_with(
        path: impl Into<PathBuf>,
        migrations: &[Migration],
    ) -> Result<(Self, LoadOutcome), SettingsError> {
        let path = path.into();
        let current = migrations.len() as u32 + 1;
        let mut store = Self {
            path,
            settings: Settings::default(),
            read_only: false,
            bus: None,
        };

        let text = match fs::read_to_string(&store.path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                store.settings.version = current;
                store.save()?;
                return Ok((store, LoadOutcome::CreatedDefaults));
            }
            Err(source) => {
                return Err(SettingsError::Io {
                    path: store.path.clone(),
                    source,
                })
            }
        };

        match parse(&text, migrations) {
            Ok((settings, from)) => {
                store.settings = settings;
                let outcome = if from > current {
                    store.read_only = true;
                    LoadOutcome::NewerVersion { found: from }
                } else if from < current {
                    store.save()?;
                    LoadOutcome::Migrated { from }
                } else {
                    LoadOutcome::Loaded
                };
                Ok((store, outcome))
            }
            Err(reason) => {
                let backup = store.backup_path();
                fs::copy(&store.path, &backup).map_err(|source| SettingsError::Io {
                    path: backup.clone(),
                    source,
                })?;
                store.settings.version = current;
                store.save()?;
                Ok((store, LoadOutcome::RecoveredFromCorrupt { backup, reason }))
            }
        }
    }

    /// publish a [`VellumEvent::SettingsChanged`] on `bus` after every change.
    pub fn with_events(mut self, bus: EventBus<VellumEvent>) -> Self {
        self.bus = Some(bus);
        self
    }

    pub fn get(&self) -> &Settings {
        &self.settings
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// changes settings with a closure, then sanitises, saves and announces them.
    pub fn update(&mut self, change: impl FnOnce(&mut Settings)) -> Result<(), SettingsError> {
        if self.read_only {
            return Err(SettingsError::ReadOnly);
        }
        let mut next = self.settings.clone();
        change(&mut next);
        self.commit(next)
    }

    /// deep merges a json object (in the camelCase shape the ui uses) into the settings.
    pub fn patch(&mut self, patch: Value) -> Result<(), SettingsError> {
        if self.read_only {
            return Err(SettingsError::ReadOnly);
        }
        let mut json = serde_json::to_value(&self.settings)
            .map_err(|e| SettingsError::Invalid(e.to_string()))?;
        merge_json(&mut json, patch);
        let next: Settings =
            serde_json::from_value(json).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        self.commit(next)
    }

    fn commit(&mut self, mut next: Settings) -> Result<(), SettingsError> {
        next.sanitize();
        next.version = self.settings.version;
        if next == self.settings {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.settings, next);
        if let Err(e) = self.save() {
            self.settings = previous;
            return Err(e);
        }
        if let Some(bus) = &self.bus {
            bus.publish(VellumEvent::SettingsChanged(self.settings.clone()));
        }
        Ok(())
    }

    /// writes to a temp file then renames it, so a crash mid write never leaves half a file.
    fn save(&self) -> Result<(), SettingsError> {
        let io_err = |source| SettingsError::Io {
            path: self.path.clone(),
            source,
        };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(io_err)?;
        }
        let text = serde_json::to_string_pretty(&self.settings)
            .map_err(|e| SettingsError::Invalid(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(io_err)?;
        fs::rename(&tmp, &self.path).map_err(io_err)
    }

    fn backup_path(&self) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let name = self
            .path
            .file_name()
            .map_or_else(|| "settings.json".into(), |n| n.to_string_lossy());
        self.path.with_file_name(format!("{name}.broken-{stamp}"))
    }
}

/// parses settings text, running migrations. returns the settings and the version found on disk.
fn parse(text: &str, migrations: &[Migration]) -> Result<(Settings, u32), String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Value::Object(mut map) = value else {
        return Err("expected a json object".to_string());
    };
    // files from before versioning existed count as version 1
    let found = match map.get("version") {
        None => 1,
        Some(v) => v
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|&v| v >= 1)
            .ok_or_else(|| format!("bad version field: {v}"))?,
    };

    let mut version = found;
    while let Some(migrate) = migrations.get(version as usize - 1) {
        migrate(&mut map).map_err(|reason| {
            SettingsError::Migration {
                from: version,
                reason,
            }
            .to_string()
        })?;
        version += 1;
        map.insert("version".into(), Value::from(version));
    }

    let mut settings: Settings =
        serde_json::from_value(Value::Object(map)).map_err(|e| e.to_string())?;
    settings.sanitize();
    // keep the real version so a newer file is never mistaken for ours
    settings.version = version.max(found);
    Ok((settings, found))
}

fn merge_json(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                match base.get_mut(&key) {
                    Some(slot) => merge_json(slot, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (slot, value) => *slot = value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        (dir, path)
    }

    #[test]
    fn first_launch_writes_defaults() {
        let (_dir, path) = temp_path();
        let (store, outcome) = SettingsStore::load(&path).unwrap();
        assert_eq!(outcome, LoadOutcome::CreatedDefaults);
        assert_eq!(store.get(), &Settings::default());
        assert!(path.exists());

        let (again, outcome) = SettingsStore::load(&path).unwrap();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(again.get(), store.get());
    }

    #[test]
    fn update_saves_sanitises_and_publishes() {
        let (_dir, path) = temp_path();
        let bus = EventBus::new();
        let rx = bus.subscribe();
        let (store, _) = SettingsStore::load(&path).unwrap();
        let mut store = store.with_events(bus);

        store
            .update(|s| {
                s.appearance.theme = Theme::Light;
                s.appearance.ui_scale = 40.0;
            })
            .unwrap();
        assert_eq!(store.get().appearance.ui_scale, 3.0);

        let (reloaded, _) = SettingsStore::load(&path).unwrap();
        assert_eq!(reloaded.get().appearance.theme, Theme::Light);
        assert!(matches!(
            rx.try_recv(),
            Ok(VellumEvent::SettingsChanged(s)) if s.appearance.theme == Theme::Light
        ));

        // a no-op change does not write or publish again
        store.update(|_| {}).unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn patch_merges_nested_camel_case_json() {
        let (_dir, path) = temp_path();
        let (mut store, _) = SettingsStore::load(&path).unwrap();
        store
            .patch(
                json!({ "appearance": { "theme": "highContrast" }, "jobs": { "maxGpuJobs": 2 } }),
            )
            .unwrap();
        let s = store.get();
        assert_eq!(s.appearance.theme, Theme::HighContrast);
        assert_eq!(s.appearance.accent, AppearanceSettings::default().accent);
        assert_eq!(s.jobs.max_gpu_jobs, 2);

        let err = store.patch(json!({ "appearance": { "theme": "purple" } }));
        assert!(matches!(err, Err(SettingsError::Invalid(_))));
        assert_eq!(store.get().appearance.theme, Theme::HighContrast);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let (_dir, path) = temp_path();
        fs::write(
            &path,
            r#"{ "version": 1, "autosave": { "enabled": false } }"#,
        )
        .unwrap();
        let (store, outcome) = SettingsStore::load(&path).unwrap();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert!(!store.get().autosave.enabled);
        assert_eq!(store.get().autosave.interval_secs, 60);
    }

    #[test]
    fn corrupt_file_is_backed_up_not_lost() {
        let (dir, path) = temp_path();
        fs::write(&path, "{ this is not json").unwrap();
        let (store, outcome) = SettingsStore::load(&path).unwrap();
        let LoadOutcome::RecoveredFromCorrupt { backup, .. } = outcome else {
            panic!("expected recovery, got {outcome:?}");
        };
        assert!(backup.starts_with(dir.path()));
        assert_eq!(fs::read_to_string(backup).unwrap(), "{ this is not json");
        assert_eq!(store.get(), &Settings::default());
    }

    #[test]
    fn newer_files_load_read_only() {
        let (_dir, path) = temp_path();
        let text = r#"{ "version": 99, "appearance": { "theme": "light" }, "futureThing": true }"#;
        fs::write(&path, text).unwrap();
        let (mut store, outcome) = SettingsStore::load(&path).unwrap();
        assert_eq!(outcome, LoadOutcome::NewerVersion { found: 99 });
        assert_eq!(store.get().appearance.theme, Theme::Light);
        assert!(store.is_read_only());
        assert!(matches!(
            store.update(|s| s.autosave.enabled = false),
            Err(SettingsError::ReadOnly)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), text, "file untouched");
    }

    #[test]
    fn migrations_run_in_order_and_resave() {
        fn v1_to_v2(map: &mut Map<String, Value>) -> Result<(), String> {
            // pretend v1 stored the theme at the top level
            if let Some(theme) = map.remove("theme") {
                map.insert("appearance".into(), json!({ "theme": theme }));
            }
            Ok(())
        }
        fn v2_to_v3(map: &mut Map<String, Value>) -> Result<(), String> {
            map.insert("autosave".into(), json!({ "intervalSecs": 120 }));
            Ok(())
        }

        let (_dir, path) = temp_path();
        fs::write(&path, r#"{ "version": 1, "theme": "light" }"#).unwrap();
        let (store, outcome) = SettingsStore::load_with(&path, &[v1_to_v2, v2_to_v3]).unwrap();
        assert_eq!(outcome, LoadOutcome::Migrated { from: 1 });
        assert_eq!(store.get().version, 3);
        assert_eq!(store.get().appearance.theme, Theme::Light);
        assert_eq!(store.get().autosave.interval_secs, 120);

        let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 3);
    }

    #[test]
    fn failing_migration_is_treated_as_corrupt() {
        fn broken(_: &mut Map<String, Value>) -> Result<(), String> {
            Err("nope".into())
        }
        let (_dir, path) = temp_path();
        fs::write(&path, r#"{ "version": 1 }"#).unwrap();
        let (_, outcome) = SettingsStore::load_with(&path, &[broken]).unwrap();
        assert!(matches!(outcome, LoadOutcome::RecoveredFromCorrupt { .. }));
    }
}
