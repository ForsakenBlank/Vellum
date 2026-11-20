//! the document a quick tool session lives in: the files it works on and the tool's options.
//!
//! options are plain json here because the core does not know what each tool needs. the
//! tool itself (`vellum-export`) reads them into its own typed settings, filling in defaults
//! for anything missing, so an old session restored after an update still opens.
//!
//! every change is a command, so changing an option or the file list can be undone, is
//! autosaved and comes back after a crash like any other document.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::command::{Command, CommandError, CommandInfo, CommandRegistry, downcast, params_of};
use crate::documents::{DocumentError, DocumentModel, RenameDocument, Titled};
use crate::settings::merge_patch;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuickToolDocument {
    pub title: String,
    pub inputs: Vec<PathBuf>,
    /// the tool's own settings, always a json object
    pub options: Value,
}

impl QuickToolDocument {
    pub fn new(title: impl Into<String>, inputs: Vec<PathBuf>) -> Self {
        Self {
            title: title.into(),
            inputs,
            options: Value::Object(Map::new()),
        }
    }
}

impl Titled for QuickToolDocument {
    fn title_mut(&mut self) -> &mut String {
        &mut self.title
    }
}

impl DocumentModel for QuickToolDocument {
    const TYPE: &'static str = "quick_tool";

    fn commands() -> CommandRegistry<Self> {
        let mut registry = CommandRegistry::new();
        let results = [
            registry.register::<RenameDocument>(CommandInfo {
                kind: RenameDocument::KIND,
                title: "Rename document",
                category: "Document",
                description: "Change the name shown on the tab.",
            }),
            registry.register::<SetOptions>(CommandInfo {
                kind: SetOptions::KIND,
                title: "Change tool options",
                category: "Quick tools",
                description: "Change a setting of the open quick tool.",
            }),
            registry.register::<AddInputs>(CommandInfo {
                kind: AddInputs::KIND,
                title: "Add files",
                category: "Quick tools",
                description: "Add files to the open quick tool.",
            }),
            registry.register::<RemoveInputs>(CommandInfo {
                kind: RemoveInputs::KIND,
                title: "Remove files",
                category: "Quick tools",
                description: "Take files out of the open quick tool.",
            }),
        ];
        for result in results {
            if let Err(err) = result {
                tracing::error!(%err, "quick tool command was registered twice");
            }
        }
        registry
    }

    fn to_state(&self) -> Result<Value, DocumentError> {
        Ok(serde_json::to_value(self)?)
    }

    fn from_state(state: Value) -> Result<Self, DocumentError> {
        let mut doc: Self = serde_json::from_value(state)?;
        if !doc.options.is_object() {
            doc.options = Value::Object(Map::new());
        }
        Ok(doc)
    }

    fn title(&self) -> Option<String> {
        Some(self.title.clone())
    }

    fn inputs(&self) -> Option<Vec<PathBuf>> {
        Some(self.inputs.clone())
    }
}

/// changes some options with a json merge patch, eg `{ "target": { "amount": 8 } }`.
/// dragging a slider sends one per step and they merge into one undo step.
#[derive(Debug, Serialize, Deserialize)]
pub struct SetOptions {
    pub patch: Value,
    #[serde(skip)]
    previous: Option<Value>,
}

impl SetOptions {
    pub const KIND: &'static str = "tool.set_options";

    pub fn new(patch: Value) -> Self {
        Self {
            patch,
            previous: None,
        }
    }

    /// the option names a patch touches, like "fps" or "target.amount"
    fn keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        collect_keys(&self.patch, "", &mut keys);
        keys.sort();
        keys
    }
}

fn collect_keys(value: &Value, prefix: &str, keys: &mut Vec<String>) {
    let Value::Object(map) = value else {
        keys.push(prefix.to_owned());
        return;
    };
    for (key, value) in map {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        collect_keys(value, &path, keys);
    }
}

