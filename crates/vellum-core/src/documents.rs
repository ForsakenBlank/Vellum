//! open documents, each with its own undo history.
//!
//! the [`DocumentManager`] is the one place that knows which documents are open, which
//! window shows them and whether they have unsaved changes. every document type implements
//! [`DocumentModel`] and gets wrapped together with a [`History`] of its own, so undo in one
//! tab never touches another.
//!
//! every change emits `history:changed` for the document and `documents:changed` with the
//! full list, so the ui can redraw tabs and the undo menu from what rust says.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::command::{
    Command, CommandError, CommandInfo, CommandRecord, CommandRegistry, downcast, params_of,
};
use crate::events::{Event, EventBus};
use crate::files::{FileKind, Workspace, display_name};
use crate::history::{
    History, HistoryChanged, HistoryError, HistoryLimits, HistorySummary, HistoryView, NodeId,
    SnapshotId,
};

/// the label of the first window, where documents go back to when another window closes
pub const MAIN_WINDOW: &str = "main";

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("that document is no longer open")]
    UnknownDocument(String),
    #[error("vellum does not know documents of type `{0}`")]
    UnknownType(String),
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error("the document could not be stored or restored: {0}")]
    State(String),
}

impl From<serde_json::Error> for DocumentError {
    fn from(err: serde_json::Error) -> Self {
        Self::State(err.to_string())
    }
}

/// a kind of document, like an image or a timeline.
pub trait DocumentModel: Send + Sized + 'static {
    /// stable name used in recovery files, eg `image`
    const TYPE: &'static str;

    /// the commands that can change this kind of document
    fn commands() -> CommandRegistry<Self>;

    /// the full state as json, for autosave and crash recovery. big documents will write
    /// binary sidecar files from their own phase on, this is enough until then.
    fn to_state(&self) -> Result<serde_json::Value, DocumentError>;

    fn from_state(state: serde_json::Value) -> Result<Self, DocumentError>;

    /// a title that overrides the file name in tabs, if the document has one
    fn title(&self) -> Option<String> {
        None
    }

    /// the files a quick tool session works on, when the document itself keeps that list
    /// (so adding and removing files can be undone). none uses the files it was opened with.
    fn inputs(&self) -> Option<Vec<PathBuf>> {
        None
    }
}

/// where a document came from and where it is shown
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMeta {
    pub path: Option<PathBuf>,
    pub kind: Option<FileKind>,
    pub workspace: Workspace,
    /// the quick tool this document is open in, if any
    pub tool: Option<String>,
    /// the files a quick tool works on
    pub inputs: Vec<PathBuf>,
    pub window: String,
}

impl DocumentMeta {
    /// a file opened in the workspace that fits its type
    pub fn for_file(path: PathBuf, kind: FileKind) -> Self {
        Self {
            path: Some(path),
            kind: Some(kind),
            workspace: kind.workspace(),
            tool: None,
            inputs: Vec::new(),
            window: MAIN_WINDOW.to_owned(),
        }
    }

    /// a quick tool session working on some files
    pub fn for_tool(tool: impl Into<String>, inputs: Vec<PathBuf>) -> Self {
        let kind = inputs.iter().find_map(|p| FileKind::of_path(p));
        Self {
            path: None,
            kind,
            workspace: Workspace::QuickTools,
            tool: Some(tool.into()),
            inputs,
            window: MAIN_WINDOW.to_owned(),
        }
    }
}

/// what the ui needs for a tab
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentInfo {
    pub id: String,
    pub title: String,
    pub path: Option<String>,
    pub kind: Option<FileKind>,
    pub workspace: Workspace,
    pub tool: Option<String>,
    pub inputs: Vec<String>,
    pub window: String,
    pub dirty: bool,
    pub doc_type: String,
}

/// a document's full state, for the placeholder workspaces to show
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentState {
    pub info: DocumentInfo,
    #[ts(type = "unknown")]
    pub state: serde_json::Value,
}

