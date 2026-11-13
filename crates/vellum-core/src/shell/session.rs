//! the open documents, the windows they sit in and what each window shows.
//!
//! this is the state behind the tab bar, the workspace rail and tear-off
//! windows. it lives in rust rather than in the ui because every window has
//! its own webview: a document moved to another window has to be owned by
//! something both windows can see. every change goes through a [`Command`],
//! so tabs opened and closed show up in macros like any other action.

use std::collections::BTreeSet;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::files::{MediaKind, OpenTarget, PlannedOpen, plan_open};
use crate::command::{
    Command, CommandError, CommandRecord, CommandRegistry, CommandResult, CommandSpec,
};
use crate::event::{Event, EventBus};
use crate::time::unix_ms;

/// the window that exists for the whole life of the app
pub const MAIN_WINDOW: &str = "main";

/// recently closed documents kept for "reopen closed tab"
const KEEP_CLOSED: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DocumentId(u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenDocument {
    pub id: DocumentId,
    pub title: String,
    pub path: Option<String>,
    pub kind: MediaKind,
    pub workspace: String,
    /// set when the document is open in a quick tool
    pub tool: Option<String>,
    /// label of the window whose tab bar holds it
    pub window: String,
    pub opened_ms: u64,
    /// counts up each time the document is brought to the front
    #[serde(default)]
    pub focused: u64,
}

/// what a window is showing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum View {
    Home,
    /// a workspace with no document of its own open, which shows its empty state
    Workspace {
        id: String,
    },
    Document {
        id: DocumentId,
    },
}

/// a position on screen in logical pixels
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WindowState {
    pub label: String,
    pub view: View,
    /// where a torn off window should appear when it is created
    pub position: Option<Point>,
}

impl WindowState {
    fn new(label: impl Into<String>, position: Option<Point>) -> Self {
        Self {
            label: label.into(),
            view: View::Home,
            position,
        }
    }
}

/// what the ui gets: everything it needs to draw tabs and pick a page
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionView {
    /// in tab order. a window's tabs are the documents with its label
    pub documents: Vec<OpenDocument>,
    pub windows: Vec<WindowState>,
    /// how many closed documents "reopen closed tab" can bring back
    pub closed_count: u32,
    pub revision: u64,
}

/// published after any change to the session
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionChanged {
    pub session: SessionView,
}

impl Event for SessionChanged {
    const NAME: &'static str = "session.changed";
}

/// the part of a session that autosave writes and crash recovery reads back
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub saved_ms: u64,
    pub documents: Vec<OpenDocument>,
    pub windows: Vec<WindowState>,
}

#[derive(Debug, Clone)]
pub struct Session {
    documents: Vec<OpenDocument>,
    windows: Vec<WindowState>,
    closed: Vec<OpenDocument>,
    next_document: u64,
    next_window: u64,
    ticks: u64,
    revision: u64,
    /// files opened since the host last asked, so it can add them to the recent list
    opened: Vec<PlannedOpen>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

fn gone() -> CommandError {
    CommandError::failed("That file is no longer open.")
}

impl Session {
    pub fn new() -> Self {
        Self {
            documents: Vec::new(),
            windows: vec![WindowState::new(MAIN_WINDOW, None)],
            closed: Vec::new(),
            next_document: 1,
            next_window: 1,
            ticks: 0,
            revision: 0,
            opened: Vec::new(),
        }
    }

    pub fn documents(&self) -> &[OpenDocument] {
        &self.documents
    }

    pub fn document(&self, id: DocumentId) -> Option<&OpenDocument> {
        self.documents.iter().find(|doc| doc.id == id)
    }

    pub fn windows(&self) -> &[WindowState] {
        &self.windows
    }

    pub fn window(&self, label: &str) -> Option<&WindowState> {
        self.windows.iter().find(|window| window.label == label)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn view(&self) -> SessionView {
        SessionView {
            documents: self.documents.clone(),
            windows: self.windows.clone(),
            closed_count: self.closed.len() as u32,
            revision: self.revision,
        }
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            saved_ms: unix_ms(),
            documents: self.documents.clone(),
            windows: self.windows.clone(),
        }
    }

