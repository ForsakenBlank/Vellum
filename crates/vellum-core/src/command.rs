//! the command system.
//!
//! every user action (button, menu, shortcut, macro step, cli call, plugin) is a
//! [`Command`]. that one rule is what gives us undo/redo, the command palette,
//! macro recording and scripting without each feature building its own.
//!
//! a command is a plain serde struct holding its parameters. whatever it needs
//! to undo itself is captured during `apply` and kept in `#[serde(skip)]` fields,
//! so the serialised form stays a clean "what the user asked for".

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// id under which a [`Batch`] is recorded
pub const BATCH_ID: &str = "core.batch";

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    Failed(String),
    #[error("unknown command `{0}`")]
    Unknown(String),
    #[error("command `{0}` is already registered")]
    Duplicate(String),
    #[error("could not read the parameters of `{id}`: {source}")]
    Params {
        id: String,
        #[source]
        source: serde_json::Error,
    },
}

impl CommandError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    fn params(id: &str, source: serde_json::Error) -> Self {
        Self::Params {
            id: id.to_owned(),
            source,
        }
    }
}

pub type CommandResult = Result<(), CommandError>;

/// the answer a command gives when offered the command that ran right after it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merge {
    /// keep them as two undo steps
    No,
    /// the earlier command took over the later one's effect
    Yes,
    /// the two cancel out, the target is back where it was before the first
    Annul,
}

/// an undoable action on a target of type `T` (a document, a timeline, the app shell).
pub trait Command<T: 'static>: Serialize + DeserializeOwned + Send + fmt::Debug + 'static {
    /// stable id used by the registry, shortcuts and macros, such as `image.layer.rename`
    const ID: &'static str;

    /// label for menus and the command palette
    const TITLE: &'static str;

    /// label for the history panel. override it to include values ("Set opacity to 40%")
    fn name(&self) -> String {
        Self::TITLE.to_owned()
    }

    /// palette and shortcut metadata
    fn spec() -> CommandSpec {
        CommandSpec::new(Self::ID, Self::TITLE)
    }

    /// changes the target and remembers what `undo` will need.
    /// if this fails the target must be left exactly as it was
    fn apply(&mut self, target: &mut T) -> CommandResult;

    /// puts the target back to how it was before `apply`
    fn undo(&mut self, target: &mut T) -> CommandResult;

    /// runs again after an undo. only override when replaying from saved state is cheaper
    fn redo(&mut self, target: &mut T) -> CommandResult {
        self.apply(target)
    }

    /// called on the previous command after `next` has been applied, so a slider
    /// drag or a run of typing collapses into one undo step
    fn merge(&mut self, _next: &Self) -> Merge {
        Merge::No
    }

    /// false for actions with nothing to undo, like exporting or copying.
    /// they still run through the command system so they show up in macros
    fn is_undoable(&self) -> bool {
        true
    }

    /// rough heap bytes held for undo, used to keep the history within its memory budget
    fn cost(&self) -> usize {
        0
    }
}

/// the object safe form of [`Command`] that the history and the registry store.
pub trait DynCommand<T: 'static>: Any + Send + fmt::Debug {
    fn id(&self) -> &'static str;
    fn name(&self) -> String;
    fn apply(&mut self, target: &mut T) -> CommandResult;
    fn undo(&mut self, target: &mut T) -> CommandResult;
    fn redo(&mut self, target: &mut T) -> CommandResult;
    fn merge(&mut self, next: &dyn DynCommand<T>) -> Merge;
    fn is_undoable(&self) -> bool;
    fn cost(&self) -> usize;
    /// the serialised form a macro stores and the registry can rebuild
    fn record(&self) -> Result<CommandRecord, CommandError>;
}

pub type BoxedCommand<T> = Box<dyn DynCommand<T>>;

struct Erased<C>(C);