/// sent whenever a document opens, closes, moves, or its title or saved state changes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentsChanged {
    /// every open document in tab order
    pub documents: Vec<DocumentInfo>,
}

impl Event for DocumentsChanged {
    const NAME: &'static str = "documents:changed";
}

/// one open document as autosave sees it
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSnapshot {
    pub id: String,
    pub doc_type: String,
    pub title: String,
    pub meta: DocumentMeta,
    pub dirty: bool,
    pub state: serde_json::Value,
}

/// the object safe face of an open document of any type
trait OpenDocument: Send {
    fn doc_type(&self) -> &'static str;
    fn title(&self) -> Option<String>;
    fn inputs(&self) -> Option<Vec<PathBuf>>;
    fn execute(&mut self, record: &CommandRecord) -> Result<(), CommandError>;
    fn undo(&mut self) -> Result<bool, CommandError>;
    fn redo(&mut self) -> Result<bool, CommandError>;
    fn seal(&mut self);
    fn checkout(&mut self, node: NodeId) -> Result<(), HistoryError>;
    fn create_snapshot(&mut self, name: String) -> SnapshotId;
    fn summary(&self) -> HistorySummary;
    fn view(&self) -> HistoryView;
    fn mark_saved(&mut self);
    fn mark_unsaved(&mut self);
    fn is_dirty(&self) -> bool;
    fn set_limits(&mut self, limits: HistoryLimits);
    fn state(&self) -> Result<serde_json::Value, DocumentError>;
}

struct Open<D: DocumentModel> {
    model: D,
    history: History<D>,
    commands: Arc<CommandRegistry<D>>,
}

impl<D: DocumentModel> OpenDocument for Open<D> {
    fn doc_type(&self) -> &'static str {
        D::TYPE
    }
    fn title(&self) -> Option<String> {
        self.model.title()
    }
    fn inputs(&self) -> Option<Vec<PathBuf>> {
        self.model.inputs()
    }
    fn execute(&mut self, record: &CommandRecord) -> Result<(), CommandError> {
        let command = self.commands.instantiate(record)?;
        self.history.execute(command, &mut self.model)?;
        Ok(())
    }
    fn undo(&mut self) -> Result<bool, CommandError> {
        self.history.undo(&mut self.model)
    }
    fn redo(&mut self) -> Result<bool, CommandError> {
        self.history.redo(&mut self.model)
    }
    fn seal(&mut self) {
        self.history.seal();
    }
    fn checkout(&mut self, node: NodeId) -> Result<(), HistoryError> {
        self.history.checkout(node, &mut self.model)
    }
    fn create_snapshot(&mut self, name: String) -> SnapshotId {
        self.history.create_snapshot(name)
    }
    fn summary(&self) -> HistorySummary {
        self.history.summary()
    }
    fn view(&self) -> HistoryView {
        self.history.view()
    }
    fn mark_saved(&mut self) {
        self.history.mark_saved();
    }
    fn mark_unsaved(&mut self) {
        self.history.mark_unsaved();
    }
    fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }
    fn set_limits(&mut self, limits: HistoryLimits) {
        self.history.set_limits(limits);
    }
    fn state(&self) -> Result<serde_json::Value, DocumentError> {
        self.model.to_state()
    }
}

type Loader = Box<
    dyn Fn(serde_json::Value, HistoryLimits) -> Result<Box<dyn OpenDocument>, DocumentError>
        + Send
        + Sync,
>;

struct DocumentType {
    infos: Vec<CommandInfo>,
    load: Loader,
}

struct Entry {
    id: String,
    meta: DocumentMeta,
    doc: Box<dyn OpenDocument>,
}