    /// files opened since the last call
    pub fn take_opened(&mut self) -> Vec<PlannedOpen> {
        std::mem::take(&mut self.opened)
    }

    fn changed(&mut self) {
        self.revision += 1;
    }

    fn window_mut(&mut self, label: &str) -> Result<&mut WindowState, CommandError> {
        self.windows
            .iter_mut()
            .find(|window| window.label == label)
            .ok_or_else(|| CommandError::failed("That window has already been closed."))
    }

    fn index_of(&self, id: DocumentId) -> Result<usize, CommandError> {
        self.documents
            .iter()
            .position(|doc| doc.id == id)
            .ok_or_else(gone)
    }

    fn tabs_of<'a>(&'a self, window: &'a str) -> impl Iterator<Item = &'a OpenDocument> + 'a {
        self.documents
            .iter()
            .filter(move |doc| doc.window == window)
    }

    fn new_window_label(&mut self) -> String {
        let label = format!("window-{}", self.next_window);
        self.next_window += 1;
        label
    }

    /// brings a document to the front of its window
    pub fn focus(&mut self, id: DocumentId) -> CommandResult {
        let index = self.index_of(id)?;
        self.ticks += 1;
        let ticks = self.ticks;
        let doc = &mut self.documents[index];
        doc.focused = ticks;
        let window = doc.window.clone();
        self.window_mut(&window)?.view = View::Document { id };
        self.changed();
        Ok(())
    }

    /// opens each file the plan routed, or brings it forward if it is already open
    /// in the same workspace and tool
    pub fn open(&mut self, opens: &[PlannedOpen], window: &str) -> Result<(), CommandError> {
        self.window_mut(window)?;
        let mut last = None;
        for open in opens {
            let existing = self.documents.iter().find(|doc| {
                doc.path.as_deref() == Some(open.path.as_str())
                    && doc.workspace == open.workspace
                    && doc.tool == open.tool
            });
            let id = match existing {
                Some(doc) => doc.id,
                None => {
                    let id = DocumentId(self.next_document);
                    self.next_document += 1;
                    self.documents.push(OpenDocument {
                        id,
                        title: open.name.clone(),
                        path: Some(open.path.clone()),
                        kind: open.kind,
                        workspace: open.workspace.clone(),
                        tool: open.tool.clone(),
                        window: window.to_owned(),
                        opened_ms: unix_ms(),
                        focused: 0,
                    });
                    id
                }
            };
            self.opened.push(open.clone());
            last = Some(id);
        }
        if let Some(id) = last {
            self.focus(id)?;
        }
        Ok(())
    }

    pub fn close(&mut self, id: DocumentId) -> CommandResult {
        let index = self.index_of(id)?;
        let window = self.documents[index].window.clone();
        // the neighbour to the right takes over, or the one to the left at the end
        let tabs: Vec<DocumentId> = self.tabs_of(&window).map(|doc| doc.id).collect();
        let position = tabs.iter().position(|tab| *tab == id).unwrap_or(0);
        let next = tabs
            .get(position + 1)
            .or_else(|| position.checked_sub(1).and_then(|before| tabs.get(before)))
            .copied();

        let doc = self.documents.remove(index);
        self.closed.push(doc);
        if self.closed.len() > KEEP_CLOSED {
            self.closed.remove(0);
        }
        self.after_leaving(&window, id, next)?;
        self.changed();
        Ok(())
    }

    /// fixes up a window a document just left: shows the next tab if the one that
    /// left was showing, and drops a torn off window that is now empty
    fn after_leaving(
        &mut self,
        window: &str,
        left: DocumentId,
        next: Option<DocumentId>,
    ) -> CommandResult {
        if window != MAIN_WINDOW && self.tabs_of(window).next().is_none() {
            self.windows.retain(|state| state.label != window);
            return Ok(());
        }
        let showing_it = self
            .window(window)
            .is_some_and(|state| state.view == View::Document { id: left });
        if showing_it {
            match next {
                Some(next) => self.focus(next)?,
                None => self.window_mut(window)?.view = View::Home,
            }
        }
        Ok(())
    }

