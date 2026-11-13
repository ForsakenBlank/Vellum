//! settings stored as one json file.
//!
//! the file is a set of named sections. each section is a plain serde struct
//! owned by whichever crate needs it, with its own version number and its own
//! migrations, so adding settings to one crate never touches another.
//! sections nobody has registered (a plugin that is not loaded, a newer build)
//! are kept as they are and written back untouched.
//!
//! ```json
//! { "format": 1, "sections": { "appearance": { "version": 1, "value": { "theme": "dark" } } } }
//! ```

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::event::{Event, EventBus};
use crate::fs::write_atomically;
use crate::time::unix_ms;

/// version of the file layout itself, not of any section
const FILE_FORMAT: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("could not read or write the settings file at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("there is no settings section called `{0}`")]
    UnknownSection(String),
    #[error("the value for `{section}` is not valid: {source}")]
    Invalid {
        section: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("`{section}` settings could not be upgraded from version {from}: {reason}")]
    Migration {
        section: String,
        from: u32,
        reason: String,
    },
}

/// one named group of settings.
pub trait SettingsSection:
    Serialize + DeserializeOwned + Default + Clone + PartialEq + Send + Sync + 'static
{
    /// key in the settings file, such as `appearance`
    const KEY: &'static str;

    /// bump this when the stored shape changes in a way serde defaults cannot absorb,
    /// and teach `migrate` the step from the old version
    const VERSION: u32 = 1;

    /// upgrades stored json by exactly one version, from `from` to `from + 1`.
    /// only fields that differ from the defaults are stored, so any field can be missing
    fn migrate(from: u32, _value: Value) -> Result<Value, String> {
        Err(format!("no migration from version {from}"))
    }

    /// clamps values into their valid ranges after loading and before saving
    fn sanitize(&mut self) {}
}

/// published after a section changed, with its new value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingsChanged {
    pub section: String,
    pub value: Value,
}

impl Event for SettingsChanged {
    const NAME: &'static str = "settings.changed";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredSection {
    version: u32,
    value: Value,
}

#[derive(Serialize, Deserialize)]
struct SettingsFile {
    format: u32,
    #[serde(default)]
    sections: BTreeMap<String, StoredSection>,
}

/// a registered section: its current value plus what is needed to accept
/// json for it without knowing the concrete type
struct Registered {
    type_id: TypeId,
    parse: fn(Value) -> Result<Parsed, SettingsError>,
    typed: Box<dyn Any + Send + Sync>,
    /// the full current value, defaults included
    json: Value,
}

struct Parsed {
    typed: Box<dyn Any + Send + Sync>,
    json: Value,
    /// what goes into the file, or none when everything is at its default
    stored: Option<StoredSection>,
}

#[derive(Default)]
struct State {
    /// exactly what is on disk, including sections this build does not know
    stored: BTreeMap<String, StoredSection>,
    registered: HashMap<&'static str, Registered>,
}

struct Inner {
    path: Option<PathBuf>,
    recovered: Option<PathBuf>,
    bus: Option<EventBus>,
    state: Mutex<State>,
}

#[derive(Clone)]
pub struct SettingsStore {
    inner: Arc<Inner>,
}

impl SettingsStore {
    /// loads the file at `path`, or starts empty if there is none yet.
    ///
    /// a file that cannot be understood is moved aside (see [`Self::recovered_file`])
    /// instead of being overwritten, and the store starts from defaults
    pub fn open(path: impl Into<PathBuf>, bus: Option<EventBus>) -> Result<Self, SettingsError> {
        let path = path.into();
        let (stored, recovered) = read_file(&path)?;
        Ok(Self::build(Some(path), recovered, bus, stored))
    }

    /// a store that never touches the disk
    pub fn in_memory(bus: Option<EventBus>) -> Self {
        Self::build(None, None, bus, BTreeMap::new())
    }

    fn build(
        path: Option<PathBuf>,
        recovered: Option<PathBuf>,
        bus: Option<EventBus>,
        stored: BTreeMap<String, StoredSection>,
    ) -> Self {
        let state = Mutex::new(State {
            stored,
            registered: HashMap::new(),
        });
        Self {
            inner: Arc::new(Inner {
                path,
                recovered,
                bus,
                state,
            }),
        }
    }