impl Command<QuickToolDocument> for SetOptions {
    fn kind(&self) -> &'static str {
        Self::KIND
    }

    fn name(&self) -> String {
        match self.keys().as_slice() {
            [one] => format!("Change {}", one.rsplit('.').next().unwrap_or(one)),
            _ => "Change options".to_owned(),
        }
    }

    fn apply(&mut self, doc: &mut QuickToolDocument) -> Result<(), CommandError> {
        if !self.patch.is_object() {
            return Err(CommandError::failed("options must be a json object"));
        }
        let mut next = doc.options.clone();
        merge_patch(&mut next, &self.patch);
        self.previous = Some(std::mem::replace(&mut doc.options, next));
        Ok(())
    }

    fn revert(&mut self, doc: &mut QuickToolDocument) -> Result<(), CommandError> {
        doc.options = self
            .previous
            .take()
            .ok_or_else(|| CommandError::failed("nothing to undo"))?;
        Ok(())
    }

    /// folds a following change of the same options into this one, so a slider drag is one
    /// step. a change to other options stays its own step.
    fn merge(&mut self, next: &dyn Command<QuickToolDocument>) -> bool {
        match downcast::<QuickToolDocument, SetOptions>(next) {
            Some(next) if next.keys() == self.keys() => {
                merge_patch(&mut self.patch, &next.patch);
                true
            }
            _ => false,
        }
    }

    fn params(&self) -> Result<Value, CommandError> {
        params_of(self)
    }

    fn memory_cost(&self) -> usize {
        self.patch.to_string().len() + self.previous.as_ref().map_or(0, |p| p.to_string().len())
    }
}

/// adds files to the end of the list, skipping ones already in it
#[derive(Debug, Serialize, Deserialize)]
pub struct AddInputs {
    pub paths: Vec<PathBuf>,
    #[serde(skip)]
    added: usize,
}

impl AddInputs {
    pub const KIND: &'static str = "tool.add_inputs";
}

impl Command<QuickToolDocument> for AddInputs {
    fn kind(&self) -> &'static str {
        Self::KIND
    }

    fn name(&self) -> String {
        files_label("Add", self.paths.len())
    }

    fn apply(&mut self, doc: &mut QuickToolDocument) -> Result<(), CommandError> {
        let before = doc.inputs.len();
        for path in &self.paths {
            if !doc.inputs.contains(path) {
                doc.inputs.push(path.clone());
            }
        }
        self.added = doc.inputs.len() - before;
        if self.added == 0 {
            return Err(CommandError::failed("those files are already in the list"));
        }
        Ok(())
    }

    fn revert(&mut self, doc: &mut QuickToolDocument) -> Result<(), CommandError> {
        let keep = doc.inputs.len().saturating_sub(self.added);
        doc.inputs.truncate(keep);
        Ok(())
    }

    fn params(&self) -> Result<Value, CommandError> {
        params_of(self)
    }
}

/// takes files out of the list, remembering where they were so undo puts them back there
#[derive(Debug, Serialize, Deserialize)]
pub struct RemoveInputs {
    pub paths: Vec<PathBuf>,
    #[serde(skip)]
    removed: Vec<(usize, PathBuf)>,
}

impl RemoveInputs {
    pub const KIND: &'static str = "tool.remove_inputs";
}

impl Command<QuickToolDocument> for RemoveInputs {
    fn kind(&self) -> &'static str {
        Self::KIND
    }

    fn name(&self) -> String {
        files_label("Remove", self.paths.len())
    }

    fn apply(&mut self, doc: &mut QuickToolDocument) -> Result<(), CommandError> {
        let removed: Vec<(usize, PathBuf)> = doc
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, p)| self.paths.contains(p))
            .map(|(i, p)| (i, p.clone()))
            .collect();
        if removed.is_empty() {
            return Err(CommandError::failed("those files are not in the list"));
        }
        doc.inputs.retain(|p| !self.paths.contains(p));
        self.removed = removed;
        Ok(())
    }

    fn revert(&mut self, doc: &mut QuickToolDocument) -> Result<(), CommandError> {
        // ascending positions, so each insert lands where it was
        for (index, path) in std::mem::take(&mut self.removed) {
            let index = index.min(doc.inputs.len());
            doc.inputs.insert(index, path);
        }
        Ok(())
    }

    fn params(&self) -> Result<Value, CommandError> {
        params_of(self)
    }
}

