//! branching undo/redo history with named snapshots.
//!
//! history is a tree, not a stack. undoing and then doing something new starts a new branch
//! instead of throwing the old one away, so the user can always get back. the root node is
//! the oldest state we can return to and has no command. every other node holds the
//! command that leads to it from its parent.
//!
//! snapshots are named pointers to nodes. restoring one walks the tree (undo up to the
//! common ancestor, redo down to the snapshot), so they cost nothing to create. later phases
//! can attach cached document states to snapshots if replaying gets too slow.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;
use ts_rs::TS;

use crate::command::{Command, CommandError, CommandRecord};
use crate::events::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS)]
#[ts(export)]
pub struct NodeId(#[ts(type = "number")] u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS)]
#[ts(export)]
pub struct SnapshotId(#[ts(type = "number")] u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub name: String,
    pub node: NodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLimits {
    /// how many steps back from the current state are kept
    pub max_steps: usize,
    /// soft cap on the undo memory reported by commands
    pub max_bytes: usize,
    /// commands only merge if they arrive within this window of the previous one
    pub merge_window: Duration,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            max_steps: 500,
            max_bytes: 1024 * 1024 * 1024,
            merge_window: Duration::from_millis(1500),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecuteOutcome {
    /// a new history step was added
    Pushed(NodeId),
    /// the command was folded into the current step
    Merged(NodeId),
    /// the command ran but is not undoable, history is unchanged
    Applied,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error("history step {0:?} no longer exists")]
    UnknownNode(NodeId),
    #[error("snapshot {0:?} does not exist")]
    UnknownSnapshot(SnapshotId),
}

/// small summary for the undo/redo buttons and the unsaved marker
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistorySummary {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_name: Option<String>,
    pub redo_name: Option<String>,
    pub dirty: bool,
}

/// sent whenever a document's history changes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryChanged {
    pub document: String,
    pub summary: HistorySummary,
}

impl Event for HistoryChanged {
    const NAME: &'static str = "history:changed";
}

/// one row in the history panel
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryEntryView {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub name: String,
    pub kind: String,
    /// true for the root and every step between it and the current state
    pub on_current_path: bool,
}

/// the whole tree, for drawing the history panel
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryView {
    pub root: NodeId,
    pub current: NodeId,
    /// sorted oldest first
    pub entries: Vec<HistoryEntryView>,
    pub snapshots: Vec<Snapshot>,
}

struct Step<T> {
    command: Box<dyn Command<T>>,
    name: String,
    kind: &'static str,
    cost: usize,
    touched: Instant,
}

struct Node<T> {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    /// the branch redo follows, which is the one most recently visited
    redo_child: Option<NodeId>,
    step: Option<Step<T>>,
}

struct RecordedStep {
    node: Option<NodeId>,
    record: CommandRecord,
}

pub struct History<T> {
    nodes: HashMap<NodeId, Node<T>>,
    root: NodeId,
    current: NodeId,
    next_node: u64,
    snapshots: Vec<Snapshot>,
    next_snapshot: u64,
    saved: Option<NodeId>,
    sealed: bool,
    limits: HistoryLimits,
    total_cost: usize,
    recording: Option<Vec<RecordedStep>>,
}

impl<T: 'static> History<T> {
    pub fn new(limits: HistoryLimits) -> Self {
        let root = NodeId(0);
        let mut nodes = HashMap::new();
        nodes.insert(
            root,
            Node {
                parent: None,
                children: Vec::new(),
                redo_child: None,
                step: None,
            },
        );
        Self {
            nodes,
            root,
            current: root,
            next_node: 1,
            snapshots: Vec::new(),
            next_snapshot: 1,
            saved: Some(root),
            sealed: false,
            limits,
            total_cost: 0,
            recording: None,
        }
    }

    pub fn limits(&self) -> HistoryLimits {
        self.limits
    }

    pub fn set_limits(&mut self, limits: HistoryLimits) {
        self.limits = limits;
        self.prune();
    }

    pub fn current(&self) -> NodeId {
        self.current
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    /// runs a command and records it. on error nothing changes.
    pub fn execute(
        &mut self,
        mut command: Box<dyn Command<T>>,
        target: &mut T,
    ) -> Result<ExecuteOutcome, CommandError> {
        // grab the record before apply so a params failure cannot leave a half done step
        let record = match self.recording {
            Some(_) => Some(CommandRecord::of(command.as_ref())?),
            None => None,
        };

        command.apply(target)?;

        if !command.undoable() {
            if let (Some(steps), Some(record)) = (self.recording.as_mut(), record) {
                steps.push(RecordedStep { node: None, record });
            }
            return Ok(ExecuteOutcome::Applied);
        }

        if self.try_merge(command.as_ref()) {
            let id = self.current;
            self.rerecord(id);
            return Ok(ExecuteOutcome::Merged(id));
        }

        let id = NodeId(self.next_node);
        self.next_node += 1;
        let cost = command.memory_cost();
        let step = Step {
            name: command.name(),
            kind: command.kind(),
            command,
            cost,
            touched: Instant::now(),
        };
        self.nodes.insert(
            id,
            Node {
                parent: Some(self.current),
                children: Vec::new(),
                redo_child: None,
                step: Some(step),
            },
        );
        if let Some(parent) = self.nodes.get_mut(&self.current) {
            parent.children.push(id);
            parent.redo_child = Some(id);
        }
        self.total_cost += cost;
        self.current = id;
        self.sealed = false;

        if let (Some(steps), Some(record)) = (self.recording.as_mut(), record) {
            steps.push(RecordedStep {
                node: Some(id),
                record,
            });
        }

        self.prune();
        Ok(ExecuteOutcome::Pushed(id))
    }

    fn try_merge(&mut self, next: &dyn Command<T>) -> bool {
        if self.sealed || self.current == self.root || self.saved == Some(self.current) {
            return false;
        }
        if self.snapshots.iter().any(|snap| snap.node == self.current) {
            return false;
        }
        let window = self.limits.merge_window;
        let Some(node) = self.nodes.get_mut(&self.current) else {
            return false;
        };
        if !node.children.is_empty() {
            return false;
        }
        let Some(step) = node.step.as_mut() else {
            return false;
        };
        if step.touched.elapsed() >= window || !step.command.merge(next) {
            return false;
        }
        step.name = step.command.name();
        step.touched = Instant::now();
        let new_cost = step.command.memory_cost();
        self.total_cost = self.total_cost - step.cost + new_cost;
        step.cost = new_cost;
        true
    }

    /// refreshes the recorded params of a step after it merged
    fn rerecord(&mut self, id: NodeId) {
        let Some(steps) = self.recording.as_mut() else {
            return;
        };
        let Some(record) = self
            .nodes
            .get(&id)
            .and_then(|node| node.step.as_ref())
            .and_then(|step| CommandRecord::of(step.command.as_ref()).ok())
        else {
            return;
        };
        match steps.iter_mut().rev().find(|s| s.node == Some(id)) {
            Some(existing) => existing.record = record,
            None => steps.push(RecordedStep {
                node: Some(id),
                record,
            }),
        }
    }

    /// stops the next command merging into the current step, eg on pointer up after a drag
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn can_undo(&self) -> bool {
        self.current != self.root
    }

    pub fn can_redo(&self) -> bool {
        self.redo_target().is_some()
    }

    pub fn undo_name(&self) -> Option<&str> {
        self.step(self.current).map(|step| step.name.as_str())
    }

    pub fn redo_name(&self) -> Option<&str> {
        self.redo_target()
            .and_then(|id| self.step(id))
            .map(|step| step.name.as_str())
    }

    /// steps back once. returns false if there was nothing to undo.
    pub fn undo(&mut self, target: &mut T) -> Result<bool, CommandError> {
        if self.current == self.root {
            return Ok(false);
        }
        let id = self.current;
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| CommandError::failed("history is out of sync"))?;
        let parent = node.parent;
        if let Some(step) = node.step.as_mut() {
            step.command.revert(target)?;
        }
        let parent = parent.ok_or_else(|| CommandError::failed("history is out of sync"))?;
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.redo_child = Some(id);
        }
        self.current = parent;
        self.sealed = true;
        if let Some(steps) = self.recording.as_mut()
            && let Some(pos) = steps.iter().rposition(|s| s.node == Some(id))
        {
            steps.remove(pos);
        }
        Ok(true)
    }

    /// steps forward along the most recently visited branch. returns false if there was
    /// nothing to redo.
    pub fn redo(&mut self, target: &mut T) -> Result<bool, CommandError> {
        let Some(id) = self.redo_target() else {
            return Ok(false);
        };
        self.redo_into(id, target)?;
        Ok(true)
    }

    fn redo_target(&self) -> Option<NodeId> {
        self.nodes
            .get(&self.current)
            .and_then(|node| node.redo_child)
    }

    fn redo_into(&mut self, id: NodeId, target: &mut T) -> Result<(), CommandError> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| CommandError::failed("history is out of sync"))?;
        let step = node
            .step
            .as_mut()
            .ok_or_else(|| CommandError::failed("history is out of sync"))?;
        step.command.apply(target)?;
        step.touched = Instant::now();
        let record = match self.recording {
            Some(_) => CommandRecord::of(step.command.as_ref()).ok(),
            None => None,
        };
        if let Some(parent) = self.nodes.get_mut(&self.current) {
            parent.redo_child = Some(id);
        }
        self.current = id;
        self.sealed = true;
        if let (Some(steps), Some(record)) = (self.recording.as_mut(), record) {
            steps.push(RecordedStep {
                node: Some(id),
                record,
            });
        }
        Ok(())
    }

    /// moves to any step in the tree, on any branch. if a command fails part way the
    /// history stops at the last step that worked and the error is returned.
    pub fn checkout(&mut self, id: NodeId, target: &mut T) -> Result<(), HistoryError> {
        if !self.nodes.contains_key(&id) {
            return Err(HistoryError::UnknownNode(id));
        }
        // path from the target up to the root, target first
        let mut path = vec![id];
        while let Some(parent) = path.last().and_then(|n| self.nodes.get(n)?.parent) {
            path.push(parent);
        }
        while !path.contains(&self.current) {
            if !self.undo(target)? {
                break;
            }
        }
        let Some(pos) = path.iter().position(|n| *n == self.current) else {
            return Err(HistoryError::UnknownNode(id));
        };
        for next in path[..pos].iter().rev() {
            self.redo_into(*next, target)?;
        }
        Ok(())
    }

    pub fn create_snapshot(&mut self, name: impl Into<String>) -> SnapshotId {
        let id = SnapshotId(self.next_snapshot);
        self.next_snapshot += 1;
        self.snapshots.push(Snapshot {
            id,
            name: name.into(),
            node: self.current,
        });
        self.sealed = true;
        id
    }

    pub fn restore_snapshot(&mut self, id: SnapshotId, target: &mut T) -> Result<(), HistoryError> {
        let node = self
            .snapshots
            .iter()
            .find(|snap| snap.id == id)
            .map(|snap| snap.node)
            .ok_or(HistoryError::UnknownSnapshot(id))?;
        self.checkout(node, target)
    }

    pub fn rename_snapshot(
        &mut self,
        id: SnapshotId,
        name: impl Into<String>,
    ) -> Result<(), HistoryError> {
        let snap = self
            .snapshots
            .iter_mut()
            .find(|snap| snap.id == id)
            .ok_or(HistoryError::UnknownSnapshot(id))?;
        snap.name = name.into();
        Ok(())
    }

    pub fn delete_snapshot(&mut self, id: SnapshotId) -> Result<(), HistoryError> {
        let before = self.snapshots.len();
        self.snapshots.retain(|snap| snap.id != id);
        if self.snapshots.len() == before {
            return Err(HistoryError::UnknownSnapshot(id));
        }
        self.prune();
        Ok(())
    }

    pub fn snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    /// call after the document was saved, so `is_dirty` can compare against it
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.current);
        self.sealed = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.current)
    }

    /// starts collecting the commands that run from now on into a macro. undo while
    /// recording drops the undone step from the macro, so it matches what the user sees.
    pub fn start_recording(&mut self) {
        self.recording = Some(Vec::new());
        self.sealed = true;
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    pub fn stop_recording(&mut self) -> Vec<CommandRecord> {
        self.recording
            .take()
            .unwrap_or_default()
            .into_iter()
            .map(|step| step.record)
            .collect()
    }

    pub fn summary(&self) -> HistorySummary {
        HistorySummary {
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            undo_name: self.undo_name().map(str::to_owned),
            redo_name: self.redo_name().map(str::to_owned),
            dirty: self.is_dirty(),
        }
    }

    pub fn view(&self) -> HistoryView {
        let mut on_path = vec![self.current];
        while let Some(parent) = on_path.last().and_then(|n| self.nodes.get(n)?.parent) {
            on_path.push(parent);
        }
        let mut entries: Vec<_> = self
            .nodes
            .iter()
            .map(|(id, node)| HistoryEntryView {
                id: *id,
                parent: node.parent,
                name: node
                    .step
                    .as_ref()
                    .map_or_else(|| "Start".to_owned(), |s| s.name.clone()),
                kind: node.step.as_ref().map_or("", |s| s.kind).to_owned(),
                on_current_path: on_path.contains(id),
            })
            .collect();
        entries.sort_by_key(|entry| entry.id);
        HistoryView {
            root: self.root,
            current: self.current,
            entries,
            snapshots: self.snapshots.clone(),
        }
    }

    /// number of steps in the whole tree, not counting the root
    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// approximate undo memory in bytes
    pub fn memory_cost(&self) -> usize {
        self.total_cost
    }

    fn step(&self, id: NodeId) -> Option<&Step<T>> {
        self.nodes.get(&id).and_then(|node| node.step.as_ref())
    }

    fn depth(&self, mut id: NodeId) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.nodes.get(&id).and_then(|node| node.parent) {
            depth += 1;
            id = parent;
        }
        depth
    }

    /// drops the oldest steps until the limits fit. this moves the root forward along the
    /// path to the current state and throws away branches hanging off the old root. it
    /// never drops a step a snapshot points at, so snapshots can hold history past the
    /// limits until the user deletes them.
    fn prune(&mut self) {
        loop {
            let over_steps = self.depth(self.current) > self.limits.max_steps;
            let over_bytes = self.total_cost > self.limits.max_bytes;
            if !(over_steps || over_bytes) || self.current == self.root {
                return;
            }
            // the root's child on the way to the current state becomes the new root
            let mut new_root = self.current;
            while let Some(parent) = self.nodes.get(&new_root).and_then(|n| n.parent) {
                if parent == self.root {
                    break;
                }
                new_root = parent;
            }

            let mut doomed = vec![self.root];
            let mut stack: Vec<NodeId> = self
                .nodes
                .get(&self.root)
                .map(|n| n.children.clone())
                .unwrap_or_default();
            while let Some(id) = stack.pop() {
                if id == new_root {
                    continue;
                }
                doomed.push(id);
                if let Some(node) = self.nodes.get(&id) {
                    stack.extend(node.children.iter().copied());
                }
            }
            if self
                .snapshots
                .iter()
                .any(|snap| doomed.contains(&snap.node))
            {
                return;
            }

            for id in &doomed {
                if let Some(step) = self.nodes.remove(id).and_then(|node| node.step) {
                    self.total_cost -= step.cost;
                }
                if self.saved == Some(*id) {
                    self.saved = None;
                }
            }
            if let Some(node) = self.nodes.get_mut(&new_root) {
                node.parent = None;
                if let Some(step) = node.step.take() {
                    self.total_cost -= step.cost;
                }
            }
            if let Some(steps) = self.recording.as_mut() {
                for step in steps.iter_mut() {
                    if step
                        .node
                        .is_some_and(|n| doomed.contains(&n) || n == new_root)
                    {
                        step.node = None;
                    }
                }
            }
            self.root = new_root;
        }
    }
}