    /// where the unreadable settings file was moved to, if that happened on open
    pub fn recovered_file(&self) -> Option<&Path> {
        self.inner.recovered.as_deref()
    }

    /// makes a section reachable by key from the ui.
    /// typed access through `get` and `update` registers on first use
    pub fn register<S: SettingsSection>(&self) {
        self.ensure_registered::<S>(&mut self.inner.state.lock());
    }

    pub fn get<S: SettingsSection>(&self) -> S {
        let mut state = self.inner.state.lock();
        self.ensure_registered::<S>(&mut state);
        current::<S>(&state)
    }

    pub fn set<S: SettingsSection>(&self, value: S) -> Result<(), SettingsError> {
        self.update::<S>(|current| *current = value).map(|_| ())
    }

    /// changes a section in place, saves it and tells the ui. returns the new value
    pub fn update<S: SettingsSection>(
        &self,
        change: impl FnOnce(&mut S),
    ) -> Result<S, SettingsError> {
        let (next, changed) = {
            let mut state = self.inner.state.lock();
            self.ensure_registered::<S>(&mut state);
            let before = current::<S>(&state);
            let mut next = before.clone();
            change(&mut next);
            next.sanitize();
            if next == before {
                return Ok(next);
            }
            let changed = self.commit(&mut state, S::KEY, parsed(next.clone())?)?;
            (next, changed)
        };
        self.announce(changed);
        Ok(next)
    }

    pub fn reset<S: SettingsSection>(&self) -> Result<S, SettingsError> {
        self.update::<S>(|current| *current = S::default())
    }

    /// current value of a registered section as json, defaults included
    pub fn get_raw(&self, key: &str) -> Result<Value, SettingsError> {
        let state = self.inner.state.lock();
        state
            .registered
            .get(key)
            .map(|section| section.json.clone())
            .ok_or_else(|| SettingsError::UnknownSection(key.to_owned()))
    }

    /// replaces a registered section with json from the ui, after checking it
    /// against the section's type. returns the value as it ended up
    pub fn set_raw(&self, key: &str, value: Value) -> Result<Value, SettingsError> {
        let changed = {
            let mut state = self.inner.state.lock();
            let (key, section) = state
                .registered
                .get_key_value(key)
                .map(|(key, section)| (*key, section))
                .ok_or_else(|| SettingsError::UnknownSection(key.to_owned()))?;
            let parsed = (section.parse)(value)?;
            if parsed.json == section.json {
                return Ok(parsed.json);
            }
            self.commit(&mut state, key, parsed)?
        };
        let value = changed.value.clone();
        self.announce(changed);
        Ok(value)
    }

    /// every registered section as json, keyed by section name
    pub fn all(&self) -> BTreeMap<String, Value> {
        let state = self.inner.state.lock();
        state
            .registered
            .iter()
            .map(|(key, section)| ((*key).to_owned(), section.json.clone()))
            .collect()
    }