    pub fn reopen_closed(&mut self, window: &str) -> CommandResult {
        let mut doc = self
            .closed
            .pop()
            .ok_or_else(|| CommandError::failed("There is no closed file to reopen."))?;
        doc.window = if self.window(window).is_some() {
            window.to_owned()
        } else {
            MAIN_WINDOW.to_owned()
        };
        let id = doc.id;
        self.documents.push(doc);
        self.focus(id)
    }

    pub fn show_home(&mut self, window: &str) -> CommandResult {
        self.window_mut(window)?.view = View::Home;
        self.changed();
        Ok(())
    }

    /// shows the document of that workspace the user had in front most recently,
    /// or the workspace's empty state when this window has none
    pub fn show_workspace(&mut self, window: &str, workspace: &str) -> CommandResult {
        let recent = self
            .tabs_of(window)
            .filter(|doc| doc.workspace == workspace)
            .max_by_key(|doc| doc.focused)
            .map(|doc| doc.id);
        match recent {
            Some(id) => self.focus(id),
            None => {
                self.window_mut(window)?.view = View::Workspace {
                    id: workspace.to_owned(),
                };
                self.changed();
                Ok(())
            }
        }
    }

    /// steps through a window's tabs, wrapping at either end
    pub fn cycle(&mut self, window: &str, forward: bool) -> CommandResult {
        let tabs: Vec<DocumentId> = self.tabs_of(window).map(|doc| doc.id).collect();
        if tabs.is_empty() {
            return Ok(());
        }
        let current = match self.window(window).map(|state| &state.view) {
            Some(View::Document { id }) => tabs.iter().position(|tab| tab == id),
            _ => None,
        };
        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => tabs.len() - 1,
            (Some(at), true) => (at + 1) % tabs.len(),
            (Some(at), false) => (at + tabs.len() - 1) % tabs.len(),
        };
        self.focus(tabs[next])
    }

    /// moves a tab to `index` among the tabs of its own window
    pub fn move_tab(&mut self, id: DocumentId, index: usize) -> CommandResult {
        let from = self.index_of(id)?;
        let doc = self.documents.remove(from);
        let siblings: Vec<usize> = self
            .documents
            .iter()
            .enumerate()
            .filter(|(_, other)| other.window == doc.window)
            .map(|(at, _)| at)
            .collect();
        let insert_at = match siblings.get(index) {
            Some(at) => *at,
            None => siblings.last().map_or(self.documents.len(), |at| at + 1),
        };
        self.documents.insert(insert_at, doc);
        self.changed();
        Ok(())
    }

    /// moves a document into another window, or into a new one when `target`
    /// is none. returns the label of the window it ended up in
    pub fn move_to_window(
        &mut self,
        id: DocumentId,
        target: Option<&str>,
        position: Option<Point>,
    ) -> Result<String, CommandError> {
        let index = self.index_of(id)?;
        let source = self.documents[index].window.clone();
        let target = match target {
            Some(label) => {
                self.window_mut(label)?;
                label.to_owned()
            }
            None => {
                let label = self.new_window_label();
                self.windows.push(WindowState::new(label.clone(), position));
                label
            }
        };
        if target == source {
            return Ok(target);
        }

        let tabs: Vec<DocumentId> = self.tabs_of(&source).map(|doc| doc.id).collect();
        let position_in_source = tabs.iter().position(|tab| *tab == id).unwrap_or(0);
        let next = tabs
            .get(position_in_source + 1)
            .or_else(|| {
                position_in_source
                    .checked_sub(1)
                    .and_then(|before| tabs.get(before))
            })
            .copied();

        // it goes to the end of the target's tabs
        let mut doc = self.documents.remove(index);
        doc.window = target.clone();
        self.documents.push(doc);
        self.focus(id)?;
        self.after_leaving(&source, id, next)?;
        self.changed();
        Ok(target)
    }

    /// a torn off window was closed by the user. its documents go back to the
    /// main window rather than being closed with it
    pub fn window_closed(&mut self, label: &str) {
        if label == MAIN_WINDOW || self.window(label).is_none() {
            return;
        }
        // keep the tab order: the returning documents go after main's own
        let (returning, mut kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.documents)
            .into_iter()
            .partition(|doc| doc.window == label);
        kept.extend(returning.into_iter().map(|mut doc| {
            doc.window = MAIN_WINDOW.to_owned();
            doc
        }));
        self.documents = kept;
        self.windows.retain(|window| window.label != label);
        self.changed();
    }

    /// brings back documents and windows from an autosave. the documents get
    /// new ids so they can never clash with ones opened since. returns how many
    /// documents came back
    pub fn restore(&mut self, snapshot: &SessionSnapshot) -> usize {
        let mut labels = std::collections::HashMap::new();
        labels.insert(MAIN_WINDOW.to_owned(), MAIN_WINDOW.to_owned());
        for window in &snapshot.windows {
            if window.label != MAIN_WINDOW && !labels.contains_key(&window.label) {
                let label = self.new_window_label();
                labels.insert(window.label.clone(), label.clone());
                self.windows.push(WindowState::new(label, window.position));
            }
        }

        let mut ids = std::collections::HashMap::new();
        for doc in &snapshot.documents {
            let id = DocumentId(self.next_document);
            self.next_document += 1;
            ids.insert(doc.id, id);
            let window = labels
                .get(&doc.window)
                .cloned()
                .unwrap_or_else(|| MAIN_WINDOW.to_owned());
            self.documents.push(OpenDocument {
                id,
                window,
                ..doc.clone()
            });
        }

        // each window shows what it showed before, as far as that still makes sense
        for old in &snapshot.windows {
            let Some(label) = labels.get(&old.label) else {
                continue;
            };
            let view = match &old.view {
                View::Document { id } => match ids.get(id) {
                    Some(id) => View::Document { id: *id },
                    None => continue,
                },
                other => other.clone(),
            };
            if let Some(window) = self.windows.iter_mut().find(|w| &w.label == label) {
                window.view = view;
            }
        }
        // a window that came back without any document is no use to anyone
        let in_use: BTreeSet<&str> = self.documents.iter().map(|d| d.window.as_str()).collect();
        self.windows
            .retain(|window| window.label == MAIN_WINDOW || in_use.contains(window.label.as_str()));

        self.changed();
        snapshot.documents.len()
    }
}

