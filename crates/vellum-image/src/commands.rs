//! every change to an image document is a command, so it undoes, redoes, autosaves and
//! records into macros like everything else in vellum.
//!
//! each command describes the user's intent (its parameters) and does its work in
//! [`Edit::run`]. the [`Cmd`] wrapper keeps the document as it was before and after the
//! first run. that is a cheap copy, because rasters share their tiles, and it makes undo
//! and redo exact for every command without each one writing its own undo code. the
//! history's memory budget sees only the tiles a step really changed.

use std::collections::HashSet;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use vellum_core::command::{
    Command, CommandError, CommandInfo, CommandRegistry, downcast, params_of,
};
use vellum_core::documents::RenameDocument;
use vellum_gpu::{BlendMode, IRect};

use crate::compose::Part;
use crate::document::{
    Content, EditTarget, ImageDocument, Label, Layer, LayerId, LayerMask, Locks, SavedChannel,
};
use crate::flatten;
use crate::paint::{self, BrushSettings, FillSettings, GradientSettings, Stroke, StrokePoint};
use crate::raster::{Depth, Raster};
use crate::selection::{self, ColorBuf, Combine, EdgeMap, Shape};

/// one kind of change to an image document
pub trait Edit: Serialize + DeserializeOwned + Clone + Send + 'static {
    const KIND: &'static str;
    const TITLE: &'static str;
    const DESCRIPTION: &'static str;

    /// what the history panel shows
    fn name(&self) -> String {
        Self::TITLE.to_owned()
    }

    /// makes the change. on an error the document is put back as it was.
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String>;

    /// true if `next` continues this edit, like the same slider still moving
    fn continues(&self, next: &Self) -> bool {
        let _ = next;
        false
    }

    fn undoable(&self) -> bool {
        true
    }
}

struct Memo {
    before: ImageDocument,
    after: ImageDocument,
    cost: usize,
}

/// an [`Edit`] as a vellum command
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cmd<E> {
    pub edit: E,
    #[serde(skip)]
    memo: Option<Memo>,
}

impl<E: Edit> Cmd<E> {
    pub fn new(edit: E) -> Self {
        Self { edit, memo: None }
    }
}

/// bytes of tile data in `after` that `before` does not share
fn changed_bytes(before: &ImageDocument, after: &ImageDocument) -> usize {
    fn rasters(doc: &ImageDocument) -> Vec<&Raster> {
        let mut out = Vec::new();
        for l in doc.all_layers() {
            out.extend(l.raster());
            out.extend(l.mask.as_ref().map(|m| &m.raster));
        }
        out.extend(doc.selection.as_ref());
        out.extend(doc.channels.iter().map(|c| &c.raster));
        out
    }
    let old: HashSet<usize> = rasters(before)
        .iter()
        .flat_map(|r| r.tiles().map(|(_, t)| Arc::as_ptr(&t.data) as usize))
        .collect();
    rasters(after)
        .iter()
        .flat_map(|r| r.tiles())
        .filter(|(_, t)| !old.contains(&(Arc::as_ptr(&t.data) as usize)))
        .map(|(_, t)| t.data.bytes())
        .sum()
}

/// puts a kept state back. the title is left alone: renaming is its own command, and a
/// state kept before a rename must not undo it.
fn restore(doc: &mut ImageDocument, kept: &ImageDocument) {
    let title = std::mem::take(&mut doc.title);
    *doc = kept.clone();
    doc.title = title;
}

impl<E: Edit> Command<ImageDocument> for Cmd<E> {
    fn kind(&self) -> &'static str {
        E::KIND
    }

    fn name(&self) -> String {
        self.edit.name()
    }

    fn apply(&mut self, doc: &mut ImageDocument) -> Result<(), CommandError> {
        if let Some(memo) = &self.memo {
            restore(doc, &memo.after);
            return Ok(());
        }
        if doc.loading.is_some() {
            return Err(CommandError::failed(
                "The picture is still opening. Try again in a moment.",
            ));
        }
        let before = doc.clone();
        if let Err(err) = self.edit.run(doc) {
            *doc = before;
            return Err(CommandError::failed(err));
        }
        let cost = changed_bytes(&before, doc);
        self.memo = Some(Memo {
            before,
            after: doc.clone(),
            cost,
        });
        Ok(())
    }

    fn revert(&mut self, doc: &mut ImageDocument) -> Result<(), CommandError> {
        let memo = self
            .memo
            .as_ref()
            .ok_or_else(|| CommandError::failed("nothing to undo"))?;
        restore(doc, &memo.before);
        Ok(())
    }

    fn merge(&mut self, next: &dyn Command<ImageDocument>) -> bool {
        let Some(next) = downcast::<ImageDocument, Cmd<E>>(next) else {
            return false;
        };
        if !self.edit.continues(&next.edit) {
            return false;
        }
        let (Some(mine), Some(theirs)) = (&mut self.memo, &next.memo) else {
            return false;
        };
        mine.after = theirs.after.clone();
        mine.cost = changed_bytes(&mine.before, &mine.after);
        self.edit = next.edit.clone();
        true
    }

    fn params(&self) -> Result<serde_json::Value, CommandError> {
        params_of(&self.edit)
    }

    fn undoable(&self) -> bool {
        self.edit.undoable()
    }

    fn memory_cost(&self) -> usize {
        // the two documents' structure, plus the pixels this step owns
        4096 + self.memo.as_ref().map_or(0, |m| m.cost)
    }
}

fn layer_mut(doc: &mut ImageDocument, id: LayerId) -> Result<&mut Layer, String> {
    doc.layer_mut(id)
        .ok_or_else(|| "That layer is gone.".to_owned())
}

fn active(doc: &ImageDocument) -> Result<LayerId, String> {
    doc.active.ok_or_else(|| "Pick a layer first.".to_owned())
}

fn unlocked(layer: &Layer) -> Result<(), String> {
    if layer.locks.all {
        return Err(format!(
            "{} is locked. Unlock it in the Layers panel first.",
            layer.name
        ));
    }
    Ok(())
}

/// the colours tools pick from: the active layer, or every layer when `all`
fn sample_source(doc: &ImageDocument, all: bool) -> Result<ColorBuf, String> {
    let canvas = doc.canvas();
    if all {
        return flatten::colors(doc, canvas);
    }
    let layer = doc.active_layer().ok_or("Pick a layer first.")?;
    match layer.raster() {
        Some(r) => Ok(ColorBuf::from_raster(r, canvas)),
        // a group shows what its children make
        None => {
            let mut only = doc.clone();
            only.layers = vec![layer.clone()];
            only.quick_mask = false;
            flatten::colors(&only, canvas)
        }
    }
}