    /// reads the stored form of `S` into memory, migrating it first if it is old
    fn ensure_registered<S: SettingsSection>(&self, state: &mut State) {
        if let Some(existing) = state.registered.get(S::KEY) {
            let same_type = existing.type_id == TypeId::of::<S>();
            debug_assert!(
                same_type,
                "two settings sections share the key `{}`",
                S::KEY
            );
            return;
        }

        let mut upgraded = false;
        let loaded = match state.stored.get(S::KEY) {
            None => parsed(S::default()),
            Some(stored) => {
                upgraded = stored.version < S::VERSION;
                load_stored::<S>(stored)
            }
        };
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                // whatever is in the file stays there until this section is saved again
                tracing::warn!(section = S::KEY, %error, "using default settings");
                upgraded = false;
                Parsed {
                    typed: Box::new(S::default()),
                    json: Value::Null,
                    stored: None,
                }
            }
        };

        state.registered.insert(
            S::KEY,
            Registered {
                type_id: TypeId::of::<S>(),
                parse: parse_value::<S>,
                typed: loaded.typed,
                json: loaded.json,
            },
        );
        if upgraded {
            replace_stored(state, S::KEY, loaded.stored);
            if let Err(error) = self.persist(state) {
                tracing::warn!(section = S::KEY, %error, "could not save migrated settings");
            }
        }
    }

    fn commit(
        &self,
        state: &mut State,
        key: &'static str,
        parsed: Parsed,
    ) -> Result<SettingsChanged, SettingsError> {
        let previous = replace_stored(state, key, parsed.stored);
        if let Err(error) = self.persist(state) {
            // keep memory and disk in step when the write fails
            replace_stored(state, key, previous);
            return Err(error);
        }
        if let Some(section) = state.registered.get_mut(key) {
            section.typed = parsed.typed;
            section.json = parsed.json.clone();
        }
        Ok(SettingsChanged {
            section: key.to_owned(),
            value: parsed.json,
        })
    }

    fn announce(&self, changed: SettingsChanged) {
        if let Some(bus) = &self.inner.bus {
            bus.publish(changed);
        }
    }

    fn persist(&self, state: &State) -> Result<(), SettingsError> {
        let Some(path) = &self.inner.path else {
            return Ok(());
        };
        let file = SettingsFile {
            format: FILE_FORMAT,
            sections: state.stored.clone(),
        };
        let json = serde_json::to_vec_pretty(&file).map_err(|source| SettingsError::Invalid {
            section: "file".to_owned(),
            source,
        })?;
        write_atomically(path, &json).map_err(|source| SettingsError::Io {
            path: path.clone(),
            source,
        })
    }
}

fn current<S: SettingsSection>(state: &State) -> S {
    state
        .registered
        .get(S::KEY)
        .and_then(|section| section.typed.downcast_ref::<S>())
        .cloned()
        .unwrap_or_default()
}

fn replace_stored(
    state: &mut State,
    key: &str,
    stored: Option<StoredSection>,
) -> Option<StoredSection> {
    match stored {
        Some(stored) => state.stored.insert(key.to_owned(), stored),
        None => state.stored.remove(key),
    }
}

fn parsed<S: SettingsSection>(value: S) -> Result<Parsed, SettingsError> {
    let invalid = |source| SettingsError::Invalid {
        section: S::KEY.to_owned(),
        source,
    };
    let json = serde_json::to_value(&value).map_err(invalid)?;
    let defaults = serde_json::to_value(S::default()).map_err(invalid)?;
    let stored = sparse(&json, &defaults).map(|value| StoredSection {
        version: S::VERSION,
        value,
    });
    Ok(Parsed {
        typed: Box::new(value),
        json,
        stored,
    })
}

/// keeps only the fields that differ from the defaults, so settings the user
/// never touched follow the defaults of whichever build reads the file
fn sparse(value: &Value, defaults: &Value) -> Option<Value> {
    match (value, defaults) {
        (Value::Object(fields), Value::Object(defaults)) => {
            let changed: serde_json::Map<String, Value> = fields
                .iter()
                .filter(|(key, value)| defaults.get(key.as_str()) != Some(*value))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            (!changed.is_empty()).then_some(Value::Object(changed))
        }
        _ => (value != defaults).then(|| value.clone()),
    }
}

fn parse_value<S: SettingsSection>(value: Value) -> Result<Parsed, SettingsError> {
    let mut typed: S = serde_json::from_value(value).map_err(|source| SettingsError::Invalid {
        section: S::KEY.to_owned(),
        source,
    })?;
    typed.sanitize();
    parsed(typed)
}

fn load_stored<S: SettingsSection>(stored: &StoredSection) -> Result<Parsed, SettingsError> {
    let mut value = stored.value.clone();
    for from in stored.version..S::VERSION {
        value = S::migrate(from, value).map_err(|reason| SettingsError::Migration {
            section: S::KEY.to_owned(),
            from,
            reason,
        })?;
    }
    // a section written by a newer build is read as well as it can be,
    // serde skips the fields this build does not know
    parse_value::<S>(value)
}