impl<C: fmt::Debug> fmt::Debug for Erased<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: 'static, C: Command<T>> DynCommand<T> for Erased<C> {
    fn id(&self) -> &'static str {
        C::ID
    }

    fn name(&self) -> String {
        self.0.name()
    }

    fn apply(&mut self, target: &mut T) -> CommandResult {
        self.0.apply(target)
    }

    fn undo(&mut self, target: &mut T) -> CommandResult {
        self.0.undo(target)
    }

    fn redo(&mut self, target: &mut T) -> CommandResult {
        self.0.redo(target)
    }

    fn merge(&mut self, next: &dyn DynCommand<T>) -> Merge {
        let next: &dyn Any = next;
        match next.downcast_ref::<Self>() {
            Some(next) => self.0.merge(&next.0),
            None => Merge::No,
        }
    }

    fn is_undoable(&self) -> bool {
        self.0.is_undoable()
    }

    fn cost(&self) -> usize {
        self.0.cost()
    }

    fn record(&self) -> Result<CommandRecord, CommandError> {
        let params = serde_json::to_value(&self.0).map_err(|e| CommandError::params(C::ID, e))?;
        Ok(CommandRecord {
            id: C::ID.to_owned(),
            params,
        })
    }
}

/// anything the history can run: a typed command, a batch, or an already boxed one.
pub trait IntoCommand<T: 'static> {
    fn into_command(self) -> BoxedCommand<T>;
}

impl<T: 'static, C: Command<T>> IntoCommand<T> for C {
    fn into_command(self) -> BoxedCommand<T> {
        Box::new(Erased(self))
    }
}

impl<T: 'static> IntoCommand<T> for BoxedCommand<T> {
    fn into_command(self) -> BoxedCommand<T> {
        self
    }
}

impl<T: 'static> IntoCommand<T> for Batch<T> {
    fn into_command(self) -> BoxedCommand<T> {
        Box::new(self)
    }
}

/// a command as stored in a macro, sent over ipc or typed into the cli.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommandRecord {
    pub id: String,
    #[serde(default)]
    pub params: Value,
}

impl CommandRecord {
    pub fn new(id: impl Into<String>, params: Value) -> Self {
        Self {
            id: id.into(),
            params,
        }
    }
}

/// what the command palette, menus and the shortcut editor know about a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommandSpec {
    pub id: String,
    pub title: String,
    /// groups commands in menus and the palette, such as "Layer"
    pub category: Option<String>,
    /// extra words the palette search should match
    pub keywords: Vec<String>,
    /// default shortcut in a platform neutral form, such as "Mod+Shift+Z"
    pub shortcut: Option<String>,
    /// whether the palette can run it as is, without asking for parameters
    pub palette: bool,
}

impl CommandSpec {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            category: None,
            keywords: Vec::new(),
            shortcut: None,
            palette: true,
        }
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn keywords<I, S>(mut self, keywords: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// for commands that only make sense with parameters supplied by a tool
    pub fn hidden_from_palette(mut self) -> Self {
        self.palette = false;
        self
    }
}

/// several commands that undo and redo as one step.
pub struct Batch<T: 'static> {
    name: String,
    commands: Vec<BoxedCommand<T>>,
}

impl<T: 'static> Batch<T> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            commands: Vec::new(),
        }
    }

    pub fn with(mut self, command: impl IntoCommand<T>) -> Self {
        self.push(command);
        self
    }

    pub fn push(&mut self, command: impl IntoCommand<T>) {
        self.commands.push(command.into_command());
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// runs `step` over the commands in order. if one fails, the ones that already
    /// ran are put back with `rollback` so the batch is all or nothing
    fn run(
        commands: &mut [&mut BoxedCommand<T>],
        target: &mut T,
        step: fn(&mut dyn DynCommand<T>, &mut T) -> CommandResult,
        rollback: fn(&mut dyn DynCommand<T>, &mut T) -> CommandResult,
    ) -> CommandResult {
        for index in 0..commands.len() {
            let Err(error) = step(commands[index].as_mut(), target) else {
                continue;
            };
            for done in commands[..index].iter_mut().rev() {
                if let Err(rollback_error) = rollback(done.as_mut(), target) {
                    tracing::error!(
                        command = done.id(),
                        error = %rollback_error,
                        "could not roll back part of a failed batch"
                    );
                }
            }
            return Err(error);
        }
        Ok(())
    }
}

impl<T: 'static> fmt::Debug for Batch<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Batch")
            .field("name", &self.name)
            .field("commands", &self.commands)
            .finish()
    }
}