/// opens files in a window. the plan is worked out again here so a macro
/// replayed on other files routes them by their own types
#[derive(Debug, Serialize, Deserialize)]
pub struct OpenFiles {
    pub paths: Vec<String>,
    #[serde(default)]
    pub target: OpenTarget,
    pub window: String,
}

impl Command<Session> for OpenFiles {
    const ID: &'static str = "shell.open";
    const TITLE: &'static str = "Open files";

    fn spec() -> CommandSpec {
        CommandSpec::new(Self::ID, Self::TITLE)
            .category("File")
            .hidden_from_palette()
    }

    fn name(&self) -> String {
        match self.paths.as_slice() {
            [one] => format!("Open {}", super::files::file_name(one)),
            many => format!("Open {} files", many.len()),
        }
    }

    fn apply(&mut self, session: &mut Session) -> CommandResult {
        let plan = plan_open(&self.paths, &self.target);
        if plan.opens.is_empty() {
            let reason = plan
                .skipped
                .first()
                .map(|skipped| format!("{} could not be opened. {}", skipped.name, skipped.reason))
                .unwrap_or_else(|| "There was nothing to open.".to_owned());
            return Err(CommandError::failed(reason));
        }
        session.open(&plan.opens, &self.window)
    }

    fn undo(&mut self, _session: &mut Session) -> CommandResult {
        Ok(())
    }

    fn is_undoable(&self) -> bool {
        false
    }
}

