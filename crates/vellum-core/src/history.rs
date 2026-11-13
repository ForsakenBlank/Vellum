//! undo/redo as a tree instead of a stack.
//!
//! undoing and then doing something new starts a new branch, the old one stays
//! reachable. named snapshots are bookmarks on nodes, and jumping to any node
//! replays the commands along the path between here and there.
//!
//! a `History` does not own the thing it edits. the owner keeps the target and
//! its history side by side and passes the target into every call.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::command::{BoxedCommand, CommandError, CommandRecord, IntoCommand, Merge};
use crate::time::unix_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SnapshotId(u64);

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
    #[error("that history state no longer exists")]
    UnknownNode(NodeId),
    #[error("that snapshot no longer exists")]
    UnknownSnapshot(SnapshotId),
    #[error(transparent)]
    Command(#[from] CommandError),
}

/// how much undo to keep. snapshots are never dropped to satisfy these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLimits {
    pub max_entries: usize,
    /// budget for the sum of `Command::cost`
    pub max_bytes: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            max_bytes: 1 << 30,
        }
    }
}

/// what `execute` did with the command it was given
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Executed {
    /// a new undo step
    Pushed(NodeId),
    /// folded into the previous step
    Merged(NodeId),
    /// it cancelled out the previous step, which is gone now
    Annulled,
    /// it ran but is not undoable, so the history did not change
    Transient,
}

struct Node<T: 'static> {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    /// the child redo goes to, which is the one most recently on the active line
    active_child: Option<NodeId>,
    /// the command that leads from the parent to this node. the root has none
    command: Option<BoxedCommand<T>>,
    name: String,
    created_ms: u64,
    /// when the head last sat here, in ticks. decides what gets pruned first
    visited: u64,
    cost: usize,
}

#[derive(Debug, Clone)]
struct Snapshot {
    id: SnapshotId,
    name: String,
    node: NodeId,
    created_ms: u64,
}

pub struct History<T: 'static> {
    nodes: HashMap<NodeId, Node<T>>,
    root: NodeId,
    head: NodeId,
    next_node: u64,
    next_snapshot: u64,
    ticks: u64,
    snapshots: Vec<Snapshot>,
    saved: Option<NodeId>,
    /// set whenever the head must not absorb the next command
    sealed: bool,
    last_execute: Option<Instant>,
    merge_window: Option<Duration>,
    limits: HistoryLimits,
    bytes: usize,
    revision: u64,
}

