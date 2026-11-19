//! the one command system every action goes through.
//!
//! buttons, menus, shortcuts, macros, the cli and plugins all build a [`Command`] and hand it to a
//! [`crate::history::History`]. that gives undo/redo, macro recording and palette search for free.

use std::any::Any;
use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    Failed(String),
    #[error("there is no command called `{0}`")]
    UnknownKind(String),
    #[error("could not read the `{kind}` command: {source}")]
    Decode {
        kind: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not record the `{kind}` command: {source}")]
    Encode {
        kind: String,
        #[source]
        source: serde_json::Error,
    },
}

impl CommandError {
    pub fn failed(msg: impl Into<String>) -> Self {
        Self::Failed(msg.into())
    }
}

/// lets `merge` downcast the incoming command to its concrete type.
pub trait AsAny: Any {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// a single undoable action on some target `T` (a document, a timeline, the settings...).
///
/// `apply` may be called again after `undo` (that is how redo works), so a command has to keep
/// whatever it needs to replay itself, usually the old value captured on the first `apply`.
pub trait Command<T>: AsAny + Send {
    /// stable id like `image.layer.rename`. used by the registry, macros and the palette.
    fn kind(&self) -> &'static str;

    /// short human readable name shown in the history panel, like "Rename layer".
    fn label(&self) -> String;

    fn apply(&mut self, target: &mut T) -> Result<(), CommandError>;

    fn undo(&mut self, target: &mut T) -> Result<(), CommandError>;

    /// fold `next` (already applied) into `self` so a slider drag becomes one history entry.
    /// return true if merged. the merged command must undo back to the state before `self`.
    fn merge(&mut self, _next: &dyn Command<T>) -> bool {
        false
    }

    /// serialisable form used by macros and the cli. usually `CommandRecord::of(self)`.
    fn record(&self) -> Result<CommandRecord, CommandError>;
}

/// a command frozen as data so it can be saved in a macro and replayed later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommandRecord {
    pub kind: String,
    pub args: serde_json::Value,
}

impl CommandRecord {
    pub fn of<T, C>(command: &C) -> Result<Self, CommandError>
    where
        C: Command<T> + Serialize,
    {
        let kind = command.kind();
        let args = serde_json::to_value(command).map_err(|source| CommandError::Encode {
            kind: kind.to_string(),
            source,
        })?;
        Ok(Self {
            kind: kind.to_string(),
            args,
        })
    }
}

/// what the palette, menus and shortcut editor need to know about a command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommandInfo {
    pub kind: String,
    pub title: String,
    pub category: String,
    /// default shortcut in the `Ctrl+Shift+K` style, `Ctrl` becomes `Cmd` on macos in the ui
    pub shortcut: Option<String>,
}

impl CommandInfo {
    pub fn new(kind: &str, title: &str, category: &str) -> Self {
        Self {
            kind: kind.to_string(),
            title: title.to_string(),
            category: category.to_string(),
            shortcut: None,
        }
    }

    pub fn shortcut(mut self, shortcut: &str) -> Self {
        self.shortcut = Some(shortcut.to_string());
        self
    }
}

type Decoder<T> =
    Box<dyn Fn(serde_json::Value) -> Result<Box<dyn Command<T>>, serde_json::Error> + Send + Sync>;

struct Entry<T> {
    info: CommandInfo,
    decode: Decoder<T>,
}

/// every command a target type supports, with enough info to rebuild one from a [`CommandRecord`].
pub struct CommandRegistry<T> {
    entries: BTreeMap<String, Entry<T>>,
}