/// writes the boilerplate for the shell commands. none of them are undoable:
/// ctrl+z belongs to the document, and closing a tab has its own way back
macro_rules! shell_command {
    (
        $name:ident { $($field:ident: $ty:ty),* $(,)? },
        $id:literal, $title:literal, $spec:expr,
        |$this:ident, $session:ident| $body:expr
    ) => {
        #[derive(Debug, Serialize, Deserialize)]
        pub struct $name {
            $(pub $field: $ty),*
        }

        impl Command<Session> for $name {
            const ID: &'static str = $id;
            const TITLE: &'static str = $title;

            fn spec() -> CommandSpec {
                let make: fn(CommandSpec) -> CommandSpec = $spec;
                make(CommandSpec::new(Self::ID, Self::TITLE))
            }

            fn apply(&mut self, $session: &mut Session) -> CommandResult {
                let $this = &*self;
                $body
            }

            fn undo(&mut self, _session: &mut Session) -> CommandResult {
                Ok(())
            }

            fn is_undoable(&self) -> bool {
                false
            }
        }
    };
}

shell_command!(
    CloseDocument { id: DocumentId },
    "shell.close",
    "Close tab",
    |spec| spec
        .category("File")
        .keywords(["close file"])
        .shortcut("Mod+W"),
    |this, session| session.close(this.id)
);

shell_command!(
    ReopenClosed { window: String },
    "shell.reopen_closed",
    "Reopen closed tab",
    |spec| spec
        .category("File")
        .keywords(["undo close", "restore tab"])
        .shortcut("Mod+Shift+T"),
    |this, session| session.reopen_closed(&this.window)
);

shell_command!(
    ActivateDocument { id: DocumentId },
    "shell.activate",
    "Switch to tab",
    |spec| spec.category("File").hidden_from_palette(),
    |this, session| session.focus(this.id)
);

shell_command!(
    ShowHome { window: String },
    "shell.home",
    "Go to home",
    |spec| spec
        .category("Go")
        .keywords(["start", "recent files"])
        .shortcut("Mod+Shift+H"),
    |this, session| session.show_home(&this.window)
);

shell_command!(
    ShowWorkspace {
        window: String,
        workspace: String
    },
    "shell.workspace",
    "Go to workspace",
    |spec| spec.category("Go").hidden_from_palette(),
    |this, session| session.show_workspace(&this.window, &this.workspace)
);

shell_command!(
    NextTab { window: String },
    "shell.next_tab",
    "Next tab",
    |spec| spec.category("Go").shortcut("Mod+Tab"),
    |this, session| session.cycle(&this.window, true)
);

shell_command!(
    PreviousTab { window: String },
    "shell.previous_tab",
    "Previous tab",
    |spec| spec.category("Go").shortcut("Mod+Shift+Tab"),
    |this, session| session.cycle(&this.window, false)
);

shell_command!(
    MoveTab {
        id: DocumentId,
        index: usize
    },
    "shell.move_tab",
    "Move tab",
    |spec| spec.category("File").hidden_from_palette(),
    |this, session| session.move_tab(this.id, this.index)
);

#[derive(Debug, Serialize, Deserialize)]
pub struct MoveToWindow {
    pub id: DocumentId,
    /// none opens a new window
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub position: Option<Point>,
}

impl Command<Session> for MoveToWindow {
    const ID: &'static str = "shell.move_to_window";
    const TITLE: &'static str = "Move tab to new window";

    fn spec() -> CommandSpec {
        CommandSpec::new(Self::ID, Self::TITLE)
            .category("Window")
            .keywords(["tear off", "detach", "pop out"])
    }

    fn apply(&mut self, session: &mut Session) -> CommandResult {
        session
            .move_to_window(self.id, self.window.as_deref(), self.position)
            .map(|_| ())
    }

    fn undo(&mut self, _session: &mut Session) -> CommandResult {
        Ok(())
    }

    fn is_undoable(&self) -> bool {
        false
    }
}

pub fn shell_registry() -> CommandRegistry<Session> {
    let mut registry = CommandRegistry::new();
    let registered = [
        registry.register::<OpenFiles>(),
        registry.register::<CloseDocument>(),
        registry.register::<ReopenClosed>(),
        registry.register::<ActivateDocument>(),
        registry.register::<ShowHome>(),
        registry.register::<ShowWorkspace>(),
        registry.register::<NextTab>(),
        registry.register::<PreviousTab>(),
        registry.register::<MoveTab>(),
        registry.register::<MoveToWindow>(),
    ];
    for result in registered {
        if let Err(error) = result {
            // only possible if two commands above share an id, which a test rules out
            tracing::error!(%error, "a shell command was registered twice");
        }
    }
    registry
}