impl Entry {
    fn info(&self) -> DocumentInfo {
        let title = self.doc.title().filter(|t| !t.trim().is_empty());
        let title = title.unwrap_or_else(|| match &self.meta.path {
            Some(path) => display_name(path),
            None => "Untitled".to_owned(),
        });
        let inputs = self
            .doc
            .inputs()
            .unwrap_or_else(|| self.meta.inputs.clone());
        let kind = match self.meta.tool {
            Some(_) => inputs.iter().find_map(|p| FileKind::of_path(p)),
            None => self.meta.kind,
        };
        DocumentInfo {
            id: self.id.clone(),
            title,
            path: self.meta.path.as_ref().map(|p| p.display().to_string()),
            kind,
            workspace: self.meta.workspace,
            tool: self.meta.tool.clone(),
            inputs: inputs.iter().map(|p| p.display().to_string()).collect(),
            window: self.meta.window.clone(),
            dirty: self.doc.is_dirty(),
            doc_type: self.doc.doc_type().to_owned(),
        }
    }
}

struct Inner {
    entries: Vec<Entry>,
    types: HashMap<&'static str, DocumentType>,
    next_id: u64,
    limits: HistoryLimits,
    /// goes up on every change, so autosave can tell whether anything happened
    revision: u64,
}

impl Inner {
    fn entry_mut(&mut self, id: &str) -> Result<&mut Entry, DocumentError> {
        self.entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| DocumentError::UnknownDocument(id.to_owned()))
    }

    fn entry(&self, id: &str) -> Result<&Entry, DocumentError> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| DocumentError::UnknownDocument(id.to_owned()))
    }
}

/// owns every open document. cheap to clone, every clone sees the same documents.
///
/// events are emitted while the lock is held. that keeps them in the same order as the
/// changes, and is safe because sinks and subscribers never call back into the manager.
#[derive(Clone)]
pub struct DocumentManager {
    inner: Arc<Mutex<Inner>>,
    bus: EventBus,
}

impl DocumentManager {
    /// a manager that knows the placeholder document type
    pub fn new(bus: EventBus, limits: HistoryLimits) -> Self {
        let manager = Self {
            inner: Arc::new(Mutex::new(Inner {
                entries: Vec::new(),
                types: HashMap::new(),
                next_id: 1,
                limits,
                revision: 0,
            })),
            bus,
        };
        manager.register_type::<PlaceholderDocument>();
        manager.register_type::<crate::quick_tool::QuickToolDocument>();
        manager
    }

    /// teaches the manager a document type, so recovery files of that type can be loaded
    pub fn register_type<D: DocumentModel>(&self) {
        let commands = Arc::new(D::commands());
        let infos = commands.infos().cloned().collect();
        let load: Loader = Box::new(move |state, limits| {
            let model = D::from_state(state)?;
            Ok(Box::new(Open {
                model,
                history: History::new(limits),
                commands: commands.clone(),
            }))
        });
        self.inner
            .lock()
            .types
            .insert(D::TYPE, DocumentType { infos, load });
    }

    /// the commands every registered document type understands
    pub fn command_infos(&self) -> Vec<CommandInfo> {
        let inner = self.inner.lock();
        let mut infos: Vec<CommandInfo> = inner
            .types
            .values()
            .flat_map(|t| t.infos.iter().cloned())
            .collect();
        infos.sort_by_key(|info| info.kind);
        // shared commands like rename belong to several types but are one entry
        infos.dedup_by_key(|info| info.kind);
        infos
    }

    pub fn open<D: DocumentModel>(
        &self,
        model: D,
        meta: DocumentMeta,
    ) -> Result<DocumentInfo, DocumentError> {
        let state = model.to_state()?;
        self.open_state(D::TYPE, state, meta, false)
    }