impl<T: 'static> Default for CommandRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static> CommandRegistry<T> {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// registers `C` under `info.kind`. registering the same kind twice replaces the old entry.
    pub fn register<C>(&mut self, info: CommandInfo)
    where
        C: Command<T> + DeserializeOwned + 'static,
    {
        let decode: Decoder<T> = Box::new(|args| {
            let command: C = serde_json::from_value(args)?;
            Ok(Box::new(command))
        });
        self.entries
            .insert(info.kind.clone(), Entry { info, decode });
    }

    pub fn decode(&self, record: &CommandRecord) -> Result<Box<dyn Command<T>>, CommandError> {
        let entry = self
            .entries
            .get(&record.kind)
            .ok_or_else(|| CommandError::UnknownKind(record.kind.clone()))?;
        let command =
            (entry.decode)(record.args.clone()).map_err(|source| CommandError::Decode {
                kind: record.kind.clone(),
                source,
            })?;
        // a mismatch here means the registry was wired up wrong, not bad user data
        debug_assert_eq!(
            command.kind(),
            record.kind,
            "registered under the wrong kind"
        );
        Ok(command)
    }

    pub fn info(&self, kind: &str) -> Option<&CommandInfo> {
        self.entries.get(kind).map(|e| &e.info)
    }

    pub fn infos(&self) -> impl Iterator<Item = &CommandInfo> {
        self.entries.values().map(|e| &e.info)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// a recorded list of commands that can be replayed on another target.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Macro {
    pub name: String,
    pub steps: Vec<CommandRecord>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// tiny target used by the command and history tests
    #[derive(Debug, Default, PartialEq)]
    pub struct Doc {
        pub value: i32,
        pub name: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct SetValue {
        pub to: i32,
        #[serde(skip)]
        pub from: Option<i32>,
    }

    impl SetValue {
        pub fn boxed(to: i32) -> Box<dyn Command<Doc>> {
            Box::new(Self { to, from: None })
        }
    }

    impl Command<Doc> for SetValue {
        fn kind(&self) -> &'static str {
            "doc.set_value"
        }

        fn label(&self) -> String {
            format!("Set value to {}", self.to)
        }

        fn apply(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            // keep the first old value so redo after undo still restores correctly
            self.from.get_or_insert(doc.value);
            doc.value = self.to;
            Ok(())
        }

        fn undo(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            doc.value = self
                .from
                .ok_or_else(|| CommandError::failed("undo before apply"))?;
            Ok(())
        }

        fn merge(&mut self, next: &dyn Command<Doc>) -> bool {
            match next.as_any().downcast_ref::<SetValue>() {
                Some(next) => {
                    self.to = next.to;
                    true
                }
                None => false,
            }
        }

        fn record(&self) -> Result<CommandRecord, CommandError> {
            CommandRecord::of(self)
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Rename {
        pub to: String,
        #[serde(skip)]
        pub from: Option<String>,
    }

    impl Rename {
        pub fn boxed(to: &str) -> Box<dyn Command<Doc>> {
            Box::new(Self {
                to: to.to_string(),
                from: None,
            })
        }
    }

    impl Command<Doc> for Rename {
        fn kind(&self) -> &'static str {
            "doc.rename"
        }

        fn label(&self) -> String {
            "Rename".to_string()
        }

        fn apply(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            if self.to.is_empty() {
                return Err(CommandError::failed("name cannot be empty"));
            }
            let old = std::mem::replace(&mut doc.name, self.to.clone());
            self.from.get_or_insert(old);
            Ok(())
        }

        fn undo(&mut self, doc: &mut Doc) -> Result<(), CommandError> {
            doc.name = self.from.clone().unwrap_or_default();
            Ok(())
        }

        fn record(&self) -> Result<CommandRecord, CommandError> {
            CommandRecord::of(self)
        }
    }

    pub fn registry() -> CommandRegistry<Doc> {
        let mut reg = CommandRegistry::new();
        reg.register::<SetValue>(
            CommandInfo::new("doc.set_value", "Set value", "Test").shortcut("Ctrl+1"),
        );
        reg.register::<Rename>(CommandInfo::new("doc.rename", "Rename", "Test"));
        reg
    }

    #[test]
    fn record_round_trips_through_the_registry() {
        let reg = registry();
        let mut doc = Doc::default();
        let original = SetValue { to: 7, from: None };
        let record = original.record().unwrap();
        assert_eq!(record.kind, "doc.set_value");
        assert_eq!(record.args, serde_json::json!({ "to": 7 }));

        let mut rebuilt = reg.decode(&record).unwrap();
        rebuilt.apply(&mut doc).unwrap();
        assert_eq!(doc.value, 7);
    }

    #[test]
    fn decode_reports_unknown_and_bad_records() {
        let reg = registry();
        let unknown = CommandRecord {
            kind: "nope".into(),
            args: serde_json::Value::Null,
        };
        assert!(matches!(
            reg.decode(&unknown),
            Err(CommandError::UnknownKind(_))
        ));

        let bad = CommandRecord {
            kind: "doc.set_value".into(),
            args: serde_json::json!({ "to": "seven" }),
        };
        assert!(matches!(reg.decode(&bad), Err(CommandError::Decode { .. })));
    }

    #[test]
    fn registry_lists_info_for_the_palette() {
        let reg = registry();
        assert_eq!(reg.len(), 2);
        let info = reg.info("doc.set_value").unwrap();
        assert_eq!(info.shortcut.as_deref(), Some("Ctrl+1"));
        let kinds: Vec<_> = reg.infos().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["doc.rename", "doc.set_value"]);
    }

    #[test]
    fn merge_only_accepts_the_same_type() {
        let mut a = SetValue { to: 1, from: None };
        assert!(a.merge(&SetValue { to: 2, from: None }));
        assert_eq!(a.to, 2);
        assert!(!a.merge(&Rename {
            to: "x".into(),
            from: None
        }));
    }
}