/// what changed in the session after a command, for the host to act on
#[derive(Debug, Clone, PartialEq)]
pub struct ShellUpdate {
    pub view: SessionView,
    /// files that were opened, for the recent list
    pub opened: Vec<PlannedOpen>,
    /// windows the host has to create
    pub windows_added: Vec<WindowState>,
    /// windows the host has to close
    pub windows_removed: Vec<String>,
}

/// the session behind a lock, with its commands, telling the ui about changes
pub struct Shell {
    session: Mutex<Session>,
    registry: CommandRegistry<Session>,
    bus: Option<EventBus>,
}

impl Shell {
    pub fn new(bus: Option<EventBus>) -> Self {
        Self {
            session: Mutex::new(Session::new()),
            registry: shell_registry(),
            bus,
        }
    }

    pub fn specs(&self) -> Vec<CommandSpec> {
        self.registry.specs().cloned().collect()
    }

    pub fn view(&self) -> SessionView {
        self.session.lock().view()
    }

    pub fn revision(&self) -> u64 {
        self.session.lock().revision()
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        self.session.lock().snapshot()
    }

    pub fn run(&self, record: &CommandRecord) -> Result<ShellUpdate, CommandError> {
        let mut command = self.registry.build(record)?;
        self.change(|session| command.apply(session))
    }

    pub fn restore(&self, snapshot: &SessionSnapshot) -> ShellUpdate {
        self.change(|session| {
            session.restore(snapshot);
            Ok(())
        })
        .unwrap_or_else(|_| self.unchanged())
    }

    pub fn window_closed(&self, label: &str) -> ShellUpdate {
        self.change(|session| {
            session.window_closed(label);
            Ok(())
        })
        .unwrap_or_else(|_| self.unchanged())
    }

    fn unchanged(&self) -> ShellUpdate {
        ShellUpdate {
            view: self.view(),
            opened: Vec::new(),
            windows_added: Vec::new(),
            windows_removed: Vec::new(),
        }
    }

