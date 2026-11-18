//! the command system.
//!
//! every user action that changes a document is a [`Command`]. buttons, menus, shortcuts,
//! macros, the cli and plugins all build the same command values, which is what gives
//! undo/redo, macro recording, palette search and scripting from one place.
//!
//! a command serialises to a [`CommandRecord`] (a stable `kind` plus json `params`). records
//! are what the ui sends over ipc, what macros store and what the cli parses, and a
//! [`CommandRegistry`] turns a record back into a live command.

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// a reversible, recordable action on a target of type `T`, usually a document.
///
/// implementors keep whatever they need to undo themselves (old values, replaced pixels)
/// in fields marked `#[serde(skip)]`, so the serialised form only carries the user's intent.
pub trait Command<T>: Any + Send {
    /// stable id used in records, eg `image.layer.set_opacity`. never change it once shipped,
    /// saved macros depend on it.
    fn kind(&self) -> &'static str;

    /// short name for the history panel and the undo menu, eg "Set layer opacity".
    fn name(&self) -> String;

    /// performs the action. on error the target must be left exactly as it was.
    fn apply(&mut self, target: &mut T) -> Result<(), CommandError>;

    /// reverses the last successful `apply`.
    fn revert(&mut self, target: &mut T) -> Result<(), CommandError>;

    /// tries to fold `next`, which has already been applied, into this command so that a
    /// slider drag or a run of typing becomes a single undo step. returns true if it merged,
    /// in which case `next` is dropped. use [`downcast`] to check the type of `next`.
    fn merge(&mut self, next: &dyn Command<T>) -> bool {
        let _ = next;
        false
    }

    /// the parameters needed to rebuild this command from scratch. undo state must not be
    /// part of it. most commands just return `params_of(self)`.
    fn params(&self) -> Result<serde_json::Value, CommandError>;

    /// false for actions that do not change the target, such as starting an export. they
    /// still run and still get recorded in macros, but never land on the undo stack.
    fn undoable(&self) -> bool {
        true
    }

    /// rough number of bytes this command holds for undo. history uses it to stay inside
    /// its memory budget, so commands that keep pixels or samples should override it.
    fn memory_cost(&self) -> usize {
        std::mem::size_of_val(self)
    }
}

/// serialises a command's fields into params, for use in [`Command::params`].
pub fn params_of<S: Serialize>(value: &S) -> Result<serde_json::Value, CommandError> {
    serde_json::to_value(value).map_err(|err| CommandError::Failed(err.to_string()))
}

/// downcasts a command trait object to a concrete type, mainly for [`Command::merge`].
pub fn downcast<T: 'static, C: Command<T>>(command: &dyn Command<T>) -> Option<&C> {
    let any: &dyn Any = command;
    any.downcast_ref::<C>()
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    Failed(String),
    #[error("no command is registered as `{0}`")]
    UnknownKind(String),
    #[error("a command is already registered as `{0}`")]
    DuplicateKind(String),
    #[error("`{expected}` was registered with a type that reports `{actual}`")]
    KindMismatch {
        expected: String,
        actual: &'static str,
    },
    #[error("invalid parameters for `{kind}`: {source}")]
    InvalidParams {
        kind: String,
        #[source]
        source: serde_json::Error,
    },
}

impl CommandError {
    pub fn failed(message: impl fmt::Display) -> Self {
        Self::Failed(message.to_string())
    }
}

/// the serialised form of a command: what the ui sends, what macros store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommandRecord {
    pub kind: String,
    #[ts(type = "unknown")]
    pub params: serde_json::Value,
}

impl CommandRecord {
    pub fn new(kind: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            params,
        }
    }

    pub fn of<T: 'static>(command: &dyn Command<T>) -> Result<Self, CommandError> {
        Ok(Self::new(command.kind(), command.params()?))
    }
}

/// static description of a command for the command palette, menus and docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommandInfo {
    pub kind: &'static str,
    /// title shown in the palette, eg "Set layer opacity"
    pub title: &'static str,
    /// palette group, eg "Layer"
    pub category: &'static str,
    /// one plain sentence for tooltips and search
    pub description: &'static str,
}

type Factory<T> = fn(serde_json::Value) -> Result<Box<dyn Command<T>>, serde_json::Error>;