/// replaces the selection, remembering the old one for "reselect"
fn set_selection(doc: &mut ImageDocument, next: Option<Raster>) {
    if let Some(old) = doc.selection.take() {
        doc.last_selection = Some(old);
    }
    doc.selection = next;
}

/// adds an empty layer or group above the active layer
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AddLayer {
    pub group: bool,
    pub name: Option<String>,
}

impl Edit for AddLayer {
    const KIND: &'static str = "image.layer.add";
    const TITLE: &'static str = "New layer";
    const DESCRIPTION: &'static str = "Add an empty layer or group above the current layer.";
    fn name(&self) -> String {
        if self.group {
            "New group".into()
        } else {
            "New layer".into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = doc.new_id();
        let name = self
            .name
            .clone()
            .unwrap_or_else(|| doc.fresh_name(if self.group { "Group" } else { "Layer" }));
        let layer = if self.group {
            Layer::group(id, name, Vec::new())
        } else {
            Layer::pixels(id, name, Raster::rgba(doc.depth))
        };
        let (parent, index) = match doc.active.and_then(|a| doc.locate(a)) {
            Some((parent, i)) => (parent, i + 1),
            None => (None, doc.layers.len()),
        };
        doc.insert(parent, index, layer);
        doc.active = Some(id);
        doc.target = EditTarget::Pixels;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoveLayers {
    pub ids: Vec<LayerId>,
}

impl Edit for RemoveLayers {
    const KIND: &'static str = "image.layer.remove";
    const TITLE: &'static str = "Delete layer";
    const DESCRIPTION: &'static str = "Remove layers.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let ids = if self.ids.is_empty() {
            vec![active(doc)?]
        } else {
            self.ids.clone()
        };
        for id in &ids {
            if let Some(l) = doc.layer(*id) {
                unlocked(l)?;
            }
        }
        let next_active = ids
            .first()
            .and_then(|id| doc.locate(*id))
            .and_then(|(parent, i)| {
                let list = doc.siblings(parent)?;
                list.iter()
                    .enumerate()
                    .filter(|(j, l)| *j != i && !ids.contains(&l.id))
                    .min_by_key(|(j, _)| (*j as i64 - i as i64).abs())
                    .map(|(_, l)| l.id)
                    .or(parent)
            });
        for id in ids {
            doc.remove(id);
        }
        doc.active = next_active
            .filter(|id| doc.layer(*id).is_some())
            .or_else(|| doc.layers.last().map(|l| l.id));
        doc.target = EditTarget::Pixels;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DuplicateLayer {
    pub id: Option<LayerId>,
}

fn duplicate(doc: &mut ImageDocument, layer: &Layer) -> Layer {
    let mut copy = layer.clone();
    copy.id = doc.new_id();
    if let Content::Pixels(r) = &mut copy.content {
        *r = r.duplicate();
    }
    if let Some(m) = &mut copy.mask {
        m.raster = m.raster.duplicate();
    }
    if let Content::Group { children, .. } = &mut copy.content {
        let kids = std::mem::take(children);
        *children = kids.iter().map(|c| duplicate(doc, c)).collect();
    }
    copy
}

impl Edit for DuplicateLayer {
    const KIND: &'static str = "image.layer.duplicate";
    const TITLE: &'static str = "Duplicate layer";
    const DESCRIPTION: &'static str = "Copy a layer.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.map_or_else(|| active(doc), Ok)?;
        let layer = doc.layer(id).ok_or("That layer is gone.")?.clone();
        let (parent, index) = doc.locate(id).ok_or("That layer is gone.")?;
        let mut copy = duplicate(doc, &layer);
        copy.name = format!("{} copy", layer.name);
        let new_id = copy.id;
        doc.insert(parent, index + 1, copy);
        doc.active = Some(new_id);
        Ok(())
    }
}

/// changes any of a layer's settings. a slider drag sends many and they merge into one
/// step.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LayerProps {
    pub id: Option<LayerId>,
    pub name: Option<String>,
    pub visible: Option<bool>,
    pub opacity: Option<f32>,
    pub fill: Option<f32>,
    pub blend: Option<BlendMode>,
    pub pass_through: Option<bool>,
    pub locks: Option<Locks>,
    pub label: Option<Label>,
    pub clipped: Option<bool>,
    pub expanded: Option<bool>,
    pub mask_enabled: Option<bool>,
    pub mask_linked: Option<bool>,
}

impl LayerProps {
    fn fields(&self) -> [bool; 12] {
        [
            self.name.is_some(),
            self.visible.is_some(),
            self.opacity.is_some(),
            self.fill.is_some(),
            self.blend.is_some(),
            self.pass_through.is_some(),
            self.locks.is_some(),
            self.label.is_some(),
            self.clipped.is_some(),
            self.expanded.is_some(),
            self.mask_enabled.is_some(),
            self.mask_linked.is_some(),
        ]
    }
}

impl Edit for LayerProps {
    const KIND: &'static str = "image.layer.props";
    const TITLE: &'static str = "Layer settings";
    const DESCRIPTION: &'static str =
        "Change a layer's name, visibility, opacity, fill, blend mode, locks or colour.";
    fn name(&self) -> String {
        match () {
            _ if self.name.is_some() => "Rename layer",
            _ if self.visible == Some(false) => "Hide layer",
            _ if self.visible == Some(true) => "Show layer",
            _ if self.opacity.is_some() => "Layer opacity",
            _ if self.fill.is_some() => "Layer fill",
            _ if self.blend.is_some() || self.pass_through.is_some() => "Blend mode",
            _ if self.locks.is_some() => "Lock layer",
            _ if self.label.is_some() => "Colour label",
            _ if self.clipped == Some(true) => "Clipping mask",
            _ if self.clipped == Some(false) => "Release clipping mask",
            _ if self.mask_enabled.is_some() => "Turn mask on or off",
            _ if self.mask_linked.is_some() => "Link mask",
            _ => "Layer settings",
        }
        .to_owned()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.map_or_else(|| active(doc), Ok)?;
        let layer = layer_mut(doc, id)?;
        // a locked layer can still be shown, hidden, unlocked and tagged
        let structural = self.name.is_some()
            || self.opacity.is_some()
            || self.fill.is_some()
            || self.blend.is_some()
            || self.clipped.is_some();
        if structural && self.locks.is_none() {
            unlocked(layer)?;
        }
        if let Some(name) = &self.name {
            let name = name.trim();
            if name.is_empty() {
                return Err("A layer needs a name.".into());
            }
            layer.name = name.to_owned();
        }
        if let Some(v) = self.visible {
            layer.visible = v;
        }
        if let Some(v) = self.opacity {
            layer.opacity = v.clamp(0.0, 1.0);
        }
        if let Some(v) = self.fill {
            layer.fill = v.clamp(0.0, 1.0);
        }
        if let Some(v) = self.blend {
            layer.blend = v;
            if let Content::Group { pass_through, .. } = &mut layer.content {
                *pass_through = false;
            }
        }
        if let Some(v) = self.pass_through
            && let Content::Group { pass_through, .. } = &mut layer.content
        {
            *pass_through = v;
            if v {
                layer.blend = BlendMode::Normal;
            }
        }
        if let Some(v) = self.locks {
            layer.locks = v;
        }
        if let Some(v) = self.label {
            layer.label = v;
        }
        if let Some(v) = self.clipped {
            layer.clipped = v;
        }
        if let Some(v) = self.expanded
            && let Content::Group { expanded, .. } = &mut layer.content
        {
            *expanded = v;
        }
        if let Some(m) = &mut layer.mask {
            if let Some(v) = self.mask_enabled {
                m.enabled = v;
            }
            if let Some(v) = self.mask_linked {
                m.linked = v;
            }
        }
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        self.id == next.id
            && self.fields() == next.fields()
            && (self.opacity.is_some() || self.fill.is_some())
    }
}

/// picks the layer tools work on, and whether its pixels or its mask. not a history step,
/// like in photoshop.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SelectLayer {
    pub id: Option<LayerId>,
    pub target: EditTarget,
}