    /// opens a document from stored state. `dirty` marks it as having unsaved changes, for
    /// documents that come back after a crash.
    pub fn open_state(
        &self,
        doc_type: &str,
        state: serde_json::Value,
        meta: DocumentMeta,
        dirty: bool,
    ) -> Result<DocumentInfo, DocumentError> {
        let mut inner = self.inner.lock();
        let limits = inner.limits;
        let ty = inner
            .types
            .get(doc_type)
            .ok_or_else(|| DocumentError::UnknownType(doc_type.to_owned()))?;
        let mut doc = (ty.load)(state, limits)?;
        if dirty {
            doc.mark_unsaved();
        }
        let id = format!("doc-{}", inner.next_id);
        inner.next_id += 1;
        let entry = Entry { id, meta, doc };
        let info = entry.info();
        inner.entries.push(entry);
        self.changed(&mut inner, Some(&info.id));
        Ok(info)
    }

    pub fn close(&self, id: &str) -> Result<(), DocumentError> {
        let mut inner = self.inner.lock();
        let before = inner.entries.len();
        inner.entries.retain(|e| e.id != id);
        if inner.entries.len() == before {
            return Err(DocumentError::UnknownDocument(id.to_owned()));
        }
        self.changed(&mut inner, None);
        Ok(())
    }

    pub fn list(&self) -> Vec<DocumentInfo> {
        self.inner.lock().entries.iter().map(Entry::info).collect()
    }

    pub fn info(&self, id: &str) -> Result<DocumentInfo, DocumentError> {
        Ok(self.inner.lock().entry(id)?.info())
    }

    pub fn state(&self, id: &str) -> Result<DocumentState, DocumentError> {
        let inner = self.inner.lock();
        let entry = inner.entry(id)?;
        Ok(DocumentState {
            info: entry.info(),
            state: entry.doc.state()?,
        })
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.inner.lock().entry(id).is_ok()
    }

    /// runs a document command from its record, adding it to the document's history
    pub fn execute(&self, id: &str, record: &CommandRecord) -> Result<(), DocumentError> {
        self.with_doc(id, |doc| Ok(doc.execute(record)?))
    }

    /// returns false if there was nothing to undo
    pub fn undo(&self, id: &str) -> Result<bool, DocumentError> {
        self.with_doc(id, |doc| Ok(doc.undo()?))
    }

    /// returns false if there was nothing to redo
    pub fn redo(&self, id: &str) -> Result<bool, DocumentError> {
        self.with_doc(id, |doc| Ok(doc.redo()?))
    }

    /// ends a drag or a run of typing, so the next command starts a new undo step
    pub fn seal(&self, id: &str) -> Result<(), DocumentError> {
        let mut inner = self.inner.lock();
        inner.entry_mut(id)?.doc.seal();
        Ok(())
    }

    pub fn history(&self, id: &str) -> Result<HistoryView, DocumentError> {
        Ok(self.inner.lock().entry(id)?.doc.view())
    }

    pub fn summary(&self, id: &str) -> Result<HistorySummary, DocumentError> {
        Ok(self.inner.lock().entry(id)?.doc.summary())
    }

    /// jumps to any step on any branch of the history
    pub fn checkout(&self, id: &str, node: NodeId) -> Result<(), DocumentError> {
        self.with_doc(id, |doc| Ok(doc.checkout(node)?))
    }

    pub fn create_snapshot(&self, id: &str, name: String) -> Result<SnapshotId, DocumentError> {
        self.with_doc(id, |doc| Ok(doc.create_snapshot(name)))
    }

    /// records that the document matches what is on disk
    pub fn mark_saved(&self, id: &str) -> Result<(), DocumentError> {
        self.with_doc(id, |doc| {
            doc.mark_saved();
            Ok(())
        })
    }