struct Registration<T> {
    info: CommandInfo,
    build: Factory<T>,
}

/// maps command kinds to factories, so records from the ui, macros or the cli can be
/// turned back into commands. kept explicit (no linker magic) so plugins can register
/// at runtime and the list is easy to inspect.
pub struct CommandRegistry<T> {
    entries: BTreeMap<&'static str, Registration<T>>,
}

impl<T: 'static> CommandRegistry<T> {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn register<C>(&mut self, info: CommandInfo) -> Result<(), CommandError>
    where
        C: Command<T> + DeserializeOwned,
    {
        if self.entries.contains_key(info.kind) {
            return Err(CommandError::DuplicateKind(info.kind.to_owned()));
        }
        self.entries.insert(
            info.kind,
            Registration {
                info,
                build: build_command::<T, C>,
            },
        );
        Ok(())
    }

    pub fn instantiate(&self, record: &CommandRecord) -> Result<Box<dyn Command<T>>, CommandError> {
        let entry = self
            .entries
            .get(record.kind.as_str())
            .ok_or_else(|| CommandError::UnknownKind(record.kind.clone()))?;
        let command =
            (entry.build)(record.params.clone()).map_err(|source| CommandError::InvalidParams {
                kind: record.kind.clone(),
                source,
            })?;
        if command.kind() != entry.info.kind {
            return Err(CommandError::KindMismatch {
                expected: entry.info.kind.to_owned(),
                actual: command.kind(),
            });
        }
        Ok(command)
    }

    pub fn info(&self, kind: &str) -> Option<&CommandInfo> {
        self.entries.get(kind).map(|entry| &entry.info)
    }

    /// every registered command, sorted by kind
    pub fn infos(&self) -> impl Iterator<Item = &CommandInfo> {
        self.entries.values().map(|entry| &entry.info)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<T: 'static> Default for CommandRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for CommandRegistry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.entries.keys()).finish()
    }
}

fn build_command<T, C>(params: serde_json::Value) -> Result<Box<dyn Command<T>>, serde_json::Error>
where
    T: 'static,
    C: Command<T> + DeserializeOwned,
{
    let command: C = serde_json::from_value(params)?;
    Ok(Box::new(command))
}

#[cfg(test)]
pub(crate) mod test_support {
    //! a tiny document and a few commands shared by the command and history tests.

    use super::*;

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct Doc {
        pub value: i32,
        pub log: Vec<String>,
    }

    /// adds a number, merges with the next `Add` like a slider drag would
    #[derive(Debug, Serialize, Deserialize)]
    pub struct Add {
        pub amount: i32,
    }