impl<T: 'static> DynCommand<T> for Batch<T> {
    fn id(&self) -> &'static str {
        BATCH_ID
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn apply(&mut self, target: &mut T) -> CommandResult {
        let mut forward: Vec<_> = self.commands.iter_mut().collect();
        Self::run(&mut forward, target, |c, t| c.apply(t), |c, t| c.undo(t))
    }

    fn undo(&mut self, target: &mut T) -> CommandResult {
        let mut backward: Vec<_> = self.commands.iter_mut().rev().collect();
        Self::run(&mut backward, target, |c, t| c.undo(t), |c, t| c.redo(t))
    }

    fn redo(&mut self, target: &mut T) -> CommandResult {
        let mut forward: Vec<_> = self.commands.iter_mut().collect();
        Self::run(&mut forward, target, |c, t| c.redo(t), |c, t| c.undo(t))
    }

    fn merge(&mut self, _next: &dyn DynCommand<T>) -> Merge {
        Merge::No
    }

    fn is_undoable(&self) -> bool {
        self.commands.iter().all(|command| command.is_undoable())
    }

    fn cost(&self) -> usize {
        self.commands.iter().map(|command| command.cost()).sum()
    }

    fn record(&self) -> Result<CommandRecord, CommandError> {
        let commands = self
            .commands
            .iter()
            .map(|command| command.record())
            .collect::<Result<Vec<_>, _>>()?;
        let params = serde_json::to_value(BatchParams {
            name: self.name.clone(),
            commands,
        })
        .map_err(|e| CommandError::params(BATCH_ID, e))?;
        Ok(CommandRecord {
            id: BATCH_ID.to_owned(),
            params,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct BatchParams {
    name: String,
    commands: Vec<CommandRecord>,
}

type Builder<T> = fn(Value) -> Result<BoxedCommand<T>, CommandError>;

struct Entry<T: 'static> {
    spec: CommandSpec,
    build: Builder<T>,
}

/// every command that can act on a `T`, looked up by id.
///
/// this is how a [`CommandRecord`] from the ui, a macro or the cli turns back
/// into a runnable command, and where the palette gets its list from.
pub struct CommandRegistry<T: 'static> {
    entries: BTreeMap<&'static str, Entry<T>>,
}

impl<T: 'static> Default for CommandRegistry<T> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl<T: 'static> CommandRegistry<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<C: Command<T>>(&mut self) -> Result<(), CommandError> {
        if C::ID == BATCH_ID || self.entries.contains_key(C::ID) {
            return Err(CommandError::Duplicate(C::ID.to_owned()));
        }
        let mut spec = C::spec();
        spec.id = C::ID.to_owned();
        self.entries.insert(
            C::ID,
            Entry {
                spec,
                build: build::<T, C>,
            },
        );
        Ok(())
    }

    pub fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    pub fn spec(&self, id: &str) -> Option<&CommandSpec> {
        self.entries.get(id).map(|entry| &entry.spec)
    }

    /// specs in id order
    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.entries.values().map(|entry| &entry.spec)
    }

    pub fn build(&self, record: &CommandRecord) -> Result<BoxedCommand<T>, CommandError> {
        if record.id == BATCH_ID {
            return self.build_batch(record);
        }
        let entry = self
            .entries
            .get(record.id.as_str())
            .ok_or_else(|| CommandError::Unknown(record.id.clone()))?;
        (entry.build)(record.params.clone())
    }

    fn build_batch(&self, record: &CommandRecord) -> Result<BoxedCommand<T>, CommandError> {
        let params: BatchParams = serde_json::from_value(record.params.clone())
            .map_err(|e| CommandError::params(BATCH_ID, e))?;
        let mut batch = Batch::new(params.name);
        for step in &params.commands {
            batch.push(self.build(step)?);
        }
        Ok(Box::new(batch))
    }
}

fn build<T: 'static, C: Command<T>>(params: Value) -> Result<BoxedCommand<T>, CommandError> {
    let command: Result<C, _> = if params.is_null() {
        // no params means "use the defaults". structs want an empty map for that,
        // unit structs want the null itself
        serde_json::from_value(Value::Object(Default::default()))
            .or_else(|error| serde_json::from_value(Value::Null).map_err(|_| error))
    } else {
        serde_json::from_value(params)
    };
    let command = command.map_err(|e| CommandError::params(C::ID, e))?;
    Ok(Box::new(Erased(command)))
}

#[cfg(test)]
pub(crate) mod testing {
    //! a tiny document and a few commands shared by the command and history tests

    use super::*;