    /// shows a document in another window, at `index` among that window's tabs
    pub fn move_to_window(
        &self,
        id: &str,
        window: &str,
        index: Option<usize>,
    ) -> Result<(), DocumentError> {
        let mut inner = self.inner.lock();
        let from = inner
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| DocumentError::UnknownDocument(id.to_owned()))?;
        let mut entry = inner.entries.remove(from);
        entry.meta.window = window.to_owned();
        // index counts only the tabs of the target window
        let at = index
            .and_then(|index| {
                inner
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.meta.window == window)
                    .nth(index)
                    .map(|(pos, _)| pos)
            })
            .unwrap_or(inner.entries.len());
        inner.entries.insert(at, entry);
        self.changed(&mut inner, None);
        Ok(())
    }

    /// moves every document of a closing window back to the main window
    pub fn window_closed(&self, window: &str) {
        let mut inner = self.inner.lock();
        let mut moved = false;
        for entry in inner.entries.iter_mut().filter(|e| e.meta.window == window) {
            entry.meta.window = MAIN_WINDOW.to_owned();
            moved = true;
        }
        if moved {
            self.changed(&mut inner, None);
        }
    }

    pub fn set_limits(&self, limits: HistoryLimits) {
        let mut inner = self.inner.lock();
        if inner.limits == limits {
            return;
        }
        inner.limits = limits;
        for entry in &mut inner.entries {
            entry.doc.set_limits(limits);
        }
    }

    pub fn limits(&self) -> HistoryLimits {
        self.inner.lock().limits
    }

    pub fn revision(&self) -> u64 {
        self.inner.lock().revision
    }

    /// every open document with its state, for autosave. documents whose state cannot be
    /// read are skipped and logged rather than failing the whole autosave.
    pub fn snapshot_all(&self) -> (u64, Vec<DocumentSnapshot>) {
        let inner = self.inner.lock();
        let mut list = Vec::new();
        for entry in &inner.entries {
            match entry.doc.state() {
                Ok(state) => list.push(DocumentSnapshot {
                    id: entry.id.clone(),
                    doc_type: entry.doc.doc_type().to_owned(),
                    title: entry.info().title,
                    meta: entry.meta.clone(),
                    dirty: entry.doc.is_dirty(),
                    state,
                }),
                Err(err) => tracing::warn!(id = entry.id, %err, "could not autosave a document"),
            }
        }
        (inner.revision, list)
    }

    fn with_doc<R>(
        &self,
        id: &str,
        f: impl FnOnce(&mut dyn OpenDocument) -> Result<R, DocumentError>,
    ) -> Result<R, DocumentError> {
        let mut inner = self.inner.lock();
        let entry = inner.entry_mut(id)?;
        let result = f(entry.doc.as_mut());
        // a failed command can still have moved part way through a checkout, so always report
        self.changed(&mut inner, Some(id));
        result
    }

    fn changed(&self, inner: &mut Inner, document: Option<&str>) {
        inner.revision += 1;
        if let Some(id) = document
            && let Ok(entry) = inner.entry(id)
        {
            self.bus.emit(HistoryChanged {
                document: id.to_owned(),
                summary: entry.doc.summary(),
            });
        }
        self.bus.emit(DocumentsChanged {
            documents: inner.entries.iter().map(Entry::info).collect(),
        });
    }
}

/// the stand-in document every workspace opens until its real editor arrives. it has a
/// title and free text notes so undo, redo, autosave and recovery can be tried for real.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaceholderDocument {
    pub title: String,
    pub notes: String,
}

impl PlaceholderDocument {
    pub fn titled(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            notes: String::new(),
        }
    }
}

impl DocumentModel for PlaceholderDocument {
    const TYPE: &'static str = "placeholder";

    fn commands() -> CommandRegistry<Self> {
        let mut registry = CommandRegistry::new();
        let results = [
            registry.register::<RenameDocument>(CommandInfo {
                kind: RenameDocument::KIND,
                title: "Rename document",
                category: "Document",
                description: "Change the name shown on the tab.",
            }),
            registry.register::<EditNotes>(CommandInfo {
                kind: EditNotes::KIND,
                title: "Edit notes",
                category: "Document",
                description: "Change the notes kept with the document.",
            }),
        ];
        for result in results {
            if let Err(err) = result {
                tracing::error!(%err, "placeholder command was registered twice");
            }
        }
        registry
    }

    fn to_state(&self) -> Result<serde_json::Value, DocumentError> {
        Ok(serde_json::to_value(self)?)
    }