    /// runs a change and works out what the host has to do about it.
    /// a failed change leaves the session as it was
    fn change(
        &self,
        change: impl FnOnce(&mut Session) -> CommandResult,
    ) -> Result<ShellUpdate, CommandError> {
        let update = {
            let mut session = self.session.lock();
            let before = session.clone();
            if let Err(error) = change(&mut session) {
                *session = before;
                return Err(error);
            }
            let had: BTreeSet<&str> = before.windows.iter().map(|w| w.label.as_str()).collect();
            let has: BTreeSet<&str> = session.windows.iter().map(|w| w.label.as_str()).collect();
            ShellUpdate {
                windows_added: session
                    .windows
                    .iter()
                    .filter(|window| !had.contains(window.label.as_str()))
                    .cloned()
                    .collect(),
                windows_removed: had
                    .difference(&has)
                    .map(|label| (*label).to_owned())
                    .collect(),
                opened: session.take_opened(),
                view: session.view(),
            }
        };
        if let Some(bus) = &self.bus {
            bus.publish(SessionChanged {
                session: update.view.clone(),
            });
        }
        Ok(update)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn open(session: &mut Session, window: &str, paths: &[&str]) -> Vec<DocumentId> {
        let paths: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
        let plan = plan_open(&paths, &OpenTarget::Auto);
        session.open(&plan.opens, window).unwrap();
        session.documents().iter().map(|doc| doc.id).collect()
    }

    fn titles(session: &Session, window: &str) -> Vec<String> {
        session
            .tabs_of(window)
            .map(|doc| doc.title.clone())
            .collect()
    }

    fn shown(session: &Session, window: &str) -> View {
        session.window(window).unwrap().view.clone()
    }

    #[test]
    fn opening_routes_by_type_and_shows_the_last_file() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.mp4"]);
        assert_eq!(session.documents()[0].workspace, "image");
        assert_eq!(session.documents()[1].workspace, "video");
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[1] });
        assert_eq!(session.take_opened().len(), 2);
        assert!(session.take_opened().is_empty());
    }

    #[test]
    fn opening_a_file_twice_brings_the_first_one_back() {
        let mut session = Session::new();
        open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png"]);
        open(&mut session, MAIN_WINDOW, &["/a.png"]);
        assert_eq!(session.documents().len(), 2);
        let first = session.documents()[0].id;
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: first });
    }

    #[test]
    fn closing_shows_the_neighbour_then_home() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png", "/c.png"]);
        session.focus(ids[1]).unwrap();

        session.close(ids[1]).unwrap();
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[2] });
        session.close(ids[2]).unwrap();
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[0] });
        session.close(ids[0]).unwrap();
        assert_eq!(shown(&session, MAIN_WINDOW), View::Home);
        assert!(session.close(ids[0]).is_err());
    }

    #[test]
    fn closing_a_tab_in_the_background_leaves_the_view_alone() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png"]);
        session.close(ids[0]).unwrap();
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[1] });
    }

    #[test]
    fn a_closed_tab_can_be_reopened() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png"]);
        session.close(ids[0]).unwrap();
        assert_eq!(session.view().closed_count, 1);

        session.reopen_closed(MAIN_WINDOW).unwrap();
        assert_eq!(titles(&session, MAIN_WINDOW), ["b.png", "a.png"]);
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[0] });
        assert!(session.reopen_closed(MAIN_WINDOW).is_err());
    }

    #[test]
    fn a_workspace_shows_its_most_recent_document_or_its_empty_state() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png", "/c.mp4"]);
        session.focus(ids[0]).unwrap();
        session.focus(ids[2]).unwrap();

        session.show_workspace(MAIN_WINDOW, "image").unwrap();
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[0] });

        session.show_workspace(MAIN_WINDOW, "audio").unwrap();
        assert_eq!(
            shown(&session, MAIN_WINDOW),
            View::Workspace { id: "audio".into() }
        );
        session.show_home(MAIN_WINDOW).unwrap();
        assert_eq!(shown(&session, MAIN_WINDOW), View::Home);
    }

    #[test]
    fn tabs_cycle_and_wrap() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png", "/c.png"]);
        session.cycle(MAIN_WINDOW, true).unwrap();
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[0] });
        session.cycle(MAIN_WINDOW, false).unwrap();
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[2] });
    }

    #[test]
    fn tabs_move_within_their_own_window() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png", "/c.png"]);
        let other = session.move_to_window(ids[1], None, None).unwrap();
        open(&mut session, &other, &["/d.png"]);

        session.move_tab(ids[2], 0).unwrap();
        assert_eq!(titles(&session, MAIN_WINDOW), ["c.png", "a.png"]);
        session.move_tab(ids[2], 99).unwrap();
        assert_eq!(titles(&session, MAIN_WINDOW), ["a.png", "c.png"]);
        assert_eq!(titles(&session, &other), ["b.png", "d.png"]);
    }

    #[test]
    fn tearing_off_makes_a_window_and_an_empty_one_goes_away() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png"]);
        let spot = Point { x: 200.0, y: 120.0 };

        let label = session.move_to_window(ids[1], None, Some(spot)).unwrap();
        assert_eq!(label, "window-1");
        assert_eq!(session.window(&label).unwrap().position, Some(spot));
        assert_eq!(shown(&session, &label), View::Document { id: ids[1] });
        // main falls back to the tab that is left
        assert_eq!(shown(&session, MAIN_WINDOW), View::Document { id: ids[0] });

        session
            .move_to_window(ids[1], Some(MAIN_WINDOW), None)
            .unwrap();
        assert!(session.window(&label).is_none());
        assert_eq!(titles(&session, MAIN_WINDOW), ["a.png", "b.png"]);
    }

    #[test]
    fn closing_a_torn_off_window_keeps_its_documents() {
        let mut session = Session::new();
        let ids = open(&mut session, MAIN_WINDOW, &["/a.png", "/b.png", "/c.png"]);
        let label = session.move_to_window(ids[0], None, None).unwrap();
        session.move_to_window(ids[2], Some(&label), None).unwrap();

        session.window_closed(&label);
        assert_eq!(titles(&session, MAIN_WINDOW), ["b.png", "a.png", "c.png"]);
        assert_eq!(session.windows().len(), 1);

        // the main window cannot be closed this way
        session.window_closed(MAIN_WINDOW);
        assert_eq!(session.windows().len(), 1);
    }

    #[test]
    fn a_snapshot_restores_with_fresh_ids_and_its_windows() {
        let mut before = Session::new();
        let ids = open(&mut before, MAIN_WINDOW, &["/a.png", "/b.mp4"]);
        before.move_to_window(ids[1], None, None).unwrap();
        before.focus(ids[0]).unwrap();
        let snapshot = before.snapshot();
        let text = serde_json::to_string(&snapshot).unwrap();
        let snapshot: SessionSnapshot = serde_json::from_str(&text).unwrap();

        let mut after = Session::new();
        open(&mut after, MAIN_WINDOW, &["/new.wav"]);
        assert_eq!(after.restore(&snapshot), 2);

        assert_eq!(titles(&after, MAIN_WINDOW), ["new.wav", "a.png"]);
        let torn: Vec<_> = after
            .windows()
            .iter()
            .filter(|w| w.label != MAIN_WINDOW)
            .collect();
        assert_eq!(torn.len(), 1);
        assert_eq!(titles(&after, &torn[0].label), ["b.mp4"]);

        let ids: BTreeSet<_> = after.documents().iter().map(|doc| doc.id).collect();
        assert_eq!(ids.len(), 3, "restored documents must not reuse ids");
        let a = after.documents()[1].id;
        assert_eq!(shown(&after, MAIN_WINDOW), View::Document { id: a });
    }

    #[test]
    fn the_shell_runs_records_and_reports_what_the_host_must_do() {
        let bus = EventBus::new();
        let seen = std::sync::Arc::new(Mutex::new(0));
        let counter = seen.clone();
        let _sub = bus.subscribe(move |_: &SessionChanged| *counter.lock() += 1);
        let shell = Shell::new(Some(bus));

        let update = shell
            .run(&CommandRecord::new(
                "shell.open",
                json!({ "paths": ["/x/a.png", "/x/b.txt"], "window": "main" }),
            ))
            .unwrap();
        assert_eq!(update.opened.len(), 1);
        assert_eq!(update.view.documents.len(), 1);
        let id = update.view.documents[0].id;

        let update = shell
            .run(&CommandRecord::new(
                "shell.move_to_window",
                json!({ "id": id, "position": { "x": 10.0, "y": 20.0 } }),
            ))
            .unwrap();
        assert_eq!(update.windows_added.len(), 1);
        let label = update.windows_added[0].label.clone();

        let update = shell
            .run(&CommandRecord::new("shell.close", json!({ "id": id })))
            .unwrap();
        assert_eq!(update.windows_removed, [label]);
        assert_eq!(*seen.lock(), 3);
    }

    #[test]
    fn a_failed_command_changes_nothing_and_says_why() {
        let shell = Shell::new(None);
        let error = shell
            .run(&CommandRecord::new(
                "shell.open",
                json!({ "paths": ["/notes.txt"], "window": "main" }),
            ))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "notes.txt could not be opened. Vellum does not know this file type yet."
        );
        assert_eq!(shell.revision(), 0);

        let error = shell
            .run(&CommandRecord::new("shell.close", json!({ "id": 42 })))
            .unwrap_err();
        assert_eq!(error.to_string(), "That file is no longer open.");
    }

    #[test]
    fn every_shell_command_is_registered_once() {
        let specs = Shell::new(None).specs();
        assert_eq!(specs.len(), 10);
        let close = specs.iter().find(|spec| spec.id == "shell.close").unwrap();
        assert_eq!(close.shortcut.as_deref(), Some("Mod+W"));
    }
}