impl Edit for SelectLayer {
    const KIND: &'static str = "image.layer.select";
    const TITLE: &'static str = "Select layer";
    const DESCRIPTION: &'static str = "Pick the layer tools work on.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if let Some(id) = self.id {
            let layer = doc.layer(id).ok_or("That layer is gone.")?;
            doc.target = if layer.mask.is_some() {
                self.target
            } else {
                EditTarget::Pixels
            };
        }
        doc.active = self.id;
        Ok(())
    }
    fn undoable(&self) -> bool {
        false
    }
}

/// moves a layer to another place in the stack, or into or out of a group
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MoveLayer {
    pub id: Option<LayerId>,
    /// the group to move into, none for the top level
    pub parent: Option<LayerId>,
    /// position in that list, counted from the bottom
    pub index: usize,
}

impl Edit for MoveLayer {
    const KIND: &'static str = "image.layer.move";
    const TITLE: &'static str = "Move layer";
    const DESCRIPTION: &'static str = "Change where a layer sits in the stack.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.map_or_else(|| active(doc), Ok)?;
        if let Some(p) = self.parent {
            if doc.is_inside(p, id) {
                return Err("A group cannot go inside itself.".into());
            }
            if !doc.layer(p).is_some_and(Layer::is_group) {
                return Err("Layers can only go inside groups.".into());
            }
        }
        let (from_parent, from_index) = doc.locate(id).ok_or("That layer is gone.")?;
        let layer = doc.remove(id).ok_or("That layer is gone.")?;
        // removing from the same list shifts everything above down by one
        let index = if from_parent == self.parent && from_index < self.index {
            self.index - 1
        } else {
            self.index
        };
        doc.insert(self.parent, index, layer);
        Ok(())
    }
}

/// puts layers in a new group where the top one was
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GroupLayers {
    pub ids: Vec<LayerId>,
}

impl Edit for GroupLayers {
    const KIND: &'static str = "image.layer.group";
    const TITLE: &'static str = "Group layers";
    const DESCRIPTION: &'static str = "Put layers in a new group.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let ids = if self.ids.is_empty() {
            vec![active(doc)?]
        } else {
            self.ids.clone()
        };
        let (parent, index) = ids
            .iter()
            .filter_map(|id| doc.locate(*id))
            .max_by_key(|(_, i)| *i)
            .ok_or("Those layers are gone.")?;
        // keep stacking order: collect in panel order from the bottom
        let mut members: Vec<(usize, Layer)> = Vec::new();
        for id in &ids {
            if let Some((p, i)) = doc.locate(*id)
                && p == parent
            {
                members.push((i, doc.layer(*id).cloned().ok_or("That layer is gone.")?));
            }
        }
        members.sort_by_key(|(i, _)| *i);
        let below = members.iter().filter(|(i, _)| *i < index).count();
        for (_, l) in &members {
            doc.remove(l.id);
        }
        let gid = doc.new_id();
        let name = doc.fresh_name("Group");
        let group = Layer::group(gid, name, members.into_iter().map(|(_, l)| l).collect());
        // the group takes the top member's place once the members below it are gone
        doc.insert(parent, index - below, group);
        doc.active = Some(gid);
        doc.target = EditTarget::Pixels;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Ungroup {
    pub id: Option<LayerId>,
}

impl Edit for Ungroup {
    const KIND: &'static str = "image.layer.ungroup";
    const TITLE: &'static str = "Ungroup layers";
    const DESCRIPTION: &'static str = "Take the layers out of a group and remove the group.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let mut id = self.id.map_or_else(|| active(doc), Ok)?;
        // ungrouping from inside a group means its parent
        if !doc.layer(id).is_some_and(Layer::is_group) {
            id = doc
                .locate(id)
                .and_then(|(p, _)| p)
                .ok_or("The layer is not in a group.")?;
        }
        let (parent, index) = doc.locate(id).ok_or("That layer is gone.")?;
        let group = doc.remove(id).ok_or("That layer is gone.")?;
        let first = group.children().last().map(|l| l.id);
        if let Content::Group { children, .. } = group.content {
            for (k, child) in children.into_iter().enumerate() {
                doc.insert(parent, index + k, child);
            }
        }
        doc.active = first;
        Ok(())
    }
}

/// merges a layer into the one below it, which keeps its name, blend mode and opacity
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MergeDown {
    pub id: Option<LayerId>,
}

impl Edit for MergeDown {
    const KIND: &'static str = "image.layer.merge_down";
    const TITLE: &'static str = "Merge down";
    const DESCRIPTION: &'static str = "Merge a layer into the one below.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.map_or_else(|| active(doc), Ok)?;
        let (parent, index) = doc.locate(id).ok_or("That layer is gone.")?;
        if index == 0 {
            return Err("There is no layer below to merge into.".into());
        }
        let list = doc.siblings(parent).ok_or("That layer is gone.")?;
        let (below, top) = (list[index - 1].clone(), list[index].clone());
        if below.is_group() {
            return Err("The layer below is a group. Merge into a layer instead.".into());
        }
        unlocked(&below)?;
        if below.locks.pixels_locked() {
            return Err(format!("{} has its pixels locked.", below.name));
        }
        let mut base = below.clone();
        base.blend = BlendMode::Normal;
        base.opacity = 1.0;
        base.fill = 1.0;
        base.clipped = false;
        let mut layer = top.clone();
        layer.visible = true;
        let mut only = doc.clone();
        only.layers = vec![base, layer];
        only.quick_mask = false;
        let rect = below.bounds().union(&top.bounds());
        let merged = flatten::merged(&only, rect, doc.depth)?;
        let target = layer_mut(doc, below.id)?;
        target.content = Content::Pixels(merged);
        target.mask = None;
        doc.remove(top.id);
        doc.active = Some(below.id);
        doc.target = EditTarget::Pixels;
        Ok(())
    }
}