    fn from_state(state: serde_json::Value) -> Result<Self, DocumentError> {
        Ok(serde_json::from_value(state)?)
    }

    fn title(&self) -> Option<String> {
        Some(self.title.clone())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RenameDocument {
    pub title: String,
    #[serde(skip)]
    previous: Option<String>,
}

impl RenameDocument {
    pub const KIND: &'static str = "document.rename";
}

/// a document with a name people can change, so every document type shares one rename
pub trait Titled {
    fn title_mut(&mut self) -> &mut String;
}

impl Titled for PlaceholderDocument {
    fn title_mut(&mut self) -> &mut String {
        &mut self.title
    }
}

impl<T: Titled + 'static> Command<T> for RenameDocument {
    fn kind(&self) -> &'static str {
        Self::KIND
    }
    fn name(&self) -> String {
        format!("Rename to \"{}\"", self.title)
    }
    fn apply(&mut self, doc: &mut T) -> Result<(), CommandError> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err(CommandError::failed("a document needs a name"));
        }
        self.previous = Some(std::mem::replace(doc.title_mut(), title.to_owned()));
        Ok(())
    }
    fn revert(&mut self, doc: &mut T) -> Result<(), CommandError> {
        let previous = self
            .previous
            .take()
            .ok_or_else(|| CommandError::failed("nothing to undo"))?;
        *doc.title_mut() = previous;
        Ok(())
    }
    fn params(&self) -> Result<serde_json::Value, CommandError> {
        params_of(self)
    }
}

/// replaces the notes. typing sends one of these per pause, and they merge into one step.
#[derive(Debug, Serialize, Deserialize)]
pub struct EditNotes {
    pub notes: String,
    #[serde(skip)]
    previous: Option<String>,
}

impl EditNotes {
    pub const KIND: &'static str = "document.edit_notes";
}

