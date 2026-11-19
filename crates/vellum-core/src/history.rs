//! branching undo/redo history.
//!
//! history is a tree, not a stack. undoing a few steps and then doing something new starts a new
//! branch instead of throwing the old steps away, so the user can always get back to them with
//! [`History::jump_to`]. named snapshots are just labels on tree nodes.

use std::collections::{BTreeMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::command::{Command, CommandError, CommandRecord};

pub type NodeId = usize;

const ROOT: NodeId = 0;

struct Node<T> {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    // child that redo should follow, the most recently visited one
    redo_child: Option<NodeId>,
    command: Option<Box<dyn Command<T>>>,
    label: String,
    created_ms: u64,
}

/// one row in the history panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryEntry {
    pub id: u32,
    pub parent: Option<u32>,
    pub label: String,
    pub created_ms: f64,
    /// true for the current node and everything between it and the root
    pub on_current_path: bool,
    pub is_current: bool,
    pub snapshot: Option<String>,
}

/// small summary the ui needs for undo/redo buttons and menu labels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub dirty: bool,
}

pub struct History<T> {
    nodes: Vec<Node<T>>,
    current: NodeId,
    snapshots: BTreeMap<String, NodeId>,
    // while true the next execute may merge into the current node (slider drags)
    merge_open: bool,
    saved: Option<NodeId>,
    recording: Option<Recording>,
}

struct Recording {
    steps: Vec<CommandRecord>,
    // node whose record is the last step, so merges replace it instead of adding a step
    last_node: Option<NodeId>,
}