fn read_file(
    path: &Path,
) -> Result<(BTreeMap<String, StoredSection>, Option<PathBuf>), SettingsError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((BTreeMap::new(), None));
        }
        Err(source) => {
            return Err(SettingsError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };

    match serde_json::from_slice::<SettingsFile>(&bytes) {
        Ok(file) if file.format <= FILE_FORMAT => Ok((file.sections, None)),
        unreadable => {
            let reason = match unreadable {
                Ok(file) => {
                    format!(
                        "it uses file format {}, this build reads {FILE_FORMAT}",
                        file.format
                    )
                }
                Err(error) => error.to_string(),
            };
            let aside = path.with_extension(format!("unreadable-{}.json", unix_ms()));
            fs::rename(path, &aside).map_err(|source| SettingsError::Io {
                path: path.to_owned(),
                source,
            })?;
            tracing::warn!(
                path = %path.display(),
                moved_to = %aside.display(),
                reason,
                "the settings file could not be read, starting from defaults"
            );
            Ok((BTreeMap::new(), Some(aside)))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    HighContrast,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AppearanceSettings {
    pub theme: Theme,
    /// accent colour as `#rrggbb`
    pub accent: String,
    pub ui_scale: f32,
    pub compact: bool,
    pub reduced_motion: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            accent: "#7c6cff".to_owned(),
            ui_scale: 1.0,
            compact: false,
            reduced_motion: false,
        }
    }
}

impl SettingsSection for AppearanceSettings {
    const KEY: &'static str = "appearance";

    fn sanitize(&mut self) {
        self.ui_scale = if self.ui_scale.is_finite() {
            self.ui_scale.clamp(0.75, 2.0)
        } else {
            1.0
        };
        let hex = self.accent.strip_prefix('#').unwrap_or("");
        if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            self.accent = Self::default().accent;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct PerformanceSettings {
    pub gpu_acceleration: bool,
    /// cpu heavy jobs allowed to run at once. 0 picks a number from the core count
    pub max_cpu_jobs: u32,
    pub max_gpu_jobs: u32,
    pub low_memory_mode: bool,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            gpu_acceleration: true,
            max_cpu_jobs: 0,
            max_gpu_jobs: 1,
            low_memory_mode: false,
        }
    }
}

impl SettingsSection for PerformanceSettings {
    const KEY: &'static str = "performance";

    fn sanitize(&mut self) {
        self.max_cpu_jobs = self.max_cpu_jobs.min(64);
        self.max_gpu_jobs = self.max_gpu_jobs.clamp(1, 8);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn temp_store() -> (TempDir, PathBuf, SettingsStore) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config").join("settings.json");
        let store = SettingsStore::open(&path, None).unwrap();
        (dir, path, store)
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn a_missing_file_gives_defaults_and_writes_nothing() {
        let (_dir, path, store) = temp_store();
        assert_eq!(
            store.get::<AppearanceSettings>(),
            AppearanceSettings::default()
        );
        assert!(!path.exists());
    }

    #[test]
    fn changes_are_saved_and_survive_a_reopen() {
        let (_dir, path, store) = temp_store();
        store
            .update::<AppearanceSettings>(|a| a.theme = Theme::Light)
            .unwrap();
        store
            .update::<PerformanceSettings>(|p| p.max_gpu_jobs = 2)
            .unwrap();

        let file = read_json(&path);
        assert_eq!(file["format"], 1);
        assert_eq!(file["sections"]["appearance"]["version"], 1);
        assert_eq!(file["sections"]["appearance"]["value"]["theme"], "light");

        let reopened = SettingsStore::open(&path, None).unwrap();
        assert_eq!(reopened.get::<AppearanceSettings>().theme, Theme::Light);
        assert_eq!(reopened.get::<PerformanceSettings>().max_gpu_jobs, 2);
        assert!(reopened.recovered_file().is_none());
    }

    #[test]
    fn no_temp_file_is_left_behind() {
        let (_dir, path, store) = temp_store();
        store
            .update::<AppearanceSettings>(|a| a.compact = true)
            .unwrap();
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["settings.json"]);
    }

    #[test]
    fn values_are_sanitised_on_the_way_in() {
        let store = SettingsStore::in_memory(None);
        let saved = store
            .update::<AppearanceSettings>(|a| {
                a.ui_scale = 40.0;
                a.accent = "red".into();
            })
            .unwrap();
        assert_eq!(saved.ui_scale, 2.0);
        assert_eq!(saved.accent, AppearanceSettings::default().accent);
    }

    #[test]
    fn missing_fields_fall_back_to_their_defaults() {
        let (_dir, path, _) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = json!({
            "format": 1,
            "sections": { "appearance": { "version": 1, "value": { "compact": true } } }
        });
        fs::write(&path, file.to_string()).unwrap();

        let appearance = SettingsStore::open(&path, None)
            .unwrap()
            .get::<AppearanceSettings>();
        assert!(appearance.compact);
        assert_eq!(appearance.ui_scale, 1.0);
    }

    #[test]
    fn a_change_publishes_one_event_and_a_no_op_publishes_none() {
        let bus = EventBus::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let _subscription =
            bus.subscribe(move |event: &SettingsChanged| sink.lock().push(event.clone()));
        let store = SettingsStore::in_memory(Some(bus));

        store
            .update::<AppearanceSettings>(|a| a.theme = Theme::HighContrast)
            .unwrap();
        store
            .update::<AppearanceSettings>(|a| a.theme = Theme::HighContrast)
            .unwrap();

        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].section, "appearance");
        assert_eq!(seen[0].value["theme"], "highContrast");
    }

    #[test]
    fn the_ui_can_read_and_write_sections_by_key() {
        let store = SettingsStore::in_memory(None);
        store.register::<AppearanceSettings>();
        store.register::<PerformanceSettings>();

        let all = store.all();
        assert_eq!(
            all.keys().collect::<Vec<_>>(),
            ["appearance", "performance"]
        );
        assert_eq!(all["appearance"]["uiScale"], 1.0);

        let mut appearance = store.get_raw("appearance").unwrap();
        appearance["theme"] = json!("light");
        appearance["uiScale"] = json!(9);
        let stored = store.set_raw("appearance", appearance).unwrap();
        assert_eq!(stored["uiScale"], 2.0);
        assert_eq!(store.get::<AppearanceSettings>().theme, Theme::Light);
    }

    #[test]
    fn the_ui_cannot_write_junk() {
        let store = SettingsStore::in_memory(None);
        store.register::<AppearanceSettings>();

        let wrong_type = store.set_raw("appearance", json!({ "theme": "neon" }));
        assert!(matches!(wrong_type, Err(SettingsError::Invalid { .. })));
        let unknown = store.set_raw("nope", json!({}));
        assert!(matches!(unknown, Err(SettingsError::UnknownSection(_))));
        assert_eq!(
            store.get::<AppearanceSettings>(),
            AppearanceSettings::default()
        );
    }

    #[test]
    fn sections_this_build_does_not_know_are_kept() {
        let (_dir, path, _) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let plugin = json!({ "version": 7, "value": { "token": "abc" } });
        fs::write(
            &path,
            json!({ "format": 1, "sections": { "some.plugin": plugin } }).to_string(),
        )
        .unwrap();

        let store = SettingsStore::open(&path, None).unwrap();
        store
            .update::<AppearanceSettings>(|a| a.compact = true)
            .unwrap();
        assert_eq!(read_json(&path)["sections"]["some.plugin"], plugin);
    }

    #[test]
    fn an_unreadable_file_is_moved_aside_not_overwritten() {
        let (_dir, path, _) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ this is not json").unwrap();

        let store = SettingsStore::open(&path, None).unwrap();
        let aside = store.recovered_file().unwrap().to_owned();
        assert_eq!(fs::read_to_string(&aside).unwrap(), "{ this is not json");
        assert!(!path.exists());
        assert_eq!(
            store.get::<AppearanceSettings>(),
            AppearanceSettings::default()
        );

        store
            .update::<AppearanceSettings>(|a| a.compact = true)
            .unwrap();
        assert!(path.exists() && aside.exists());
    }

    #[test]
    fn a_file_from_a_newer_layout_is_moved_aside_too() {
        let (_dir, path, _) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, json!({ "format": 99, "sections": {} }).to_string()).unwrap();
        assert!(
            SettingsStore::open(&path, None)
                .unwrap()
                .recovered_file()
                .is_some()
        );
    }

    /// version 1 stored `{ "size": 12 }`, version 2 renamed it to `fontSize`,
    /// version 3 turned it into a string with a unit
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", default)]
    struct Editor {
        font_size: String,
    }

    impl Default for Editor {
        fn default() -> Self {
            Self {
                font_size: "14px".into(),
            }
        }
    }

    impl SettingsSection for Editor {
        const KEY: &'static str = "editor";
        const VERSION: u32 = 3;

        fn migrate(from: u32, mut value: Value) -> Result<Value, String> {
            let map = value.as_object_mut().ok_or("expected an object")?;
            match from {
                1 => {
                    if let Some(size) = map.remove("size") {
                        map.insert("fontSize".into(), size);
                    }
                }
                2 => {
                    if let Some(size) = map.get("fontSize").and_then(Value::as_u64) {
                        map.insert("fontSize".into(), json!(format!("{size}px")));
                    }
                }
                other => return Err(format!("no migration from version {other}")),
            }
            Ok(value)
        }
    }

    fn write_editor(path: &Path, version: u32, value: Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let section = json!({ "version": version, "value": value });
        let file = json!({ "format": 1, "sections": { "editor": section } });
        fs::write(path, file.to_string()).unwrap();
    }

    #[test]
    fn old_sections_are_migrated_step_by_step_and_saved_back() {
        let (_dir, path, _) = temp_store();
        write_editor(&path, 1, json!({ "size": 18 }));

        let store = SettingsStore::open(&path, None).unwrap();
        assert_eq!(store.get::<Editor>().font_size, "18px");

        let file = read_json(&path);
        assert_eq!(file["sections"]["editor"]["version"], 3);
        assert_eq!(
            file["sections"]["editor"]["value"],
            json!({ "fontSize": "18px" })
        );
    }

    #[test]
    fn migration_can_start_from_any_older_version() {
        let (_dir, path, _) = temp_store();
        write_editor(&path, 2, json!({ "fontSize": 20 }));
        assert_eq!(
            SettingsStore::open(&path, None)
                .unwrap()
                .get::<Editor>()
                .font_size,
            "20px"
        );
    }

    #[test]
    fn a_failed_migration_uses_defaults_and_leaves_the_file_alone() {
        let (_dir, path, _) = temp_store();
        write_editor(&path, 1, json!("not an object"));
        let before = fs::read(&path).unwrap();

        let store = SettingsStore::open(&path, None).unwrap();
        assert_eq!(store.get::<Editor>(), Editor::default());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_section_from_a_newer_build_is_read_as_well_as_possible() {
        let (_dir, path, _) = temp_store();
        write_editor(&path, 9, json!({ "fontSize": "11px", "ligatures": true }));
        assert_eq!(
            SettingsStore::open(&path, None)
                .unwrap()
                .get::<Editor>()
                .font_size,
            "11px"
        );
    }

    #[test]
    fn reset_goes_back_to_defaults() {
        let store = SettingsStore::in_memory(None);
        store
            .update::<PerformanceSettings>(|p| p.low_memory_mode = true)
            .unwrap();
        assert_eq!(
            store.reset::<PerformanceSettings>().unwrap(),
            PerformanceSettings::default()
        );
    }

    #[test]
    fn only_changed_fields_are_written() {
        let (_dir, path, store) = temp_store();
        store
            .update::<AppearanceSettings>(|a| a.compact = true)
            .unwrap();
        store
            .update::<PerformanceSettings>(|p| p.low_memory_mode = true)
            .unwrap();
        assert_eq!(
            read_json(&path)["sections"]["appearance"]["value"],
            json!({ "compact": true })
        );

        // back at the defaults there is nothing left to store for the section
        store.reset::<AppearanceSettings>().unwrap();
        let file = read_json(&path);
        assert!(file["sections"].get("appearance").is_none());
        assert_eq!(
            file["sections"]["performance"]["value"],
            json!({ "lowMemoryMode": true })
        );
    }

    #[test]
    fn a_failed_write_reports_the_error_and_keeps_the_old_value() {
        let (dir, _path, store) = temp_store();
        // a file where the settings folder should be makes every write fail
        fs::write(dir.path().join("config"), "").unwrap();

        let result = store.update::<AppearanceSettings>(|a| a.compact = true);
        assert!(
            matches!(result, Err(SettingsError::Io { .. })),
            "{result:?}"
        );
        assert!(!store.get::<AppearanceSettings>().compact);
        assert_eq!(store.get_raw("appearance").unwrap()["compact"], false);
    }
}