/// merges every visible layer into one, or with `all` flattens the whole document
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MergeVisible {
    pub flatten: bool,
}

impl Edit for MergeVisible {
    const KIND: &'static str = "image.layer.merge_visible";
    const TITLE: &'static str = "Merge visible";
    const DESCRIPTION: &'static str = "Merge every visible layer into one, or flatten the picture.";
    fn name(&self) -> String {
        if self.flatten {
            "Flatten image".into()
        } else {
            "Merge visible".into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let visible: Vec<&Layer> = doc.layers.iter().filter(|l| l.visible).collect();
        if visible.is_empty() {
            return Err("No layer is visible.".into());
        }
        let rect = visible
            .iter()
            .fold(IRect::EMPTY, |a, l| a.union(&l.bounds()));
        let top = visible.last().map(|l| l.id);
        let mut only = doc.clone();
        only.quick_mask = false;
        let merged = flatten::merged(&only, rect, doc.depth)?;
        let id = doc.new_id();
        let name = if self.flatten {
            "Background".to_owned()
        } else {
            top.and_then(|t| doc.layer(t))
                .map_or_else(|| "Merged".into(), |l| l.name.clone())
        };
        let layer = Layer::pixels(id, name, merged);
        if self.flatten {
            doc.layers = vec![layer];
        } else {
            let at = doc
                .layers
                .iter()
                .position(|l| Some(l.id) == top)
                .unwrap_or(0);
            let keep_below = doc.layers[..at].iter().filter(|l| !l.visible).count();
            doc.layers.retain(|l| !l.visible);
            doc.layers.insert(keep_below, layer);
        }
        doc.active = Some(id);
        doc.target = EditTarget::Pixels;
        Ok(())
    }
}

/// a new layer from the selected pixels of the active layer, optionally cutting them out
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LayerViaCopy {
    pub cut: bool,
}

impl Edit for LayerViaCopy {
    const KIND: &'static str = "image.layer.via_copy";
    const TITLE: &'static str = "Layer via copy";
    const DESCRIPTION: &'static str = "Copy or cut the selected pixels to a new layer.";
    fn name(&self) -> String {
        if self.cut {
            "Layer via cut".into()
        } else {
            "Layer via copy".into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = active(doc)?;
        let layer = doc.layer(id).ok_or("Pick a layer first.")?;
        let source = layer
            .raster()
            .ok_or("Groups cannot be copied this way. Pick a layer.")?
            .clone();
        if self.cut && layer.locks.pixels_locked() {
            return Err(format!("{} has its pixels locked.", layer.name));
        }
        let sel = doc.selection.clone();
        let mut copy = source.duplicate();
        if let Some(s) = &sel {
            let area = copy.tile_extent();
            copy.update(area, |x, y, px| px[3] *= s.get(x, y, 0));
        }
        if copy.content_bounds().is_empty() {
            return Err("There are no pixels in the selection to copy.".into());
        }
        if self.cut {
            let mut rest = source;
            let area = rest.tile_extent();
            rest.update(area, |x, y, px| {
                px[3] *= 1.0 - sel.as_ref().map_or(1.0, |s| s.get(x, y, 0));
            });
            paint::set_part(doc, &Part::Pixels(id), rest);
        }
        let (parent, index) = doc.locate(id).ok_or("That layer is gone.")?;
        let new_id = doc.new_id();
        let name = doc.fresh_name("Layer");
        doc.insert(parent, index + 1, Layer::pixels(new_id, name, copy));
        doc.active = Some(new_id);
        doc.target = EditTarget::Pixels;
        Ok(())
    }
}

/// moves layers by whole pixels, the move tool's drag
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OffsetLayers {
    pub ids: Vec<LayerId>,
    pub dx: i32,
    pub dy: i32,
}

/// a raster shifted by whole pixels
pub fn shifted(r: &Raster, dx: i32, dy: i32) -> Raster {
    if dx == 0 && dy == 0 {
        return r.clone();
    }
    let mut out = r.empty_like();
    let ts = crate::raster::TILE as i32;
    if dx % ts == 0 && dy % ts == 0 {
        // whole tiles just change places, no pixels move
        for (&(tx, ty), t) in r.tiles() {
            out.set_tile(tx + dx / ts, ty + dy / ts, Some(t.data.clone()));
        }
        return out;
    }
    let src = r.tile_extent();
    let values = r.read(src);
    out.write(src.translate(dx, dy), &values);
    out
}

impl Edit for OffsetLayers {
    const KIND: &'static str = "image.layer.offset";
    const TITLE: &'static str = "Move";
    const DESCRIPTION: &'static str = "Move layers by some pixels.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let ids = if self.ids.is_empty() {
            vec![active(doc)?]
        } else {
            self.ids.clone()
        };
        fn shift_layer(l: &mut Layer, dx: i32, dy: i32) {
            if let Content::Pixels(r) = &mut l.content {
                *r = shifted(r, dx, dy);
            }
            if let Some(m) = &mut l.mask
                && m.linked
            {
                m.raster = shifted(&m.raster, dx, dy);
            }
            for c in l.children_mut().into_iter().flatten() {
                shift_layer(c, dx, dy);
            }
        }
        for id in ids {
            let layer = layer_mut(doc, id)?;
            if layer.locks.position_locked() {
                return Err(format!(
                    "{} cannot be moved, its position is locked.",
                    layer.name
                ));
            }
            shift_layer(layer, self.dx, self.dy);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AddMask {
    pub id: Option<LayerId>,
    /// with no selection: true shows the whole layer, false hides it
    pub reveal: bool,
}

impl Edit for AddMask {
    const KIND: &'static str = "image.mask.add";
    const TITLE: &'static str = "Add layer mask";
    const DESCRIPTION: &'static str = "Add a mask to a layer, from the selection if there is one.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.map_or_else(|| active(doc), Ok)?;
        let depth = doc.depth;
        let raster = match &doc.selection {
            Some(sel) => {
                let mut m = Raster::gray(depth, 0.0);
                let area = sel.content_bounds();
                m.update(area, |x, y, v| v[0] = sel.get(x, y, 0));
                m
            }
            None => Raster::gray(depth, if self.reveal { 1.0 } else { 0.0 }),
        };
        let layer = layer_mut(doc, id)?;
        unlocked(layer)?;
        if layer.mask.is_some() {
            return Err("The layer already has a mask.".into());
        }
        layer.mask = Some(LayerMask {
            raster,
            enabled: true,
            linked: true,
        });
        doc.active = Some(id);
        doc.target = EditTarget::Mask;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoveMask {
    pub id: Option<LayerId>,
    /// bake the mask into the layer's pixels first
    pub apply: bool,
}

impl Edit for RemoveMask {
    const KIND: &'static str = "image.mask.remove";
    const TITLE: &'static str = "Delete layer mask";
    const DESCRIPTION: &'static str = "Remove a layer's mask, or apply it to the pixels.";
    fn name(&self) -> String {
        if self.apply {
            "Apply layer mask".into()
        } else {
            "Delete layer mask".into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.map_or_else(|| active(doc), Ok)?;
        let layer = layer_mut(doc, id)?;
        unlocked(layer)?;
        let mask = layer.mask.take().ok_or("The layer has no mask.")?;
        if self.apply
            && let Content::Pixels(r) = &mut layer.content
        {
            let area = r.tile_extent();
            r.update(area, |x, y, px| px[3] *= mask.raster.get(x, y, 0));
        }
        doc.target = EditTarget::Pixels;
        Ok(())
    }
}

/// a marquee or lasso selection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectShape {
    pub shape: Shape,
    #[serde(default)]
    pub mode: Combine,
    #[serde(default = "yes")]
    pub anti_alias: bool,
    #[serde(default)]
    pub feather: f32,
}

fn yes() -> bool {
    true
}

impl Edit for SelectShape {
    const KIND: &'static str = "image.select.shape";
    const TITLE: &'static str = "Select";
    const DESCRIPTION: &'static str = "Select a rectangle, an ellipse or a drawn outline.";
    fn name(&self) -> String {
        match self.shape {
            Shape::Rect { .. } => "Rectangular marquee",
            Shape::Ellipse { .. } => "Elliptical marquee",
            Shape::Polygon { .. } => "Lasso",
        }
        .to_owned()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let canvas = doc.canvas();
        let cov = self.shape.coverage(self.anti_alias);
        let mut shape =
            selection::combine(None, canvas, self.shape.bounds(), Combine::Replace, &*cov);
        if self.feather > 0.0
            && let Some(s) = &shape
        {
            shape = selection::feather(s, canvas, self.feather)?;
        }
        let next = match (self.mode, shape) {
            (Combine::Replace, s) => s,
            (mode, Some(s)) => selection::combine_masks(doc.selection.as_ref(), &s, canvas, mode),
            (Combine::Intersect, None) => None,
            (_, None) => doc.selection.clone(),
        };
        set_selection(doc, next);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectWand {
    pub x: i32,
    pub y: i32,
    #[serde(default = "tolerance")]
    pub tolerance: f32,
    #[serde(default = "yes")]
    pub contiguous: bool,
    #[serde(default)]
    pub sample_all: bool,
    #[serde(default = "yes")]
    pub anti_alias: bool,
    #[serde(default)]
    pub mode: Combine,
}

fn tolerance() -> f32 {
    32.0
}

impl Edit for SelectWand {
    const KIND: &'static str = "image.select.wand";
    const TITLE: &'static str = "Magic wand";
    const DESCRIPTION: &'static str = "Select an area of similar colour.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let img = sample_source(doc, self.sample_all)?;
        let found = selection::magic_wand(
            &img,
            (self.x, self.y),
            self.tolerance,
            self.contiguous,
            self.anti_alias,
        );
        let mask = found.to_raster(doc.canvas());
        apply_mask(doc, mask, self.mode);
        Ok(())
    }
}

fn apply_mask(doc: &mut ImageDocument, mask: Option<Raster>, mode: Combine) {
    let canvas = doc.canvas();
    let next = match (mode, mask) {
        (Combine::Replace, m) => m,
        (mode, Some(m)) => selection::combine_masks(doc.selection.as_ref(), &m, canvas, mode),
        (Combine::Intersect, None) => None,
        (_, None) => doc.selection.clone(),
    };
    set_selection(doc, next);
}

/// the quick selection brush: adds (or with subtract, removes) the area a stroke finds
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectQuick {
    pub points: Vec<[f32; 2]>,
    pub radius: f32,
    #[serde(default)]
    pub sample_all: bool,
    #[serde(default)]
    pub subtract: bool,
}

impl Edit for SelectQuick {
    const KIND: &'static str = "image.select.quick";
    const TITLE: &'static str = "Quick selection";
    const DESCRIPTION: &'static str =
        "Paint over something to select it, the selection spreads to its edges.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let img = sample_source(doc, self.sample_all)?;
        let found = selection::quick_select(&img, &self.points, self.radius);
        let mode = if self.subtract {
            Combine::Subtract
        } else if doc.selection.is_some() {
            Combine::Add
        } else {
            Combine::Replace
        };
        apply_mask(doc, found.to_raster(doc.canvas()), mode);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectRange {
    /// straight srgb samples
    pub colors: Vec<[f32; 3]>,
    #[serde(default = "fuzziness")]
    pub fuzziness: f32,
    #[serde(default)]
    pub sample_all: bool,
    #[serde(default)]
    pub invert: bool,
    #[serde(default)]
    pub mode: Combine,
}

fn fuzziness() -> f32 {
    40.0
}

impl Edit for SelectRange {
    const KIND: &'static str = "image.select.range";
    const TITLE: &'static str = "Colour range";
    const DESCRIPTION: &'static str = "Select every pixel close to some colours.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if self.colors.is_empty() {
            return Err("Pick at least one colour to select.".into());
        }
        let img = sample_source(doc, self.sample_all)?;
        let colors: Vec<[f32; 3]> = self
            .colors
            .iter()
            .map(|c| paint::doc_color(doc, *c))
            .collect();
        let mut found = selection::color_range(&img, &colors, self.fuzziness);
        if self.invert {
            found.data.iter_mut().for_each(|v| *v = 1.0 - *v);
        }
        apply_mask(doc, found.to_raster(doc.canvas()), self.mode);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectOpKind {
    All,
    None,
    Reselect,
    Invert,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectOp {
    pub op: SelectOpKind,
}

impl Edit for SelectOp {
    const KIND: &'static str = "image.select.op";
    const TITLE: &'static str = "Select";
    const DESCRIPTION: &'static str = "Select all, deselect, reselect or invert the selection.";
    fn name(&self) -> String {
        match self.op {
            SelectOpKind::All => "Select all",
            SelectOpKind::None => "Deselect",
            SelectOpKind::Reselect => "Reselect",
            SelectOpKind::Invert => "Invert selection",
        }
        .to_owned()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let canvas = doc.canvas();
        match self.op {
            SelectOpKind::All => set_selection(doc, Some(selection::select_all(canvas))),
            SelectOpKind::None => {
                if doc.selection.is_none() {
                    return Err("Nothing is selected.".into());
                }
                set_selection(doc, None);
            }
            SelectOpKind::Reselect => {
                let last = doc
                    .last_selection
                    .clone()
                    .ok_or("There is no earlier selection to bring back.")?;
                set_selection(doc, Some(last));
            }
            SelectOpKind::Invert => {
                let next = selection::invert(doc.selection.as_ref(), canvas);
                set_selection(doc, next);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Modify {
    Feather,
    Expand,
    Contract,
    Border,
    Smooth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifySelection {
    pub op: Modify,
    /// pixels
    pub amount: f32,
}

impl Edit for ModifySelection {
    const KIND: &'static str = "image.select.modify";
    const TITLE: &'static str = "Modify selection";
    const DESCRIPTION: &'static str = "Feather, expand, contract, border or smooth the selection.";
    fn name(&self) -> String {
        match self.op {
            Modify::Feather => "Feather",
            Modify::Expand => "Expand selection",
            Modify::Contract => "Contract selection",
            Modify::Border => "Border selection",
            Modify::Smooth => "Smooth selection",
        }
        .to_owned()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let sel = doc.selection.clone().ok_or("Select something first.")?;
        let canvas = doc.canvas();
        let amount = self.amount.clamp(0.0, 1000.0);
        let next = match self.op {
            Modify::Feather => selection::feather(&sel, canvas, amount)?,
            Modify::Expand => selection::expand(&sel, canvas, amount)?,
            Modify::Contract => selection::expand(&sel, canvas, -amount)?,
            Modify::Border => selection::border(&sel, canvas, amount)?,
            Modify::Smooth => selection::smooth(&sel, canvas, amount)?,
        };
        set_selection(doc, next);
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SaveSelection {
    pub name: Option<String>,
}

impl Edit for SaveSelection {
    const KIND: &'static str = "image.select.save";
    const TITLE: &'static str = "Save selection";
    const DESCRIPTION: &'static str = "Keep the selection as a channel.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let sel = doc.selection.clone().ok_or("Select something first.")?;
        let id = doc.channels.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let name = self
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Selection {id}"));
        doc.channels.push(SavedChannel {
            id,
            name,
            raster: sel.duplicate(),
        });
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSelection {
    pub channel: u32,
    #[serde(default)]
    pub mode: Combine,
    #[serde(default)]
    pub invert: bool,
}

impl Edit for LoadSelection {
    const KIND: &'static str = "image.select.load";
    const TITLE: &'static str = "Load selection";
    const DESCRIPTION: &'static str = "Bring back a saved selection.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let canvas = doc.canvas();
        let saved = doc
            .channels
            .iter()
            .find(|c| c.id == self.channel)
            .map(|c| c.raster.clone())
            .ok_or("That saved selection is gone.")?;
        let saved = if self.invert {
            selection::invert(Some(&saved), canvas)
        } else {
            Some(saved)
        };
        apply_mask(doc, saved, self.mode);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveChannel {
    pub channel: u32,
}

impl Edit for RemoveChannel {
    const KIND: &'static str = "image.channel.remove";
    const TITLE: &'static str = "Delete saved selection";
    const DESCRIPTION: &'static str = "Remove a saved selection.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let before = doc.channels.len();
        doc.channels.retain(|c| c.id != self.channel);
        if doc.channels.len() == before {
            return Err("That saved selection is gone.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickMask {
    pub on: bool,
}

impl Edit for QuickMask {
    const KIND: &'static str = "image.quick_mask";
    const TITLE: &'static str = "Quick mask";
    const DESCRIPTION: &'static str = "Show the selection as a red overlay that brushes can paint.";
    fn name(&self) -> String {
        if self.on {
            "Enter quick mask".into()
        } else {
            "Exit quick mask".into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        doc.quick_mask = self.on;
        Ok(())
    }
}

/// a brush, pencil or eraser stroke. replaying it repaints it from the points; when the
/// stroke was painted live, the finished pixels come along and are used as they are.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintStroke {
    pub settings: BrushSettings,
    pub points: Vec<StrokePoint>,
    #[serde(skip)]
    pub painted: Option<Painted>,
}

/// pixels a live stroke already made, with the pixels it started from
#[derive(Debug, Clone)]
pub struct Painted {
    pub part: Part,
    pub base: Raster,
    pub result: Raster,
}

impl PaintStroke {
    /// the command for a stroke that was drawn live
    pub fn from_stroke(stroke: &Stroke, base: Raster) -> Self {
        Self {
            settings: stroke.settings().clone(),
            points: stroke.points().to_vec(),
            painted: Some(Painted {
                part: stroke.part.clone(),
                base,
                result: stroke.result().clone(),
            }),
        }
    }
}

impl Edit for PaintStroke {
    const KIND: &'static str = "image.paint.stroke";
    const TITLE: &'static str = "Brush";
    const DESCRIPTION: &'static str = "Paint, draw or erase along a line.";
    fn name(&self) -> String {
        match self.settings.tool {
            paint::BrushTool::Brush => "Brush",
            paint::BrushTool::Pencil => "Pencil",
            paint::BrushTool::Eraser => "Eraser",
        }
        .to_owned()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if let Some(p) = self.painted.take() {
            // the live stroke still fits if nothing changed its part meanwhile
            if paint::raster_for(doc, &p.part).is_ok_and(|now| now.same_as(&p.base)) {
                paint::set_part(doc, &p.part, p.result);
                return Ok(());
            }
        }
        let mut stroke = Stroke::new(doc, self.settings.clone())?;
        stroke.add(&self.points);
        stroke.finish();
        let part = stroke.part.clone();
        paint::set_part(doc, &part, stroke.result().clone());
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub settings: FillSettings,
}

impl Edit for Bucket {
    const KIND: &'static str = "image.paint.bucket";
    const TITLE: &'static str = "Paint bucket";
    const DESCRIPTION: &'static str = "Fill an area of similar colour.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let part = paint::active_part(doc)?;
        let img = match &part {
            Part::Pixels(_) => sample_source(doc, self.settings.sample_all)?,
            other => {
                // masks fill by their own grey values
                let canvas = doc.canvas();
                let grey = paint::raster_for(doc, other)?.read(canvas);
                ColorBuf {
                    rect: canvas,
                    data: grey
                        .iter()
                        .map(|v| {
                            let q = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
                            [q, q, q, 65535]
                        })
                        .collect(),
                }
            }
        };
        paint::bucket(doc, &img, self.x, self.y, &self.settings)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gradient {
    pub from: [f32; 2],
    pub to: [f32; 2],
    #[serde(default)]
    pub settings: GradientSettings,
}

impl Edit for Gradient {
    const KIND: &'static str = "image.paint.gradient";
    const TITLE: &'static str = "Gradient";
    const DESCRIPTION: &'static str = "Fill with a gradient.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        paint::gradient(doc, self.from, self.to, &self.settings)
    }
}

/// fills the selection (or the whole layer) with a colour, or clears it
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillSelection {
    /// straight srgb; none clears to transparent
    pub color: Option<[f32; 3]>,
    #[serde(default = "one")]
    pub opacity: f32,
}

fn one() -> f32 {
    1.0
}

impl Edit for FillSelection {
    const KIND: &'static str = "image.paint.fill";
    const TITLE: &'static str = "Fill";
    const DESCRIPTION: &'static str =
        "Fill the selection with a colour, or delete the selected pixels.";
    fn name(&self) -> String {
        if self.color.is_some() {
            "Fill".into()
        } else {
            "Clear".into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let part = paint::active_part(doc)?;
        let area = match (&part, doc.edit_area()) {
            (Part::Selection, _) | (_, None) => doc.canvas(),
            (_, Some(r)) => r,
        };
        let opacity = self.opacity.clamp(0.0, 1.0);
        match self.color {
            Some(c) => paint::paint_area(doc, &part, area, c, BlendMode::Normal, &|_, _| opacity),
            None => {
                let mut r = paint::raster_for(doc, &part)?;
                if let Part::Pixels(id) = part
                    && doc.layer(id).is_some_and(|l| l.locks.transparency)
                {
                    return Err(
                        "See through pixels are locked on this layer, so it cannot be cleared."
                            .into(),
                    );
                }
                let sel = doc.selection.clone();
                let gray = r.channels() == crate::raster::Channels::Gray;
                let clear_area = if sel.is_some() {
                    area
                } else {
                    r.tile_extent().union(&area)
                };
                r.update(clear_area, |x, y, px| {
                    let a = opacity * sel.as_ref().map_or(1.0, |s| s.get(x, y, 0));
                    if gray {
                        px[0] *= 1.0 - a;
                    } else {
                        px[3] *= 1.0 - a;
                    }
                });
                paint::set_part(doc, &part, r);
                Ok(())
            }
        }
    }
}

impl vellum_core::documents::Titled for ImageDocument {
    fn title_mut(&mut self) -> &mut String {
        &mut self.title
    }
}

fn add<E: Edit>(r: &mut CommandRegistry<ImageDocument>, category: &'static str) {
    let info = CommandInfo {
        kind: E::KIND,
        title: E::TITLE,
        category,
        description: E::DESCRIPTION,
    };
    if let Err(err) = r.register::<Cmd<E>>(info) {
        tracing::error!(%err, "an image command was registered twice");
    }
}

/// every command an image document understands
pub fn registry() -> CommandRegistry<ImageDocument> {
    let mut r = CommandRegistry::new();
    add::<AddLayer>(&mut r, "Layers");
    add::<RemoveLayers>(&mut r, "Layers");
    add::<DuplicateLayer>(&mut r, "Layers");
    add::<LayerProps>(&mut r, "Layers");
    add::<SelectLayer>(&mut r, "Layers");
    add::<MoveLayer>(&mut r, "Layers");
    add::<GroupLayers>(&mut r, "Layers");
    add::<Ungroup>(&mut r, "Layers");
    add::<MergeDown>(&mut r, "Layers");
    add::<MergeVisible>(&mut r, "Layers");
    add::<LayerViaCopy>(&mut r, "Layers");
    add::<OffsetLayers>(&mut r, "Layers");
    add::<AddMask>(&mut r, "Layers");
    add::<RemoveMask>(&mut r, "Layers");
    add::<SelectShape>(&mut r, "Select");
    add::<SelectWand>(&mut r, "Select");
    add::<SelectQuick>(&mut r, "Select");
    add::<SelectRange>(&mut r, "Select");
    add::<SelectOp>(&mut r, "Select");
    add::<ModifySelection>(&mut r, "Select");
    add::<SaveSelection>(&mut r, "Select");
    add::<LoadSelection>(&mut r, "Select");
    add::<RemoveChannel>(&mut r, "Select");
    add::<QuickMask>(&mut r, "Select");
    add::<PaintStroke>(&mut r, "Paint");
    add::<Bucket>(&mut r, "Paint");
    add::<Gradient>(&mut r, "Paint");
    add::<FillSelection>(&mut r, "Paint");
    add::<crate::transform::Transform>(&mut r, "Transform");
    add::<crate::canvas::OrientCanvas>(&mut r, "Image");
    add::<crate::canvas::OrientLayer>(&mut r, "Transform");
    add::<crate::canvas::CanvasSize>(&mut r, "Image");
    add::<crate::canvas::ImageSize>(&mut r, "Image");
    add::<crate::canvas::Crop>(&mut r, "Image");
    add::<crate::canvas::AddGuide>(&mut r, "View");
    add::<crate::canvas::MoveGuide>(&mut r, "View");
    add::<crate::canvas::ClearGuides>(&mut r, "View");
    add::<crate::layer_ops::AddAdjustment>(&mut r, "Layers");
    add::<crate::layer_ops::SetAdjustment>(&mut r, "Layers");
    add::<crate::layer_ops::DeriveAdjustment>(&mut r, "Image");
    add::<crate::layer_ops::MatchColorFrom>(&mut r, "Image");
    add::<crate::layer_ops::AddFill>(&mut r, "Layers");
    add::<crate::layer_ops::SetFill>(&mut r, "Layers");
    add::<crate::layer_ops::ConvertToSmart>(&mut r, "Layers");
    add::<crate::layer_ops::PlaceSmart>(&mut r, "File");
    add::<crate::layer_ops::ReplaceContents>(&mut r, "Layers");
    add::<crate::layer_ops::Rasterize>(&mut r, "Layers");
    add::<crate::layer_ops::AddSmartFilter>(&mut r, "Filter");
    add::<crate::layer_ops::SetSmartFilter>(&mut r, "Filter");
    add::<crate::layer_ops::ApplyFilter>(&mut r, "Filter");
    add::<crate::layer_ops::TargetFilterMask>(&mut r, "Filter");
    if let Err(err) = r.register::<RenameDocument>(CommandInfo {
        kind: RenameDocument::KIND,
        title: "Rename document",
        category: "Document",
        description: "Change the name shown on the tab.",
    }) {
        tracing::error!(%err, "rename was registered twice");
    }
    r
}

/// the magnetic lasso's edges, made once when a drag starts
pub fn edges_for(doc: &ImageDocument, sample_all: bool, contrast: f32) -> Result<EdgeMap, String> {
    Ok(EdgeMap::new(&sample_source(doc, sample_all)?, contrast))
}

/// the eyedropper: a colour from the active layer or what every layer shows
pub fn pick_color(
    doc: &ImageDocument,
    x: i32,
    y: i32,
    size: u32,
    all: bool,
) -> Result<[f32; 4], String> {
    let half = (size.max(1) / 2) as i32;
    let rect = IRect::new(x - half, y - half, size.max(1), size.max(1)).intersect(&doc.canvas());
    if rect.is_empty() {
        return Err("Pick a colour inside the picture.".into());
    }
    let img = if all {
        flatten::colors(doc, rect)?
    } else {
        let layer = doc.active_layer().ok_or("Pick a layer first.")?;
        match layer.raster() {
            Some(r) => ColorBuf::from_raster(r, rect),
            None => flatten::colors(doc, rect)?,
        }
    };
    let mut c = paint::sample(&img, x, y, size);
    if doc.depth == Depth::F32 {
        for v in c.iter_mut().take(3) {
            *v = vellum_gpu::Transfer::Srgb.encode(*v);
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_core::command::CommandRecord;
    use vellum_core::history::{History, HistoryLimits};

    fn run(
        h: &mut History<ImageDocument>,
        doc: &mut ImageDocument,
        kind: &str,
        params: serde_json::Value,
    ) {
        let cmd = registry()
            .instantiate(&CommandRecord::new(kind, params))
            .unwrap();
        h.execute(cmd, doc).unwrap();
    }

    #[test]
    fn every_edit_undoes_and_redoes_exactly() {
        let mut doc = ImageDocument::new("t", 64, 64, Depth::U8, Some([1.0; 4]));
        let mut h = History::new(HistoryLimits::default());
        let steps: Vec<(&str, serde_json::Value)> = vec![
            (AddLayer::KIND, serde_json::json!({})),
            (
                PaintStroke::KIND,
                serde_json::json!({ "settings": { "smoothing": 0.0, "color": [1.0, 0.0, 0.0] }, "points": [{ "x": 5.0, "y": 5.0 }, { "x": 50.0, "y": 40.0 }] }),
            ),
            (LayerProps::KIND, serde_json::json!({ "opacity": 0.5 })),
            (
                SelectShape::KIND,
                serde_json::json!({ "shape": { "type": "rect", "x": 10.0, "y": 10.0, "w": 20.0, "h": 20.0 } }),
            ),
            (
                FillSelection::KIND,
                serde_json::json!({ "color": [0.0, 0.0, 1.0] }),
            ),
            (
                ModifySelection::KIND,
                serde_json::json!({ "op": "expand", "amount": 3.0 }),
            ),
            (AddMask::KIND, serde_json::json!({})),
            (DuplicateLayer::KIND, serde_json::json!({})),
            (MergeDown::KIND, serde_json::json!({})),
        ];
        let mut states = vec![doc.bytes()];
        for (kind, params) in steps.iter().cloned() {
            run(&mut h, &mut doc, kind, params);
            h.seal();
            states.push(doc.bytes());
        }
        assert_eq!(doc.layers.len(), 2);
        let final_layers = doc.layers.len();
        while h.undo(&mut doc).unwrap() {}
        assert_eq!(doc.layers.len(), 1);
        assert!(doc.selection.is_none());
        assert_eq!(doc.bytes(), states[0]);
        while h.redo(&mut doc).unwrap() {}
        assert_eq!(doc.layers.len(), final_layers);
        assert_eq!(doc.bytes(), *states.last().unwrap());
    }

    #[test]
    fn slider_drags_merge_into_one_step() {
        let mut doc = ImageDocument::new("t", 8, 8, Depth::U8, None);
        let mut h = History::new(HistoryLimits::default());
        for v in [0.9, 0.7, 0.4] {
            run(
                &mut h,
                &mut doc,
                LayerProps::KIND,
                serde_json::json!({ "opacity": v }),
            );
        }
        assert_eq!(h.view().entries.len(), 2);
        h.undo(&mut doc).unwrap();
        assert_eq!(doc.layers[0].opacity, 1.0);
    }

    #[test]
    fn failed_edits_leave_the_document_alone() {
        let mut doc = ImageDocument::new("t", 8, 8, Depth::U8, None);
        let mut h = History::new(HistoryLimits::default());
        let cmd = registry()
            .instantiate(&CommandRecord::new(MergeDown::KIND, serde_json::json!({})))
            .unwrap();
        assert!(h.execute(cmd, &mut doc).is_err());
        assert_eq!(doc.layers.len(), 1);
    }

    #[test]
    fn locks_stop_painting_and_moving() {
        let mut doc = ImageDocument::new("t", 8, 8, Depth::U8, None);
        doc.layers[0].locks.pixels = true;
        doc.layers[0].locks.position = true;
        let mut h = History::new(HistoryLimits::default());
        for (kind, params) in [
            (
                PaintStroke::KIND,
                serde_json::json!({ "settings": {}, "points": [{ "x": 1.0, "y": 1.0 }] }),
            ),
            (OffsetLayers::KIND, serde_json::json!({ "dx": 3, "dy": 0 })),
        ] {
            let cmd = registry()
                .instantiate(&CommandRecord::new(kind, params))
                .unwrap();
            assert!(h.execute(cmd, &mut doc).is_err(), "{kind}");
        }
    }

    #[test]
    fn groups_ungroup_and_moves_keep_order() {
        let mut doc = ImageDocument::new("t", 8, 8, Depth::U8, None);
        let mut h = History::new(HistoryLimits::default());
        run(&mut h, &mut doc, AddLayer::KIND, serde_json::json!({}));
        run(&mut h, &mut doc, AddLayer::KIND, serde_json::json!({}));
        let names = |d: &ImageDocument| d.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&doc), ["Layer 1", "Layer 2", "Layer 3"]);
        let top = doc.layers[2].id;
        run(
            &mut h,
            &mut doc,
            MoveLayer::KIND,
            serde_json::json!({ "id": top, "index": 0 }),
        );
        assert_eq!(names(&doc), ["Layer 3", "Layer 1", "Layer 2"]);
        let ids: Vec<LayerId> = doc.layers[1..].iter().map(|l| l.id).collect();
        run(
            &mut h,
            &mut doc,
            GroupLayers::KIND,
            serde_json::json!({ "ids": ids }),
        );
        assert_eq!(names(&doc), ["Layer 3", "Group 1"]);
        assert_eq!(doc.layers[1].children().len(), 2);
        run(&mut h, &mut doc, Ungroup::KIND, serde_json::json!({}));
        assert_eq!(names(&doc), ["Layer 3", "Layer 1", "Layer 2"]);
    }
}