impl<T: 'static> Default for History<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static> History<T> {
    pub fn new() -> Self {
        let root = Node {
            parent: None,
            children: Vec::new(),
            redo_child: None,
            command: None,
            label: "Open".to_string(),
            created_ms: now_ms(),
        };
        Self {
            nodes: vec![root],
            current: ROOT,
            snapshots: BTreeMap::new(),
            merge_open: false,
            saved: Some(ROOT),
            recording: None,
        }
    }

    /// applies `command` and records it. if the command fails nothing changes.
    pub fn execute(
        &mut self,
        mut command: Box<dyn Command<T>>,
        target: &mut T,
    ) -> Result<(), CommandError> {
        let record = match self.recording {
            Some(_) => Some(command.record()?),
            None => None,
        };
        command.apply(target)?;

        if self.try_merge(command.as_ref()) {
            let current = self.current;
            if let Some(cmd) = &self.nodes[current].command {
                self.nodes[current].label = cmd.label();
            }
            if let (Some(rec), Some(record)) = (self.recording.as_mut(), record) {
                if rec.last_node == Some(current) {
                    // the merged command now carries the final value, so re-record it
                    if let (Some(last), Some(cmd)) =
                        (rec.steps.last_mut(), &self.nodes[current].command)
                    {
                        *last = cmd.record()?;
                    }
                } else {
                    rec.steps.push(record);
                    rec.last_node = Some(current);
                }
            }
            return Ok(());
        }

        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: Some(self.current),
            children: Vec::new(),
            redo_child: None,
            label: command.label(),
            command: Some(command),
            created_ms: now_ms(),
        });
        let parent = &mut self.nodes[self.current];
        parent.children.push(id);
        parent.redo_child = Some(id);
        self.current = id;
        self.merge_open = true;

        if let (Some(rec), Some(record)) = (self.recording.as_mut(), record) {
            rec.steps.push(record);
            rec.last_node = Some(id);
        }
        Ok(())
    }

    fn try_merge(&mut self, next: &dyn Command<T>) -> bool {
        // merging into a node that has children would silently rewrite another branch
        if !self.merge_open || !self.nodes[self.current].children.is_empty() {
            return false;
        }
        // never merge into a node someone named or saved, they expect that exact state back
        if self.saved == Some(self.current) || self.snapshots.values().any(|&n| n == self.current) {
            return false;
        }
        match self.nodes[self.current].command.as_mut() {
            Some(cmd) if cmd.kind() == next.kind() => cmd.merge(next),
            _ => false,
        }
    }

    /// stops the next command merging into the current one. call it when a drag ends.
    pub fn seal(&mut self) {
        self.merge_open = false;
    }

    pub fn can_undo(&self) -> bool {
        self.current != ROOT
    }

    pub fn can_redo(&self) -> bool {
        self.redo_target().is_some()
    }

    fn redo_target(&self) -> Option<NodeId> {
        let node = &self.nodes[self.current];
        node.redo_child.or_else(|| node.children.last().copied())
    }

    /// returns false if there was nothing to undo.
    pub fn undo(&mut self, target: &mut T) -> Result<bool, CommandError> {
        self.seal();
        let Some(parent) = self.nodes[self.current].parent else {
            return Ok(false);
        };
        if let Some(cmd) = self.nodes[self.current].command.as_mut() {
            cmd.undo(target)?;
        }
        self.nodes[parent].redo_child = Some(self.current);
        self.current = parent;
        Ok(true)
    }

    /// returns false if there was nothing to redo.
    pub fn redo(&mut self, target: &mut T) -> Result<bool, CommandError> {
        self.seal();
        let Some(child) = self.redo_target() else {
            return Ok(false);
        };
        self.redo_into(child, target)?;
        Ok(true)
    }

    fn redo_into(&mut self, child: NodeId, target: &mut T) -> Result<(), CommandError> {
        if let Some(cmd) = self.nodes[child].command.as_mut() {
            cmd.apply(target)?;
        }
        self.nodes[self.current].redo_child = Some(child);
        self.current = child;
        Ok(())
    }

    /// moves to any node in the tree, undoing up to the common ancestor and redoing down.
    /// if a step fails the history stops at the last node that worked.
    pub fn jump_to(&mut self, id: NodeId, target: &mut T) -> Result<(), CommandError> {
        if id >= self.nodes.len() {
            return Err(CommandError::failed(format!(
                "history step {id} does not exist"
            )));
        }
        self.seal();
        let target_path = self.path_from_root(id);
        let on_target_path: HashSet<NodeId> = target_path.iter().copied().collect();

        while !on_target_path.contains(&self.current) {
            self.undo(target)?;
        }
        let start = target_path
            .iter()
            .position(|&n| n == self.current)
            .map_or(0, |i| i + 1);
        for &node in &target_path[start..] {
            self.redo_into(node, target)?;
        }
        Ok(())
    }

    fn path_from_root(&self, mut id: NodeId) -> Vec<NodeId> {
        let mut path = vec![id];
        while let Some(parent) = self.nodes[id].parent {
            path.push(parent);
            id = parent;
        }
        path.reverse();
        path
    }

    pub fn current(&self) -> NodeId {
        self.current
    }

    /// names the current state. using an existing name moves it here.
    pub fn snapshot(&mut self, name: &str) {
        self.seal();
        self.snapshots.insert(name.to_string(), self.current);
    }

    pub fn restore_snapshot(&mut self, name: &str, target: &mut T) -> Result<(), CommandError> {
        let id = *self
            .snapshots
            .get(name)
            .ok_or_else(|| CommandError::failed(format!("no snapshot called \"{name}\"")))?;
        self.jump_to(id, target)
    }

    pub fn remove_snapshot(&mut self, name: &str) -> bool {
        self.snapshots.remove(name).is_some()
    }

    pub fn snapshots(&self) -> impl Iterator<Item = (&str, NodeId)> {
        self.snapshots.iter().map(|(name, &id)| (name.as_str(), id))
    }

    pub fn mark_saved(&mut self) {
        self.seal();
        self.saved = Some(self.current);
    }

    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.current)
    }

    pub fn start_recording(&mut self) {
        self.seal();
        self.recording = Some(Recording {
            steps: Vec::new(),
            last_node: None,
        });
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// stops recording and hands back the steps, ready to save as a macro.
    pub fn stop_recording(&mut self) -> Vec<CommandRecord> {
        self.recording.take().map(|r| r.steps).unwrap_or_default()
    }

    pub fn state(&self) -> HistoryState {
        HistoryState {
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            undo_label: self
                .can_undo()
                .then(|| self.nodes[self.current].label.clone()),
            redo_label: self.redo_target().map(|n| self.nodes[n].label.clone()),
            dirty: self.is_dirty(),
        }
    }

    /// every node in creation order, for the history panel.
    pub fn entries(&self) -> Vec<HistoryEntry> {
        let path: HashSet<NodeId> = self.path_from_root(self.current).into_iter().collect();
        let mut names: BTreeMap<NodeId, &str> = BTreeMap::new();
        for (name, &id) in &self.snapshots {
            names.entry(id).or_insert(name);
        }
        self.nodes
            .iter()
            .enumerate()
            .map(|(id, node)| HistoryEntry {
                id: id as u32,
                parent: node.parent.map(|p| p as u32),
                label: node.label.clone(),
                created_ms: node.created_ms as f64,
                on_current_path: path.contains(&id),
                is_current: id == self.current,
                snapshot: names.get(&id).map(|s| s.to_string()),
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::tests::{registry, Doc, Rename, SetValue};

    fn run(history: &mut History<Doc>, doc: &mut Doc, to: i32) {
        history.execute(SetValue::boxed(to), doc).unwrap();
        history.seal();
    }

    #[test]
    fn undo_and_redo_walk_back_and_forth() {
        let mut doc = Doc::default();
        let mut h = History::new();
        run(&mut h, &mut doc, 1);
        run(&mut h, &mut doc, 2);
        run(&mut h, &mut doc, 3);
        assert_eq!(doc.value, 3);

        assert!(h.undo(&mut doc).unwrap());
        assert!(h.undo(&mut doc).unwrap());
        assert_eq!(doc.value, 1);
        assert!(h.redo(&mut doc).unwrap());
        assert_eq!(doc.value, 2);

        assert!(h.undo(&mut doc).unwrap());
        assert!(h.undo(&mut doc).unwrap());
        assert!(!h.undo(&mut doc).unwrap(), "nothing left to undo");
        assert_eq!(doc.value, 0);
        assert!(!h.can_undo());
    }

    #[test]
    fn failed_command_leaves_history_untouched() {
        let mut doc = Doc::default();
        let mut h = History::new();
        assert!(h.execute(Rename::boxed(""), &mut doc).is_err());
        assert!(h.is_empty());
        assert!(!h.is_dirty());
    }

    #[test]
    fn slider_drags_merge_until_sealed() {
        let mut doc = Doc::default();
        let mut h = History::new();
        for v in 1..=10 {
            h.execute(SetValue::boxed(v), &mut doc).unwrap();
        }
        h.seal();
        assert_eq!(h.len(), 1, "one drag is one step");
        assert_eq!(h.state().undo_label.as_deref(), Some("Set value to 10"));

        h.execute(SetValue::boxed(20), &mut doc).unwrap();
        assert_eq!(h.len(), 2);

        h.undo(&mut doc).unwrap();
        assert_eq!(doc.value, 10);
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.value, 0, "merged command undoes the whole drag");
    }

    #[test]
    fn different_commands_do_not_merge() {
        let mut doc = Doc::default();
        let mut h = History::new();
        h.execute(SetValue::boxed(1), &mut doc).unwrap();
        h.execute(Rename::boxed("a"), &mut doc).unwrap();
        h.execute(SetValue::boxed(2), &mut doc).unwrap();
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn new_work_after_undo_branches_instead_of_deleting() {
        let mut doc = Doc::default();
        let mut h = History::new();
        run(&mut h, &mut doc, 1);
        run(&mut h, &mut doc, 2);
        let old_tip = h.current();

        h.undo(&mut doc).unwrap();
        run(&mut h, &mut doc, 50);
        assert_eq!(doc.value, 50);
        assert_eq!(h.len(), 3, "the old branch is still there");

        // jump across branches: undo to the fork, then redo down the old branch
        h.jump_to(old_tip, &mut doc).unwrap();
        assert_eq!(doc.value, 2);

        // redo after undo follows the branch we came from
        h.undo(&mut doc).unwrap();
        h.redo(&mut doc).unwrap();
        assert_eq!(doc.value, 2);
    }

    #[test]
    fn named_snapshots_restore_exact_states() {
        let mut doc = Doc::default();
        let mut h = History::new();
        run(&mut h, &mut doc, 1);
        h.snapshot("before");
        run(&mut h, &mut doc, 2);
        h.execute(Rename::boxed("after"), &mut doc).unwrap();

        h.restore_snapshot("before", &mut doc).unwrap();
        assert_eq!(
            doc,
            Doc {
                value: 1,
                name: String::new()
            }
        );
        assert!(h.restore_snapshot("missing", &mut doc).is_err());

        let entries = h.entries();
        let snap = entries.iter().find(|e| e.snapshot.is_some()).unwrap();
        assert!(snap.is_current);
    }

    #[test]
    fn snapshot_node_is_never_merged_into() {
        let mut doc = Doc::default();
        let mut h = History::new();
        h.execute(SetValue::boxed(1), &mut doc).unwrap();
        h.snapshot("one");
        h.execute(SetValue::boxed(2), &mut doc).unwrap();
        h.restore_snapshot("one", &mut doc).unwrap();
        assert_eq!(doc.value, 1);
    }

    #[test]
    fn dirty_tracks_the_saved_node() {
        let mut doc = Doc::default();
        let mut h = History::new();
        assert!(!h.is_dirty());
        run(&mut h, &mut doc, 1);
        assert!(h.is_dirty());
        h.mark_saved();
        assert!(!h.state().dirty);
        h.undo(&mut doc).unwrap();
        assert!(h.is_dirty());
        h.redo(&mut doc).unwrap();
        assert!(!h.is_dirty());
    }

    #[test]
    fn recorded_macro_replays_on_a_fresh_document() {
        let reg = registry();
        let mut doc = Doc::default();
        let mut h = History::new();
        h.start_recording();
        // a drag records as one step with the final value
        for v in 1..=5 {
            h.execute(SetValue::boxed(v), &mut doc).unwrap();
        }
        h.seal();
        h.execute(Rename::boxed("done"), &mut doc).unwrap();
        let steps = h.stop_recording();
        assert_eq!(steps.len(), 2);
        assert!(!h.is_recording());

        let mut other = Doc::default();
        let mut other_history = History::new();
        for step in &steps {
            let cmd = reg.decode(step).unwrap();
            other_history.execute(cmd, &mut other).unwrap();
        }
        assert_eq!(other, doc);
    }

    #[test]
    fn entries_mark_the_current_path() {
        let mut doc = Doc::default();
        let mut h = History::new();
        run(&mut h, &mut doc, 1);
        run(&mut h, &mut doc, 2);
        h.undo(&mut doc).unwrap();
        run(&mut h, &mut doc, 3);

        let entries = h.entries();
        assert_eq!(entries.len(), 4);
        let on_path: Vec<u32> = entries
            .iter()
            .filter(|e| e.on_current_path)
            .map(|e| e.id)
            .collect();
        assert_eq!(on_path, [0, 1, 3]);
        assert_eq!(entries[2].parent, Some(1));
    }

    #[test]
    fn jump_to_unknown_node_is_an_error() {
        let mut doc = Doc::default();
        let mut h: History<Doc> = History::new();
        assert!(h.jump_to(99, &mut doc).is_err());
    }
}