    #[derive(Debug, Default, Clone, PartialEq)]
    pub struct Doc {
        pub opacity: i32,
        pub text: String,
        pub exports: u32,
    }

    #[derive(Debug, Default, Clone, Serialize, Deserialize)]
    pub struct SetOpacity {
        pub value: i32,
        #[serde(skip)]
        pub before: Option<i32>,
    }

    impl SetOpacity {
        pub fn new(value: i32) -> Self {
            Self {
                value,
                before: None,
            }
        }
    }

    impl Command<Doc> for SetOpacity {
        const ID: &'static str = "test.set_opacity";
        const TITLE: &'static str = "Set opacity";

        fn name(&self) -> String {
            format!("Set opacity to {}%", self.value)
        }

        fn spec() -> CommandSpec {
            CommandSpec::new(Self::ID, Self::TITLE)
                .category("Layer")
                .keywords(["transparency", "alpha"])
                .hidden_from_palette()
        }

        fn apply(&mut self, doc: &mut Doc) -> CommandResult {
            if !(0..=100).contains(&self.value) {
                return Err(CommandError::failed("opacity must be between 0 and 100"));
            }
            self.before.get_or_insert(doc.opacity);
            doc.opacity = self.value;
            Ok(())
        }

        fn undo(&mut self, doc: &mut Doc) -> CommandResult {
            doc.opacity = self
                .before
                .ok_or_else(|| CommandError::failed("not applied yet"))?;
            Ok(())
        }

        fn merge(&mut self, next: &Self) -> Merge {
            self.value = next.value;
            if self.before == Some(self.value) {
                Merge::Annul
            } else {
                Merge::Yes
            }
        }
    }

    #[derive(Debug, Default, Clone, Serialize, Deserialize)]
    pub struct Append {
        pub text: String,
    }

    impl Append {
        pub fn new(text: &str) -> Self {
            Self {
                text: text.to_owned(),
            }
        }
    }

    impl Command<Doc> for Append {
        const ID: &'static str = "test.append";
        const TITLE: &'static str = "Type";

        fn apply(&mut self, doc: &mut Doc) -> CommandResult {
            doc.text.push_str(&self.text);
            Ok(())
        }

        fn undo(&mut self, doc: &mut Doc) -> CommandResult {
            let keep = doc.text.len() - self.text.len();
            doc.text.truncate(keep);
            Ok(())
        }

        fn cost(&self) -> usize {
            self.text.len()
        }
    }

    #[derive(Debug, Default, Clone, Serialize, Deserialize)]
    pub struct Export;

    impl Command<Doc> for Export {
        const ID: &'static str = "test.export";
        const TITLE: &'static str = "Export";

        fn apply(&mut self, doc: &mut Doc) -> CommandResult {
            doc.exports += 1;
            Ok(())
        }

        fn undo(&mut self, _doc: &mut Doc) -> CommandResult {
            Ok(())
        }

        fn is_undoable(&self) -> bool {
            false
        }
    }