impl Command<PlaceholderDocument> for EditNotes {
    fn kind(&self) -> &'static str {
        Self::KIND
    }
    fn name(&self) -> String {
        "Edit notes".to_owned()
    }
    fn apply(&mut self, doc: &mut PlaceholderDocument) -> Result<(), CommandError> {
        self.previous = Some(std::mem::replace(&mut doc.notes, self.notes.clone()));
        Ok(())
    }
    fn revert(&mut self, doc: &mut PlaceholderDocument) -> Result<(), CommandError> {
        let previous = self
            .previous
            .take()
            .ok_or_else(|| CommandError::failed("nothing to undo"))?;
        doc.notes = previous;
        Ok(())
    }
    fn merge(&mut self, next: &dyn Command<PlaceholderDocument>) -> bool {
        match downcast::<PlaceholderDocument, EditNotes>(next) {
            Some(next) => {
                self.notes.clone_from(&next.notes);
                true
            }
            None => false,
        }
    }
    fn params(&self) -> Result<serde_json::Value, CommandError> {
        params_of(self)
    }
    fn memory_cost(&self) -> usize {
        self.notes.len() + self.previous.as_ref().map_or(0, String::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manager() -> (DocumentManager, EventBus) {
        let bus = EventBus::new();
        (
            DocumentManager::new(bus.clone(), HistoryLimits::default()),
            bus,
        )
    }

    fn open(manager: &DocumentManager, name: &str) -> DocumentInfo {
        let path = PathBuf::from(format!("/media/{name}"));
        let kind = FileKind::of_path(&path).unwrap();
        manager
            .open(
                PlaceholderDocument::default(),
                DocumentMeta::for_file(path, kind),
            )
            .unwrap()
    }

    fn rename(title: &str) -> CommandRecord {
        CommandRecord::new(RenameDocument::KIND, json!({ "title": title }))
    }

    fn notes(text: &str) -> CommandRecord {
        CommandRecord::new(EditNotes::KIND, json!({ "notes": text }))
    }

    #[test]
    fn opening_lists_documents_with_their_workspace() {
        let (manager, _) = manager();
        let clip = open(&manager, "clip.mp4");
        let photo = open(&manager, "photo.png");
        assert_eq!(clip.title, "clip.mp4");
        assert_eq!(clip.workspace, Workspace::Video);
        assert_eq!(photo.workspace, Workspace::Image);
        assert_ne!(clip.id, photo.id);
        assert!(!clip.dirty);
        let ids: Vec<_> = manager.list().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, [clip.id, photo.id]);
    }

    #[test]
    fn each_document_has_its_own_history() {
        let (manager, _) = manager();
        let a = open(&manager, "a.png").id;
        let b = open(&manager, "b.png").id;
        manager.execute(&a, &rename("First")).unwrap();
        manager.execute(&b, &rename("Second")).unwrap();

        assert!(manager.undo(&a).unwrap());
        assert_eq!(manager.info(&a).unwrap().title, "a.png");
        assert_eq!(manager.info(&b).unwrap().title, "Second");
        assert!(!manager.undo(&a).unwrap());

        assert!(manager.redo(&a).unwrap());
        assert_eq!(manager.info(&a).unwrap().title, "First");
    }

    #[test]
    fn changes_emit_history_and_document_events() {
        let (manager, bus) = manager();
        let history = bus.subscribe::<HistoryChanged>();
        let docs = bus.subscribe::<DocumentsChanged>();
        let id = open(&manager, "a.png").id;
        manager.execute(&id, &rename("Poster")).unwrap();

        let last = history.drain().pop().unwrap();
        assert_eq!(last.document, id);
        assert!(last.summary.can_undo);
        assert!(last.summary.dirty);
        assert_eq!(
            last.summary.undo_name.as_deref(),
            Some("Rename to \"Poster\"")
        );

        let last = docs.drain().pop().unwrap();
        assert_eq!(last.documents[0].title, "Poster");
        assert!(last.documents[0].dirty);

        manager.mark_saved(&id).unwrap();
        assert!(!docs.drain().pop().unwrap().documents[0].dirty);
    }

    #[test]
    fn typing_merges_into_one_step() {
        let (manager, _) = manager();
        let id = open(&manager, "a.png").id;
        for text in ["h", "he", "hello"] {
            manager.execute(&id, &notes(text)).unwrap();
        }
        assert_eq!(manager.history(&id).unwrap().entries.len(), 2);
        manager.seal(&id).unwrap();
        manager.execute(&id, &notes("hello world")).unwrap();
        assert_eq!(manager.history(&id).unwrap().entries.len(), 3);

        manager.undo(&id).unwrap();
        assert_eq!(manager.state(&id).unwrap().state["notes"], "hello");
        manager.undo(&id).unwrap();
        assert_eq!(manager.state(&id).unwrap().state["notes"], "");
    }

    #[test]
    fn failed_commands_change_nothing() {
        let (manager, _) = manager();
        let id = open(&manager, "a.png").id;
        assert!(manager.execute(&id, &rename("  ")).is_err());
        assert!(
            manager
                .execute(&id, &CommandRecord::new("nope", json!({})))
                .is_err()
        );
        assert!(!manager.summary(&id).unwrap().can_undo);
        assert!(matches!(
            manager.undo("doc-99"),
            Err(DocumentError::UnknownDocument(_))
        ));
    }

    #[test]
    fn checkout_moves_between_branches() {
        let (manager, _) = manager();
        let id = open(&manager, "a.png").id;
        manager.execute(&id, &rename("One")).unwrap();
        let one = manager.history(&id).unwrap().current;
        manager.undo(&id).unwrap();
        manager.execute(&id, &rename("Two")).unwrap();
        assert_eq!(manager.info(&id).unwrap().title, "Two");
        manager.checkout(&id, one).unwrap();
        assert_eq!(manager.info(&id).unwrap().title, "One");
        let snap = manager.create_snapshot(&id, "Good".into()).unwrap();
        assert_eq!(manager.history(&id).unwrap().snapshots[0].id, snap);
    }

    #[test]
    fn closing_removes_the_document() {
        let (manager, _) = manager();
        let id = open(&manager, "a.png").id;
        manager.close(&id).unwrap();
        assert!(manager.list().is_empty());
        assert!(!manager.is_open(&id));
        assert!(manager.close(&id).is_err());
    }

    #[test]
    fn documents_move_between_windows_and_come_back() {
        let (manager, _) = manager();
        let a = open(&manager, "a.png").id;
        let b = open(&manager, "b.png").id;
        let c = open(&manager, "c.png").id;
        manager.move_to_window(&b, "doc-1", None).unwrap();
        manager.move_to_window(&c, "doc-1", Some(0)).unwrap();
        let in_window = |w: &str| -> Vec<String> {
            manager
                .list()
                .into_iter()
                .filter(|d| d.window == w)
                .map(|d| d.id)
                .collect()
        };
        assert_eq!(in_window("doc-1"), [c.clone(), b.clone()]);
        assert_eq!(in_window(MAIN_WINDOW), std::slice::from_ref(&a));

        // reorder inside the main window
        manager.move_to_window(&c, MAIN_WINDOW, Some(0)).unwrap();
        assert_eq!(in_window(MAIN_WINDOW), [c.clone(), a.clone()]);

        manager.window_closed("doc-1");
        assert_eq!(in_window(MAIN_WINDOW), [c, a, b]);
    }

    #[test]
    fn restored_documents_start_dirty() {
        let (manager, _) = manager();
        let info = manager
            .open_state(
                PlaceholderDocument::TYPE,
                json!({ "title": "Recovered", "notes": "draft" }),
                DocumentMeta::for_tool("video-to-gif", vec![PathBuf::from("/a/clip.mp4")]),
                true,
            )
            .unwrap();
        assert!(info.dirty);
        assert_eq!(info.title, "Recovered");
        assert_eq!(info.tool.as_deref(), Some("video-to-gif"));
        assert_eq!(info.kind, Some(FileKind::Video));
        assert_eq!(info.workspace, Workspace::QuickTools);

        let unknown = manager.open_state(
            "hologram",
            json!({}),
            DocumentMeta::for_tool("x", vec![]),
            false,
        );
        assert!(matches!(unknown, Err(DocumentError::UnknownType(_))));
    }

    #[test]
    fn snapshots_include_dirty_state_and_revision_moves() {
        let (manager, _) = manager();
        let id = open(&manager, "a.png").id;
        let (rev, _) = manager.snapshot_all();
        manager.execute(&id, &notes("x")).unwrap();
        let (next, docs) = manager.snapshot_all();
        assert!(next > rev);
        assert!(docs[0].dirty);
        assert_eq!(docs[0].state["notes"], "x");
        assert_eq!(docs[0].doc_type, "placeholder");
    }

    #[test]
    fn limits_reach_open_documents() {
        let (manager, _) = manager();
        let id = open(&manager, "a.png").id;
        manager.set_limits(HistoryLimits {
            max_steps: 2,
            ..HistoryLimits::default()
        });
        for n in 0..5 {
            manager.seal(&id).unwrap();
            manager.execute(&id, &notes(&n.to_string())).unwrap();
        }
        // root plus at most two steps
        assert!(manager.history(&id).unwrap().entries.len() <= 3);
    }

    #[test]
    fn command_infos_list_document_commands() {
        let (manager, _) = manager();
        let kinds: Vec<_> = manager.command_infos().iter().map(|i| i.kind).collect();
        // rename belongs to both document types but is listed once
        assert_eq!(
            kinds,
            [
                EditNotes::KIND,
                RenameDocument::KIND,
                "tool.add_inputs",
                "tool.remove_inputs",
                "tool.rename_files",
                "tool.set_options"
            ]
        );
    }
}