impl<T: 'static> Default for History<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static> History<T> {
    pub fn new() -> Self {
        let root = NodeId(0);
        let node = Node {
            parent: None,
            children: Vec::new(),
            active_child: None,
            command: None,
            name: "Start".to_owned(),
            created_ms: unix_ms(),
            visited: 0,
            cost: 0,
        };
        Self {
            nodes: HashMap::from([(root, node)]),
            root,
            head: root,
            next_node: 1,
            next_snapshot: 1,
            ticks: 0,
            snapshots: Vec::new(),
            saved: Some(root),
            sealed: true,
            last_execute: None,
            merge_window: Some(Duration::from_secs(2)),
            limits: HistoryLimits::default(),
            bytes: 0,
            revision: 0,
        }
    }

    pub fn with_limits(mut self, limits: HistoryLimits) -> Self {
        self.limits = limits;
        self
    }

    /// commands further apart than this never merge. `None` leaves it to `seal`
    pub fn with_merge_window(mut self, window: Option<Duration>) -> Self {
        self.merge_window = window;
        self
    }

    pub fn set_limits(&mut self, limits: HistoryLimits) {
        self.limits = limits;
        self.prune();
    }

    /// applies the command and records it as a new undo step, or folds it into
    /// the previous step when that command agrees to merge
    pub fn execute(
        &mut self,
        target: &mut T,
        command: impl IntoCommand<T>,
    ) -> Result<Executed, HistoryError> {
        self.execute_at(target, command.into_command(), Instant::now())
    }

    fn execute_at(
        &mut self,
        target: &mut T,
        mut command: BoxedCommand<T>,
        now: Instant,
    ) -> Result<Executed, HistoryError> {
        command.apply(target)?;
        if !command.is_undoable() {
            return Ok(Executed::Transient);
        }

        if self.head_accepts_merge(now) {
            let head = self.head;
            let node = self.node_mut(head);
            if let Some(previous) = node.command.as_mut() {
                match previous.merge(command.as_ref()) {
                    Merge::Yes => {
                        let cost = previous.cost();
                        node.name = previous.name();
                        let old_cost = std::mem::replace(&mut node.cost, cost);
                        self.bytes = self.bytes - old_cost + cost;
                        self.last_execute = Some(now);
                        self.revision += 1;
                        self.prune();
                        return Ok(Executed::Merged(head));
                    }
                    Merge::Annul => {
                        self.drop_head();
                        return Ok(Executed::Annulled);
                    }
                    Merge::No => {}
                }
            }
        }

        let id = self.push(command);
        self.last_execute = Some(now);
        self.prune();
        Ok(Executed::Pushed(id))
    }

    /// records a command whose effect is already in the target, for gestures like
    /// a brush stroke that paint as they go and only know their undo data at the end
    pub fn push_applied(&mut self, command: impl IntoCommand<T>) -> NodeId {
        let id = self.push(command.into_command());
        self.sealed = true;
        self.prune();
        id
    }

    /// ends the current merge run. call it on pointer up so the next drag of the
    /// same slider becomes its own undo step
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn can_undo(&self) -> bool {
        self.head != self.root
    }

    pub fn can_redo(&self) -> bool {
        self.redo_target().is_some()
    }

    pub fn undo_name(&self) -> Option<&str> {
        self.can_undo()
            .then(|| self.nodes[&self.head].name.as_str())
    }

    pub fn redo_name(&self) -> Option<&str> {
        self.redo_target().map(|id| self.nodes[&id].name.as_str())
    }

    pub fn undo(&mut self, target: &mut T) -> Result<NodeId, HistoryError> {
        let head = self.head;
        let node = self.node_mut(head);
        let (Some(parent), Some(command)) = (node.parent, node.command.as_mut()) else {
            return Err(HistoryError::NothingToUndo);
        };
        command.undo(target)?;
        self.move_head(parent);
        Ok(parent)
    }

    /// steps forward along the branch that was most recently active
    pub fn redo(&mut self, target: &mut T) -> Result<NodeId, HistoryError> {
        let next = self.redo_target().ok_or(HistoryError::NothingToRedo)?;
        self.step_into(target, next)?;
        Ok(next)
    }

    /// moves to any node, on any branch, by undoing up to the shared ancestor
    /// and redoing down from there. if a step fails the head stays where it got to
    pub fn go_to(&mut self, target: &mut T, node: NodeId) -> Result<(), HistoryError> {
        if !self.nodes.contains_key(&node) {
            return Err(HistoryError::UnknownNode(node));
        }
        let current_line: HashSet<NodeId> = self.line_to(self.head).into_iter().collect();
        let mut descent = Vec::new();
        let mut meet = node;
        while !current_line.contains(&meet) {
            descent.push(meet);
            meet = self.nodes[&meet].parent.unwrap_or(self.root);
        }

        while self.head != meet {
            self.undo(target)?;
        }
        for step in descent.into_iter().rev() {
            self.step_into(target, step)?;
        }
        self.sealed = true;
        Ok(())
    }

    /// bookmarks the current state under a name
    pub fn snapshot(&mut self, name: impl Into<String>) -> SnapshotId {
        let id = SnapshotId(self.next_snapshot);
        self.next_snapshot += 1;
        self.snapshots.push(Snapshot {
            id,
            name: name.into(),
            node: self.head,
            created_ms: unix_ms(),
        });
        self.sealed = true;
        self.revision += 1;
        id
    }

    pub fn restore_snapshot(&mut self, target: &mut T, id: SnapshotId) -> Result<(), HistoryError> {
        let node = self.find_snapshot(id)?.node;
        self.go_to(target, node)
    }

    pub fn rename_snapshot(
        &mut self,
        id: SnapshotId,
        name: impl Into<String>,
    ) -> Result<(), HistoryError> {
        let snapshot = self
            .snapshots
            .iter_mut()
            .find(|snapshot| snapshot.id == id)
            .ok_or(HistoryError::UnknownSnapshot(id))?;
        snapshot.name = name.into();
        self.revision += 1;
        Ok(())
    }

    pub fn delete_snapshot(&mut self, id: SnapshotId) -> Result<(), HistoryError> {
        self.find_snapshot(id)?;
        self.snapshots.retain(|snapshot| snapshot.id != id);
        self.revision += 1;
        // the node it pinned may be over budget now
        self.prune();
        Ok(())
    }

    /// remembers the current state as the one on disk
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.head);
        self.sealed = true;
        self.revision += 1;
    }

    /// true when the current state differs from the last saved one
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.head)
    }

    pub fn head(&self) -> NodeId {
        self.head
    }

    /// changes on every edit of the history, so the ui knows when to refresh
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// number of undo steps kept, across all branches
    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// the commands that lead from `since` to the current state, oldest first.
    /// a macro recorder notes the head when recording starts and calls this when it stops
    pub fn records_since(&self, since: NodeId) -> Result<Vec<CommandRecord>, HistoryError> {
        let line = self.line_to(self.head);
        let start = line
            .iter()
            .position(|id| *id == since)
            .ok_or(HistoryError::UnknownNode(since))?;
        line[start + 1..]
            .iter()
            .filter_map(|id| self.nodes[id].command.as_ref())
            .map(|command| command.record().map_err(HistoryError::from))
            .collect()
    }

    /// drops every undo step and snapshot, keeping the target as it is
    pub fn clear(&mut self) {
        let dirty = self.is_dirty();
        let revision = self.revision + 1;
        let limits = self.limits;
        let merge_window = self.merge_window;
        *self = Self::new()
            .with_limits(limits)
            .with_merge_window(merge_window);
        self.revision = revision;
        if dirty {
            self.saved = None;
        }
    }

    /// a serialisable picture of the whole tree for the history panel
    pub fn view(&self) -> HistoryView {
        let mut active: HashSet<NodeId> = self.line_to(self.head).into_iter().collect();
        let mut cursor = self.head;
        while let Some(next) = self.redo_target_of(cursor) {
            active.insert(next);
            cursor = next;
        }

        let mut nodes: Vec<HistoryNodeView> = self
            .nodes
            .iter()
            .map(|(id, node)| HistoryNodeView {
                id: *id,
                parent: node.parent,
                name: node.name.clone(),
                command_id: node.command.as_ref().map(|command| command.id().to_owned()),
                created_ms: node.created_ms,
                active: active.contains(id),
            })
            .collect();
        nodes.sort_by_key(|node| node.id);

        HistoryView {
            revision: self.revision,
            root: self.root,
            head: self.head,
            saved: self.saved,
            dirty: self.is_dirty(),
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            undo_name: self.undo_name().map(str::to_owned),
            redo_name: self.redo_name().map(str::to_owned),
            nodes,
            snapshots: self
                .snapshots
                .iter()
                .map(|snapshot| SnapshotView {
                    id: snapshot.id,
                    name: snapshot.name.clone(),
                    node: snapshot.node,
                    created_ms: snapshot.created_ms,
                })
                .collect(),
        }
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node<T> {
        self.nodes
            .get_mut(&id)
            .expect("node ids are only kept for nodes that still exist")
    }

    fn tick(&mut self) -> u64 {
        self.ticks += 1;
        self.ticks
    }

    fn head_accepts_merge(&self, now: Instant) -> bool {
        if self.sealed || self.head == self.root {
            return false;
        }
        match (self.merge_window, self.last_execute) {
            (Some(window), Some(last)) => now.saturating_duration_since(last) <= window,
            _ => true,
        }
    }

    fn push(&mut self, command: BoxedCommand<T>) -> NodeId {
        let id = NodeId(self.next_node);
        self.next_node += 1;
        let cost = command.cost();
        let node = Node {
            parent: Some(self.head),
            children: Vec::new(),
            active_child: None,
            name: command.name(),
            command: Some(command),
            created_ms: unix_ms(),
            visited: self.tick(),
            cost,
        };
        self.nodes.insert(id, node);

        let parent = self.node_mut(self.head);
        parent.children.push(id);
        parent.active_child = Some(id);

        self.head = id;
        self.sealed = false;
        self.bytes += cost;
        self.revision += 1;
        id
    }

    /// removes the head after its command was cancelled out by a merge.
    /// only an unsealed head gets here, and that is always a leaf nothing points at
    fn drop_head(&mut self) {
        let head = self.head;
        let Some(node) = self.nodes.remove(&head) else {
            return;
        };
        let parent = node.parent.unwrap_or(self.root);
        self.bytes -= node.cost;
        self.detach_child(parent, head);
        self.move_head(parent);
    }

    fn detach_child(&mut self, parent: NodeId, child: NodeId) {
        let parent = self.node_mut(parent);
        parent.children.retain(|id| *id != child);
        if parent.active_child == Some(child) {
            parent.active_child = parent.children.last().copied();
        }
    }

    fn move_head(&mut self, to: NodeId) {
        let tick = self.tick();
        self.node_mut(to).visited = tick;
        self.head = to;
        self.sealed = true;
        self.revision += 1;
    }

    fn step_into(&mut self, target: &mut T, child: NodeId) -> Result<(), HistoryError> {
        let node = self.node_mut(child);
        let parent = node.parent;
        if let Some(command) = node.command.as_mut() {
            command.redo(target)?;
        }
        if let Some(parent) = parent {
            self.node_mut(parent).active_child = Some(child);
        }
        self.move_head(child);
        Ok(())
    }

    fn redo_target(&self) -> Option<NodeId> {
        self.redo_target_of(self.head)
    }

    fn redo_target_of(&self, id: NodeId) -> Option<NodeId> {
        let node = &self.nodes[&id];
        node.active_child.or_else(|| node.children.last().copied())
    }

    /// node ids from the root down to `node`, both included
    fn line_to(&self, node: NodeId) -> Vec<NodeId> {
        let mut line = vec![node];
        let mut cursor = node;
        while let Some(parent) = self.nodes[&cursor].parent {
            line.push(parent);
            cursor = parent;
        }
        line.reverse();
        line
    }

    fn find_snapshot(&self, id: SnapshotId) -> Result<&Snapshot, HistoryError> {
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.id == id)
            .ok_or(HistoryError::UnknownSnapshot(id))
    }

    fn is_pinned(&self, node: NodeId) -> bool {
        self.snapshots.iter().any(|snapshot| snapshot.node == node)
    }

    fn over_limits(&self) -> bool {
        self.len() > self.limits.max_entries || self.bytes > self.limits.max_bytes
    }

    /// brings the tree back within its limits. branch tips the user has not been
    /// near for the longest go first. once only the active line is left, the
    /// oldest step falls off the bottom, like a plain undo stack would do
    fn prune(&mut self) {
        while self.over_limits() {
            if let Some(leaf) = self.stalest_leaf() {
                self.remove_leaf(leaf);
            } else if !self.advance_root() {
                break;
            }
        }
    }

    fn stalest_leaf(&self) -> Option<NodeId> {
        self.nodes
            .iter()
            .filter(|(id, node)| {
                node.children.is_empty()
                    && **id != self.head
                    && **id != self.root
                    && !self.is_pinned(**id)
            })
            .min_by_key(|(_, node)| node.visited)
            .map(|(id, _)| *id)
    }

    fn remove_leaf(&mut self, leaf: NodeId) {
        let Some(node) = self.nodes.remove(&leaf) else {
            return;
        };
        self.bytes -= node.cost;
        if let Some(parent) = node.parent {
            self.detach_child(parent, leaf);
        }
        if self.saved == Some(leaf) {
            self.saved = None;
        }
        self.revision += 1;
    }

    /// makes the root's only child the new root, which forgets the oldest step
    fn advance_root(&mut self) -> bool {
        let root = &self.nodes[&self.root];
        if self.root == self.head || root.children.len() != 1 || self.is_pinned(self.root) {
            return false;
        }
        let next = root.children[0];
        self.nodes.remove(&self.root);
        if self.saved == Some(self.root) {
            self.saved = None;
        }

        let node = self.node_mut(next);
        node.parent = None;
        node.command = None;
        let freed = std::mem::take(&mut node.cost);
        self.bytes -= freed;
        self.root = next;
        self.revision += 1;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryView {
    pub revision: u64,
    pub root: NodeId,
    pub head: NodeId,
    pub saved: Option<NodeId>,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_name: Option<String>,
    pub redo_name: Option<String>,
    /// every node, oldest first
    pub nodes: Vec<HistoryNodeView>,
    pub snapshots: Vec<SnapshotView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryNodeView {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub name: String,
    /// none for the root
    pub command_id: Option<String>,
    pub created_ms: u64,
    /// on the line that undo and redo walk along
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SnapshotView {
    pub id: SnapshotId,
    pub name: String,
    pub node: NodeId,
    pub created_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::testing::*;
    use crate::command::{Batch, Command, CommandResult};

    fn history() -> History<Doc> {
        // most tests want every command to be its own step
        History::new().with_merge_window(None)
    }

    fn typed(history: &mut History<Doc>, doc: &mut Doc, text: &str) -> NodeId {
        history.seal();
        match history.execute(doc, Append::new(text)).unwrap() {
            Executed::Pushed(id) => id,
            other => panic!("expected a new step, got {other:?}"),
        }
    }

    #[test]
    fn undo_and_redo_walk_the_line() {
        let (mut h, mut doc) = (history(), Doc::default());
        typed(&mut h, &mut doc, "a");
        typed(&mut h, &mut doc, "b");
        assert_eq!(doc.text, "ab");
        assert_eq!(h.undo_name(), Some("Type"));

        h.undo(&mut doc).unwrap();
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.text, "");
        assert!(!h.can_undo());
        assert!(matches!(h.undo(&mut doc), Err(HistoryError::NothingToUndo)));

        h.redo(&mut doc).unwrap();
        h.redo(&mut doc).unwrap();
        assert_eq!(doc.text, "ab");
        assert!(matches!(h.redo(&mut doc), Err(HistoryError::NothingToRedo)));
    }

    #[test]
    fn a_failed_command_is_not_recorded() {
        let (mut h, mut doc) = (history(), Doc::default());
        assert!(h.execute(&mut doc, SetOpacity::new(500)).is_err());
        assert!(h.is_empty());
        assert!(!h.is_dirty());
    }

    #[test]
    fn slider_drags_merge_into_one_step() {
        let (mut h, mut doc) = (
            history(),
            Doc {
                opacity: 100,
                ..Doc::default()
            },
        );
        assert!(matches!(
            h.execute(&mut doc, SetOpacity::new(90)).unwrap(),
            Executed::Pushed(_)
        ));
        for value in [80, 70, 60] {
            let merged = h.execute(&mut doc, SetOpacity::new(value)).unwrap();
            assert!(matches!(merged, Executed::Merged(_)));
        }
        assert_eq!(h.len(), 1);
        assert_eq!(h.undo_name(), Some("Set opacity to 60%"));

        h.undo(&mut doc).unwrap();
        assert_eq!(doc.opacity, 100);
        h.redo(&mut doc).unwrap();
        assert_eq!(doc.opacity, 60);
    }

    #[test]
    fn sealing_starts_a_new_step() {
        let (mut h, mut doc) = (history(), Doc::default());
        h.execute(&mut doc, SetOpacity::new(10)).unwrap();
        h.seal();
        h.execute(&mut doc, SetOpacity::new(20)).unwrap();
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn commands_outside_the_merge_window_stay_separate() {
        let mut h = History::new().with_merge_window(Some(Duration::from_millis(500)));
        let mut doc = Doc::default();
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);

        h.execute_at(&mut doc, SetOpacity::new(10).into_command(), at(0))
            .unwrap();
        h.execute_at(&mut doc, SetOpacity::new(20).into_command(), at(400))
            .unwrap();
        assert_eq!(h.len(), 1);
        h.execute_at(&mut doc, SetOpacity::new(30).into_command(), at(1000))
            .unwrap();
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn a_drag_back_to_the_start_leaves_no_step() {
        let (mut h, mut doc) = (
            history(),
            Doc {
                opacity: 50,
                ..Doc::default()
            },
        );
        h.execute(&mut doc, SetOpacity::new(60)).unwrap();
        let annulled = h.execute(&mut doc, SetOpacity::new(50)).unwrap();
        assert_eq!(annulled, Executed::Annulled);
        assert!(h.is_empty());
        assert!(!h.is_dirty());
        assert_eq!(doc.opacity, 50);
    }

    #[test]
    fn undo_and_redo_stop_merging() {
        let (mut h, mut doc) = (history(), Doc::default());
        h.execute(&mut doc, SetOpacity::new(10)).unwrap();
        h.undo(&mut doc).unwrap();
        h.redo(&mut doc).unwrap();
        h.execute(&mut doc, SetOpacity::new(20)).unwrap();
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn one_way_commands_run_but_leave_no_step() {
        let (mut h, mut doc) = (history(), Doc::default());
        assert_eq!(h.execute(&mut doc, Export).unwrap(), Executed::Transient);
        assert_eq!(doc.exports, 1);
        assert!(h.is_empty());
        assert!(!h.is_dirty());
    }

    #[test]
    fn a_new_edit_after_undo_branches_instead_of_discarding() {
        let (mut h, mut doc) = (history(), Doc::default());
        typed(&mut h, &mut doc, "a");
        let b = typed(&mut h, &mut doc, "b");
        h.undo(&mut doc).unwrap();
        let c = typed(&mut h, &mut doc, "c");
        assert_eq!(doc.text, "ac");
        assert_eq!(h.len(), 3);

        // the old branch is still there and reachable
        h.go_to(&mut doc, b).unwrap();
        assert_eq!(doc.text, "ab");
        h.go_to(&mut doc, c).unwrap();
        assert_eq!(doc.text, "ac");
    }

    #[test]
    fn redo_follows_the_branch_used_last() {
        let (mut h, mut doc) = (history(), Doc::default());
        let a = typed(&mut h, &mut doc, "a");
        let b = typed(&mut h, &mut doc, "b");
        h.undo(&mut doc).unwrap();
        typed(&mut h, &mut doc, "c");

        h.undo(&mut doc).unwrap();
        h.redo(&mut doc).unwrap();
        assert_eq!(doc.text, "ac");

        h.go_to(&mut doc, b).unwrap();
        h.go_to(&mut doc, a).unwrap();
        h.redo(&mut doc).unwrap();
        assert_eq!(doc.text, "ab");
    }

    #[test]
    fn go_to_crosses_branches_in_both_directions() {
        let (mut h, mut doc) = (history(), Doc::default());
        let root = h.head();
        typed(&mut h, &mut doc, "a");
        typed(&mut h, &mut doc, "b");
        let deep = typed(&mut h, &mut doc, "c");
        h.go_to(&mut doc, root).unwrap();
        assert_eq!(doc.text, "");

        typed(&mut h, &mut doc, "x");
        let other = typed(&mut h, &mut doc, "y");
        h.go_to(&mut doc, deep).unwrap();
        assert_eq!(doc.text, "abc");
        h.go_to(&mut doc, other).unwrap();
        assert_eq!(doc.text, "xy");
        h.go_to(&mut doc, other).unwrap();
        assert_eq!(doc.text, "xy");

        assert!(matches!(
            h.go_to(&mut doc, NodeId(999)),
            Err(HistoryError::UnknownNode(_))
        ));
    }

    #[test]
    fn snapshots_restore_across_branches() {
        let (mut h, mut doc) = (history(), Doc::default());
        typed(&mut h, &mut doc, "draft");
        let draft = h.snapshot("First draft");
        typed(&mut h, &mut doc, " one");
        h.undo(&mut doc).unwrap();
        typed(&mut h, &mut doc, " two");
        let second = h.snapshot("Second try");

        h.restore_snapshot(&mut doc, draft).unwrap();
        assert_eq!(doc.text, "draft");
        h.restore_snapshot(&mut doc, second).unwrap();
        assert_eq!(doc.text, "draft two");

        h.rename_snapshot(draft, "Keep this").unwrap();
        let view = h.view();
        let names: Vec<_> = view.snapshots.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Keep this", "Second try"]);

        h.delete_snapshot(draft).unwrap();
        assert!(matches!(
            h.restore_snapshot(&mut doc, draft),
            Err(HistoryError::UnknownSnapshot(_))
        ));
    }

    #[test]
    fn a_snapshot_ends_the_merge_run() {
        let (mut h, mut doc) = (history(), Doc::default());
        h.execute(&mut doc, SetOpacity::new(10)).unwrap();
        let before = h.snapshot("Ten");
        h.execute(&mut doc, SetOpacity::new(20)).unwrap();
        h.restore_snapshot(&mut doc, before).unwrap();
        assert_eq!(doc.opacity, 10);
    }

    #[test]
    fn dirty_tracks_distance_from_the_saved_state() {
        let (mut h, mut doc) = (history(), Doc::default());
        assert!(!h.is_dirty());
        typed(&mut h, &mut doc, "a");
        assert!(h.is_dirty());
        h.mark_saved();
        assert!(!h.is_dirty());

        typed(&mut h, &mut doc, "b");
        assert!(h.is_dirty());
        h.undo(&mut doc).unwrap();
        assert!(!h.is_dirty());
        h.undo(&mut doc).unwrap();
        assert!(h.is_dirty());
    }

    #[test]
    fn saving_ends_the_merge_run_so_later_drags_count_as_changes() {
        let (mut h, mut doc) = (history(), Doc::default());
        h.execute(&mut doc, SetOpacity::new(10)).unwrap();
        h.mark_saved();
        h.execute(&mut doc, SetOpacity::new(20)).unwrap();
        assert!(h.is_dirty());
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn the_entry_limit_drops_the_oldest_steps_of_a_single_line() {
        let mut h = history().with_limits(HistoryLimits {
            max_entries: 3,
            max_bytes: usize::MAX,
        });
        let mut doc = Doc::default();
        for letter in ["a", "b", "c", "d", "e"] {
            typed(&mut h, &mut doc, letter);
        }
        assert_eq!(h.len(), 3);

        while h.can_undo() {
            h.undo(&mut doc).unwrap();
        }
        // the two oldest steps can no longer be undone
        assert_eq!(doc.text, "ab");
        while h.can_redo() {
            h.redo(&mut doc).unwrap();
        }
        assert_eq!(doc.text, "abcde");
    }

    #[test]
    fn stale_branches_are_pruned_before_the_active_line() {
        let mut h = history().with_limits(HistoryLimits {
            max_entries: 4,
            max_bytes: usize::MAX,
        });
        let mut doc = Doc::default();
        let a = typed(&mut h, &mut doc, "a");
        let old_tip = typed(&mut h, &mut doc, "b");
        h.go_to(&mut doc, a).unwrap();
        typed(&mut h, &mut doc, "c");
        typed(&mut h, &mut doc, "d");
        assert_eq!(h.len(), 4);

        typed(&mut h, &mut doc, "e");
        assert_eq!(h.len(), 4);
        assert!(matches!(
            h.go_to(&mut doc, old_tip),
            Err(HistoryError::UnknownNode(_))
        ));

        // the whole active line survived
        h.go_to(&mut doc, a).unwrap();
        assert_eq!(doc.text, "a");
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.text, "");
    }

    #[test]
    fn snapshots_are_never_pruned() {
        let mut h = history().with_limits(HistoryLimits {
            max_entries: 2,
            max_bytes: usize::MAX,
        });
        let mut doc = Doc::default();
        typed(&mut h, &mut doc, "a");
        let keep = h.snapshot("Keep");
        let root = h.view().root;
        h.go_to(&mut doc, root).unwrap();
        for letter in ["x", "y", "z"] {
            typed(&mut h, &mut doc, letter);
        }

        h.restore_snapshot(&mut doc, keep).unwrap();
        assert_eq!(doc.text, "a");
    }

    #[test]
    fn the_byte_budget_counts_command_cost() {
        let mut h = history().with_limits(HistoryLimits {
            max_entries: usize::MAX,
            max_bytes: 10,
        });
        let mut doc = Doc::default();
        typed(&mut h, &mut doc, "aaaa");
        typed(&mut h, &mut doc, "bbbb");
        assert_eq!(h.len(), 2);
        typed(&mut h, &mut doc, "cccc");
        assert_eq!(h.len(), 2);

        h.undo(&mut doc).unwrap();
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.text, "aaaa");
        assert!(!h.can_undo());
    }

    #[test]
    fn losing_the_saved_state_to_pruning_keeps_the_document_dirty() {
        let mut h = history().with_limits(HistoryLimits {
            max_entries: 1,
            max_bytes: usize::MAX,
        });
        let mut doc = Doc::default();
        typed(&mut h, &mut doc, "a");
        typed(&mut h, &mut doc, "b");
        h.undo(&mut doc).unwrap();
        assert!(h.is_dirty());
    }

    #[test]
    fn push_applied_records_without_running() {
        let (mut h, mut doc) = (history(), Doc::default());
        // the stroke already painted itself into the document
        doc.text.push_str("stroke");
        h.push_applied(Append::new("stroke"));
        assert_eq!(doc.text, "stroke");

        h.undo(&mut doc).unwrap();
        assert_eq!(doc.text, "");
        h.redo(&mut doc).unwrap();
        assert_eq!(doc.text, "stroke");
    }

    #[test]
    fn a_batch_is_one_undo_step() {
        let (mut h, mut doc) = (history(), Doc::default());
        let batch = Batch::new("Style")
            .with(SetOpacity::new(30))
            .with(Append::new("hi"));
        h.execute(&mut doc, batch).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h.undo_name(), Some("Style"));
        h.undo(&mut doc).unwrap();
        assert_eq!(doc, Doc::default());
    }

    #[test]
    fn records_since_gives_a_macro_its_steps() {
        let (mut h, mut doc) = (history(), Doc::default());
        typed(&mut h, &mut doc, "before");
        let start = h.head();
        typed(&mut h, &mut doc, "a");
        h.seal();
        h.execute(&mut doc, SetOpacity::new(10)).unwrap();
        h.execute(&mut doc, SetOpacity::new(20)).unwrap();
        typed(&mut h, &mut doc, "oops");
        h.undo(&mut doc).unwrap();

        let records = h.records_since(start).unwrap();
        let ids: Vec<_> = records.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(ids, ["test.append", "test.set_opacity"]);
        assert_eq!(records[1].params["value"], 20);

        // replaying the records on a fresh document gives the same result
        let registry = registry();
        let mut replay = Doc {
            text: "before".into(),
            ..Doc::default()
        };
        for record in &records {
            registry.build(record).unwrap().apply(&mut replay).unwrap();
        }
        assert_eq!(replay, doc);
    }

    #[test]
    fn the_view_describes_the_tree() {
        let (mut h, mut doc) = (history(), Doc::default());
        let a = typed(&mut h, &mut doc, "a");
        let b = typed(&mut h, &mut doc, "b");
        h.undo(&mut doc).unwrap();
        let c = typed(&mut h, &mut doc, "c");
        h.undo(&mut doc).unwrap();

        let view = h.view();
        assert_eq!(view.head, a);
        assert_eq!(view.nodes.len(), 4);
        assert!(view.can_undo && view.can_redo);
        assert_eq!(view.redo_name.as_deref(), Some("Type"));

        let node = |id| view.nodes.iter().find(|node| node.id == id).unwrap();
        assert_eq!(node(view.root).command_id, None);
        assert_eq!(node(b).parent, Some(a));
        assert_eq!(node(c).command_id.as_deref(), Some("test.append"));
        // redo would go to c, so b is off the active line
        assert!(node(c).active && !node(b).active && node(a).active);
    }

    #[test]
    fn clear_forgets_steps_but_remembers_unsaved_changes() {
        let (mut h, mut doc) = (history(), Doc::default());
        typed(&mut h, &mut doc, "a");
        let revision = h.revision();
        h.clear();
        assert!(h.is_empty() && !h.can_undo());
        assert!(h.is_dirty());
        assert!(h.revision() > revision);
    }

    #[derive(Debug, Default, Clone, Serialize, Deserialize)]
    struct FailingUndo;

    impl Command<Doc> for FailingUndo {
        const ID: &'static str = "test.failing_undo";
        const TITLE: &'static str = "Break";

        fn apply(&mut self, doc: &mut Doc) -> CommandResult {
            doc.text.push('!');
            Ok(())
        }

        fn undo(&mut self, _doc: &mut Doc) -> CommandResult {
            Err(CommandError::failed("cannot undo"))
        }
    }

    #[test]
    fn a_failed_undo_keeps_the_head_where_it_was() {
        let (mut h, mut doc) = (history(), Doc::default());
        let root = h.head();
        typed(&mut h, &mut doc, "a");
        h.execute(&mut doc, FailingUndo).unwrap();
        let stuck = h.head();
        typed(&mut h, &mut doc, "b");

        assert!(h.go_to(&mut doc, root).is_err());
        assert_eq!(h.head(), stuck);
        assert_eq!(doc.text, "a!");
    }
}