fn files_label(verb: &str, count: usize) -> String {
    match count {
        1 => format!("{verb} a file"),
        n => format!("{verb} {n} files"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandRecord;
    use crate::history::{History, HistoryLimits};
    use serde_json::json;

    fn run(
        history: &mut History<QuickToolDocument>,
        doc: &mut QuickToolDocument,
        kind: &str,
        params: Value,
    ) {
        let registry = QuickToolDocument::commands();
        let command = registry
            .instantiate(&CommandRecord {
                kind: kind.into(),
                params,
            })
            .unwrap();
        history.execute(command, doc).unwrap();
    }

    #[test]
    fn options_change_merge_and_undo() {
        let mut doc = QuickToolDocument::new("Export", vec![]);
        let mut history = History::new(HistoryLimits::default());
        run(
            &mut history,
            &mut doc,
            SetOptions::KIND,
            json!({ "patch": { "fps": 20 } }),
        );
        // a slider drag on the same option merges into one step
        run(
            &mut history,
            &mut doc,
            SetOptions::KIND,
            json!({ "patch": { "fps": 12 } }),
        );
        assert_eq!(history.summary().undo_name.as_deref(), Some("Change fps"));
        run(
            &mut history,
            &mut doc,
            SetOptions::KIND,
            json!({ "patch": { "crop": { "x": 4 } } }),
        );
        assert_eq!(doc.options, json!({ "fps": 12, "crop": { "x": 4 } }));

        history.undo(&mut doc).unwrap();
        assert_eq!(doc.options, json!({ "fps": 12 }));
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.options, json!({}));
        assert!(!history.summary().can_undo);
    }

    #[test]
    fn files_are_added_and_removed_in_place() {
        let mut doc = QuickToolDocument::new("Convert", vec!["a.png".into(), "b.png".into()]);
        let mut history = History::new(HistoryLimits::default());
        run(
            &mut history,
            &mut doc,
            AddInputs::KIND,
            json!({ "paths": ["b.png", "c.png"] }),
        );
        assert_eq!(doc.inputs.len(), 3);
        run(
            &mut history,
            &mut doc,
            RemoveInputs::KIND,
            json!({ "paths": ["a.png", "c.png"] }),
        );
        assert_eq!(doc.inputs, vec![PathBuf::from("b.png")]);
        history.undo(&mut doc).unwrap();
        assert_eq!(
            doc.inputs,
            ["a.png", "b.png", "c.png"].map(PathBuf::from).to_vec()
        );
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.inputs, ["a.png", "b.png"].map(PathBuf::from).to_vec());
    }

    #[test]
    fn adding_nothing_new_is_refused() {
        let mut doc = QuickToolDocument::new("Convert", vec!["a.png".into()]);
        let mut history = History::new(HistoryLimits::default());
        let registry = QuickToolDocument::commands();
        let command = registry
            .instantiate(&CommandRecord {
                kind: AddInputs::KIND.into(),
                params: json!({ "paths": ["a.png"] }),
            })
            .unwrap();
        assert!(history.execute(command, &mut doc).is_err());
        assert_eq!(doc.inputs.len(), 1);
    }

    #[test]
    fn broken_options_are_reset_on_load() {
        let doc = QuickToolDocument::from_state(json!({ "title": "x", "options": 5 })).unwrap();
        assert_eq!(doc.options, json!({}));
    }
}