    pub fn registry() -> CommandRegistry<Doc> {
        let mut registry = CommandRegistry::new();
        registry.register::<SetOpacity>().unwrap();
        registry.register::<Append>().unwrap();
        registry.register::<Export>().unwrap();
        registry
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::testing::*;
    use super::*;

    #[test]
    fn a_record_round_trips_through_the_registry() {
        let registry = registry();
        let record = SetOpacity::new(40).into_command().record().unwrap();
        assert_eq!(
            record,
            CommandRecord::new("test.set_opacity", json!({ "value": 40 }))
        );

        let mut doc = Doc {
            opacity: 100,
            ..Doc::default()
        };
        let mut command = registry.build(&record).unwrap();
        command.apply(&mut doc).unwrap();
        assert_eq!(doc.opacity, 40);
        assert_eq!(command.name(), "Set opacity to 40%");

        command.undo(&mut doc).unwrap();
        assert_eq!(doc.opacity, 100);
    }

    #[test]
    fn undo_state_is_not_part_of_the_record() {
        let mut doc = Doc::default();
        let mut command = SetOpacity::new(10).into_command();
        command.apply(&mut doc).unwrap();
        assert_eq!(command.record().unwrap().params, json!({ "value": 10 }));
    }

    #[test]
    fn records_without_params_build_with_defaults() {
        let registry = registry();
        let mut doc = Doc::default();

        let mut export = registry
            .build(&CommandRecord::new("test.export", Value::Null))
            .unwrap();
        export.apply(&mut doc).unwrap();
        assert_eq!(doc.exports, 1);

        let parsed: CommandRecord = serde_json::from_str(r#"{ "id": "test.append" }"#).unwrap();
        let error = registry.build(&parsed).unwrap_err();
        assert!(matches!(error, CommandError::Params { .. }), "{error}");
    }

    #[test]
    fn unknown_and_duplicate_ids_are_rejected() {
        let mut registry = registry();
        let unknown = registry
            .build(&CommandRecord::new("nope", Value::Null))
            .unwrap_err();
        assert!(matches!(unknown, CommandError::Unknown(id) if id == "nope"));
        assert!(matches!(
            registry.register::<Append>(),
            Err(CommandError::Duplicate(_))
        ));
    }

    #[test]
    fn bad_params_name_the_command() {
        let record = CommandRecord::new("test.set_opacity", json!({ "value": "loud" }));
        let error = registry().build(&record).unwrap_err();
        assert!(error.to_string().contains("test.set_opacity"), "{error}");
    }

    #[test]
    fn specs_come_back_in_id_order_with_their_metadata() {
        let registry = registry();
        let ids: Vec<_> = registry.specs().map(|spec| spec.id.as_str()).collect();
        assert_eq!(ids, ["test.append", "test.export", "test.set_opacity"]);

        let spec = registry.spec("test.set_opacity").unwrap();
        assert_eq!(spec.category.as_deref(), Some("Layer"));
        assert_eq!(spec.keywords, ["transparency", "alpha"]);
        assert!(!spec.palette);
        assert!(registry.spec("test.append").unwrap().palette);
    }

    #[test]
    fn merge_only_happens_between_the_same_command_type() {
        let mut first = SetOpacity::new(10).into_command();
        let mut doc = Doc::default();
        first.apply(&mut doc).unwrap();

        assert_eq!(
            first.merge(Append::new("a").into_command().as_ref()),
            Merge::No
        );
        assert_eq!(
            first.merge(SetOpacity::new(20).into_command().as_ref()),
            Merge::Yes
        );
        assert_eq!(
            first.merge(SetOpacity::new(0).into_command().as_ref()),
            Merge::Annul
        );
    }

    #[test]
    fn a_batch_applies_and_undoes_as_one() {
        let mut doc = Doc::default();
        let mut batch = Batch::new("Style")
            .with(SetOpacity::new(30))
            .with(Append::new("hi"));
        assert_eq!(batch.len(), 2);

        batch.apply(&mut doc).unwrap();
        assert_eq!((doc.opacity, doc.text.as_str()), (30, "hi"));
        batch.undo(&mut doc).unwrap();
        assert_eq!(doc, Doc::default());
        batch.redo(&mut doc).unwrap();
        assert_eq!((doc.opacity, doc.text.as_str()), (30, "hi"));
    }

    #[test]
    fn a_failing_batch_leaves_the_target_untouched() {
        let mut doc = Doc {
            text: "keep".into(),
            ..Doc::default()
        };
        let before = doc.clone();
        let mut batch = Batch::new("Broken")
            .with(Append::new(" me"))
            .with(SetOpacity::new(999));

        assert!(batch.apply(&mut doc).is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn a_batch_round_trips_through_the_registry() {
        let registry = registry();
        let inner = Batch::new("Inner").with(Append::new("b"));
        let batch = Batch::new("Outer").with(Append::new("a")).with(inner);
        let record = batch.record().unwrap();
        assert_eq!(record.id, BATCH_ID);

        let mut doc = Doc::default();
        let mut rebuilt = registry.build(&record).unwrap();
        rebuilt.apply(&mut doc).unwrap();
        assert_eq!(doc.text, "ab");
        assert_eq!(rebuilt.name(), "Outer");
        assert_eq!(rebuilt.record().unwrap(), record);
    }

    #[test]
    fn a_batch_with_a_one_way_step_is_not_undoable() {
        let batch = Batch::<Doc>::new("Export all")
            .with(Append::new("a"))
            .with(Export);
        assert!(!batch.is_undoable());
        assert!(
            Batch::<Doc>::new("Edit")
                .with(Append::new("a"))
                .is_undoable()
        );
    }
}