impl<T: 'static> Default for History<T> {
    fn default() -> Self {
        Self::new(HistoryLimits::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    fn history() -> History<Doc> {
        History::default()
    }

    fn sealed_add(history: &mut History<Doc>, doc: &mut Doc, amount: i32) -> ExecuteOutcome {
        history.seal();
        history.execute(Box::new(Add { amount }), doc).unwrap()
    }

    #[test]
    fn undo_and_redo_walk_back_and_forth() {
        let mut doc = Doc::default();
        let mut h = history();
        assert!(!h.can_undo());
        assert!(!h.undo(&mut doc).unwrap());

        h.execute(Box::new(Set::new(5)), &mut doc).unwrap();
        h.execute(Box::new(Set::new(9)), &mut doc).unwrap();
        assert_eq!(doc.value, 9);
        assert_eq!(h.undo_name(), Some("Set to 9"));

        assert!(h.undo(&mut doc).unwrap());
        assert_eq!(doc.value, 5);
        assert_eq!(h.redo_name(), Some("Set to 9"));
        assert!(h.undo(&mut doc).unwrap());
        assert_eq!(doc.value, 0);
        assert!(!h.can_undo());

        assert!(h.redo(&mut doc).unwrap());
        assert!(h.redo(&mut doc).unwrap());
        assert_eq!(doc.value, 9);
        assert!(!h.redo(&mut doc).unwrap());
    }

    #[test]
    fn failed_command_leaves_history_alone() {
        let mut doc = Doc::default();
        let mut h = history();
        h.execute(Box::new(Set::new(1)), &mut doc).unwrap();
        assert!(h.execute(Box::new(Fail), &mut doc).is_err());
        assert_eq!(h.len(), 1);
        assert_eq!(doc.value, 1);
        assert_eq!(h.undo_name(), Some("Set to 1"));
    }

    #[test]
    fn slider_drag_merges_into_one_step() {
        let mut doc = Doc::default();
        let mut h = history();
        assert!(matches!(
            h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap(),
            ExecuteOutcome::Pushed(_)
        ));
        for _ in 0..9 {
            assert!(matches!(
                h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap(),
                ExecuteOutcome::Merged(_)
            ));
        }
        assert_eq!(doc.value, 10);
        assert_eq!(h.len(), 1);
        assert_eq!(h.undo_name(), Some("Add 10"));
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.value, 0);
    }

    #[test]
    fn seal_and_other_types_stop_merging() {
        let mut doc = Doc::default();
        let mut h = history();
        h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap();
        h.seal();
        h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap();
        h.execute(Box::new(Set::new(4)), &mut doc).unwrap();
        h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap();
        assert_eq!(h.len(), 4);
        assert_eq!(doc.value, 5);
    }

    #[test]
    fn merge_window_expiry_stops_merging() {
        let mut doc = Doc::default();
        let mut h = History::new(HistoryLimits {
            merge_window: Duration::ZERO,
            ..HistoryLimits::default()
        });
        h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap();
        h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap();
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn no_merge_into_a_saved_or_snapshotted_step() {
        let mut doc = Doc::default();
        let mut h = history();
        h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap();
        h.mark_saved();
        assert!(!h.is_dirty());
        h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap();
        assert!(h.is_dirty());
        h.create_snapshot("before");
        h.execute(Box::new(Add { amount: 1 }), &mut doc).unwrap();
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn doing_something_after_undo_starts_a_branch() {
        let mut doc = Doc::default();
        let mut h = history();
        sealed_add(&mut h, &mut doc, 1);
        let ExecuteOutcome::Pushed(old_branch) = sealed_add(&mut h, &mut doc, 10) else {
            panic!("expected a new step");
        };
        h.undo(&mut doc).unwrap();
        let ExecuteOutcome::Pushed(new_branch) = sealed_add(&mut h, &mut doc, 100) else {
            panic!("expected a new step");
        };
        assert_eq!(doc.value, 101);
        assert_eq!(h.len(), 3);

        // the old branch is still there and reachable
        h.checkout(old_branch, &mut doc).unwrap();
        assert_eq!(doc.value, 11);
        assert_eq!(h.current(), old_branch);

        // redo follows the branch visited most recently
        h.undo(&mut doc).unwrap();
        h.redo(&mut doc).unwrap();
        assert_eq!(h.current(), old_branch);

        h.checkout(new_branch, &mut doc).unwrap();
        assert_eq!(doc.value, 101);
        h.undo(&mut doc).unwrap();
        h.redo(&mut doc).unwrap();
        assert_eq!(h.current(), new_branch);

        h.checkout(h.root(), &mut doc).unwrap();
        assert_eq!(doc.value, 0);
    }

    #[test]
    fn snapshots_restore_across_branches() {
        let mut doc = Doc::default();
        let mut h = history();
        h.execute(Box::new(Set::new(3)), &mut doc).unwrap();
        let snap = h.create_snapshot("three");
        h.execute(Box::new(Set::new(4)), &mut doc).unwrap();
        h.undo(&mut doc).unwrap();
        h.undo(&mut doc).unwrap();
        h.execute(Box::new(Set::new(50)), &mut doc).unwrap();

        h.restore_snapshot(snap, &mut doc).unwrap();
        assert_eq!(doc.value, 3);

        h.rename_snapshot(snap, "the good one").unwrap();
        assert_eq!(h.snapshots()[0].name, "the good one");
        h.delete_snapshot(snap).unwrap();
        assert!(h.snapshots().is_empty());
        assert!(matches!(
            h.restore_snapshot(snap, &mut doc),
            Err(HistoryError::UnknownSnapshot(_))
        ));
    }

    #[test]
    fn dirty_tracking_follows_the_saved_step() {
        let mut doc = Doc::default();
        let mut h = history();
        assert!(!h.is_dirty());
        sealed_add(&mut h, &mut doc, 1);
        assert!(h.is_dirty());
        h.mark_saved();
        sealed_add(&mut h, &mut doc, 1);
        assert!(h.is_dirty());
        h.undo(&mut doc).unwrap();
        assert!(!h.is_dirty());
        assert!(!h.summary().dirty);
    }

    #[test]
    fn step_limit_drops_the_oldest_steps() {
        let mut doc = Doc::default();
        let mut h = History::new(HistoryLimits {
            max_steps: 3,
            ..HistoryLimits::default()
        });
        for _ in 0..10 {
            sealed_add(&mut h, &mut doc, 1);
        }
        assert_eq!(doc.value, 10);
        assert_eq!(h.len(), 3);
        while h.undo(&mut doc).unwrap() {}
        assert_eq!(doc.value, 7);
        // the original save point was pruned, so the document counts as unsaved
        assert!(h.is_dirty());
    }

    #[test]
    fn memory_limit_drops_old_steps() {
        let mut doc = Doc::default();
        let cost = std::mem::size_of::<Add>();
        let mut h = History::new(HistoryLimits {
            max_bytes: cost * 4,
            ..HistoryLimits::default()
        });
        for _ in 0..10 {
            sealed_add(&mut h, &mut doc, 1);
        }
        assert_eq!(h.len(), 4);
        assert_eq!(h.memory_cost(), cost * 4);
    }

    #[test]
    fn snapshots_pin_history_against_pruning() {
        let mut doc = Doc::default();
        let mut h = History::new(HistoryLimits {
            max_steps: 2,
            ..HistoryLimits::default()
        });
        sealed_add(&mut h, &mut doc, 1);
        let snap = h.create_snapshot("one");
        for _ in 0..5 {
            sealed_add(&mut h, &mut doc, 1);
        }
        h.restore_snapshot(snap, &mut doc).unwrap();
        assert_eq!(doc.value, 1);

        // once the snapshot is gone the limit applies again
        h.checkout(NodeId(h.next_node - 1), &mut doc).unwrap();
        h.delete_snapshot(snap).unwrap();
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn pruning_drops_branches_off_the_old_root() {
        let mut doc = Doc::default();
        let mut h = History::new(HistoryLimits {
            max_steps: 2,
            ..HistoryLimits::default()
        });
        sealed_add(&mut h, &mut doc, 1);
        sealed_add(&mut h, &mut doc, 1);
        h.undo(&mut doc).unwrap();
        h.undo(&mut doc).unwrap();
        // a second branch straight off the root
        sealed_add(&mut h, &mut doc, 5);
        sealed_add(&mut h, &mut doc, 5);
        sealed_add(&mut h, &mut doc, 5);
        assert_eq!(h.len(), 2);
        assert!(h.view().entries.iter().all(|e| e.on_current_path));
    }

    #[test]
    fn non_undoable_commands_skip_history() {
        let mut doc = Doc::default();
        let mut h = history();
        let outcome = h
            .execute(
                Box::new(Note {
                    text: "exported".into(),
                }),
                &mut doc,
            )
            .unwrap();
        assert_eq!(outcome, ExecuteOutcome::Applied);
        assert_eq!(doc.log, ["exported"]);
        assert!(h.is_empty());
        assert!(!h.is_dirty());
    }

    #[test]
    fn recording_builds_a_macro_that_replays() {
        let mut doc = Doc::default();
        let mut h = history();
        h.start_recording();
        assert!(h.is_recording());

        // a drag of three steps records as one merged step
        for _ in 0..3 {
            h.execute(Box::new(Add { amount: 2 }), &mut doc).unwrap();
        }
        h.execute(
            Box::new(Note {
                text: "hello".into(),
            }),
            &mut doc,
        )
        .unwrap();
        h.execute(Box::new(Set::new(40)), &mut doc).unwrap();
        // undone steps drop out of the macro
        h.execute(Box::new(Set::new(99)), &mut doc).unwrap();
        h.undo(&mut doc).unwrap();

        let steps = h.stop_recording();
        let kinds: Vec<_> = steps.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(kinds, ["test.add", "test.note", "test.set"]);
        assert_eq!(steps[0].params["amount"], 6);

        let registry = registry();
        let mut replay_doc = Doc::default();
        let mut replay = History::default();
        for step in &steps {
            replay
                .execute(registry.instantiate(step).unwrap(), &mut replay_doc)
                .unwrap();
        }
        assert_eq!(replay_doc, doc);
    }

    #[test]
    fn view_marks_the_current_path() {
        let mut doc = Doc::default();
        let mut h = history();
        sealed_add(&mut h, &mut doc, 1);
        sealed_add(&mut h, &mut doc, 2);
        h.undo(&mut doc).unwrap();
        sealed_add(&mut h, &mut doc, 3);
        let view = h.view();
        assert_eq!(view.entries.len(), 4);
        let names: Vec<_> = view
            .entries
            .iter()
            .filter(|e| e.on_current_path)
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, ["Start", "Add 1", "Add 3"]);
        let json = serde_json::to_value(&view).unwrap();
        assert!(json["entries"][0]["onCurrentPath"].as_bool().unwrap());
    }

    #[test]
    fn checkout_of_a_missing_step_is_an_error() {
        let mut doc = Doc::default();
        let mut h = history();
        assert!(matches!(
            h.checkout(NodeId(42), &mut doc),
            Err(HistoryError::UnknownNode(_))
        ));
    }
}