    impl Command<Doc> for Add {
        fn kind(&self) -> &'static str {
            "test.add"
        }
        fn name(&self) -> String {
            format!("Add {}", self.amount)
        }
        fn apply(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            doc.value += self.amount;
            Ok(())
        }
        fn revert(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            doc.value -= self.amount;
            Ok(())
        }
        fn merge(&mut self, next: &dyn Command<Doc>) -> bool {
            match downcast::<Doc, Add>(next) {
                Some(next) => {
                    self.amount += next.amount;
                    true
                }
                None => false,
            }
        }
        fn params(&self) -> Result<serde_json::Value, CommandError> {
            params_of(self)
        }
    }

    /// sets the value, keeping the old one for undo
    #[derive(Debug, Serialize, Deserialize)]
    pub struct Set {
        pub value: i32,
        #[serde(skip)]
        pub previous: Option<i32>,
    }

    impl Set {
        pub fn new(value: i32) -> Self {
            Self {
                value,
                previous: None,
            }
        }
    }

    impl Command<Doc> for Set {
        fn kind(&self) -> &'static str {
            "test.set"
        }
        fn name(&self) -> String {
            format!("Set to {}", self.value)
        }
        fn apply(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            self.previous = Some(doc.value);
            doc.value = self.value;
            Ok(())
        }
        fn revert(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            let previous = self
                .previous
                .ok_or_else(|| CommandError::failed("set was never applied"))?;
            doc.value = previous;
            Ok(())
        }
        fn params(&self) -> Result<serde_json::Value, CommandError> {
            params_of(self)
        }
    }

    /// always fails, to check that failures leave history alone
    #[derive(Debug, Serialize, Deserialize)]
    pub struct Fail;

    impl Command<Doc> for Fail {
        fn kind(&self) -> &'static str {
            "test.fail"
        }
        fn name(&self) -> String {
            "Fail".into()
        }
        fn apply(&mut self, _doc: &mut Doc) -> Result<(), CommandError> {
            Err(CommandError::failed("nope"))
        }
        fn revert(&mut self, _doc: &mut Doc) -> Result<(), CommandError> {
            Ok(())
        }
        fn params(&self) -> Result<serde_json::Value, CommandError> {
            Ok(serde_json::Value::Null)
        }
    }

    /// does not touch the document, like starting an export
    #[derive(Debug, Serialize, Deserialize)]
    pub struct Note {
        pub text: String,
    }

    impl Command<Doc> for Note {
        fn kind(&self) -> &'static str {
            "test.note"
        }
        fn name(&self) -> String {
            "Note".into()
        }
        fn apply(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            doc.log.push(self.text.clone());
            Ok(())
        }
        fn revert(&mut self, _doc: &mut Doc) -> Result<(), CommandError> {
            Ok(())
        }
        fn params(&self) -> Result<serde_json::Value, CommandError> {
            params_of(self)
        }
        fn undoable(&self) -> bool {
            false
        }
    }

    pub fn info(kind: &'static str) -> CommandInfo {
        CommandInfo {
            kind,
            title: kind,
            category: "Test",
            description: "test command",
        }
    }

    pub fn registry() -> CommandRegistry<Doc> {
        let mut registry = CommandRegistry::new();
        registry.register::<Add>(info("test.add")).unwrap();
        registry.register::<Set>(info("test.set")).unwrap();
        registry.register::<Note>(info("test.note")).unwrap();
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use serde_json::json;

    #[test]
    fn record_round_trip_skips_undo_state() {
        let mut set = Set::new(7);
        let mut doc = Doc {
            value: 3,
            ..Doc::default()
        };
        set.apply(&mut doc).unwrap();

        let record = CommandRecord::of::<Doc>(&set).unwrap();
        assert_eq!(
            record,
            CommandRecord::new("test.set", json!({ "value": 7 }))
        );

        let text = serde_json::to_string(&record).unwrap();
        let back: CommandRecord = serde_json::from_str(&text).unwrap();
        let mut rebuilt = registry().instantiate(&back).unwrap();
        let mut other = Doc::default();
        rebuilt.apply(&mut other).unwrap();
        assert_eq!(other.value, 7);
        rebuilt.revert(&mut other).unwrap();
        assert_eq!(other.value, 0);
    }

    #[test]
    fn unknown_kind_is_an_error() {
        let err = registry()
            .instantiate(&CommandRecord::new("test.nope", json!({})))
            .err()
            .unwrap();
        assert!(matches!(err, CommandError::UnknownKind(kind) if kind == "test.nope"));
    }

    #[test]
    fn bad_params_are_an_error() {
        let err = registry()
            .instantiate(&CommandRecord::new("test.add", json!({ "amount": "lots" })))
            .err()
            .unwrap();
        assert!(matches!(err, CommandError::InvalidParams { .. }));
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let mut registry = registry();
        let err = registry.register::<Add>(info("test.add")).unwrap_err();
        assert!(matches!(err, CommandError::DuplicateKind(_)));
    }

    #[test]
    fn mismatched_kind_is_caught() {
        let mut registry = CommandRegistry::<Doc>::new();
        registry.register::<Add>(info("test.wrong")).unwrap();
        let err = registry
            .instantiate(&CommandRecord::new("test.wrong", json!({ "amount": 1 })))
            .err()
            .unwrap();
        assert!(matches!(err, CommandError::KindMismatch { .. }));
    }

    #[test]
    fn infos_are_sorted_for_the_palette() {
        let registry = registry();
        let kinds: Vec<_> = registry.infos().map(|info| info.kind).collect();
        assert_eq!(kinds, ["test.add", "test.note", "test.set"]);
        assert_eq!(registry.info("test.add").unwrap().category, "Test");
        assert_eq!(registry.len(), 3);
    }

    #[test]
    fn downcast_only_matches_the_right_type() {
        let add = Add { amount: 1 };
        assert!(downcast::<Doc, Add>(&add).is_some());
        assert!(downcast::<Doc, Set>(&add).is_none());
    }
}
