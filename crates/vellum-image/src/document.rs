//! the image document: canvas, layers, the selection and saved selections.
//!
//! layers are stored bottom to top, the order they composite in. a group holds its own
//! list the same way. every raster is in document coordinates, so moving a layer moves its
//! pixels rather than keeping an offset that every tool would have to remember.
//!
//! the document is cheap to clone (rasters share their tiles), which is how undo works:
//! a command keeps the document as it was before and after, see [`crate::commands`].

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::tiles::SourceColor;
use vellum_gpu::{
    BlendMode, ColorSpace, ColorTransform, IRect, IccProfile, Precision, WorkingSpace,
};

use crate::raster::{Depth, Raster};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LayerId(pub u32);

/// the colour tag on a layer, like photoshop's
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Label {
    #[default]
    None,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Violet,
    Gray,
}

/// what may not be changed on a layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Locks {
    /// painting keeps see through pixels see through
    pub transparency: bool,
    /// no painting, filling or erasing
    pub pixels: bool,
    /// no moving
    pub position: bool,
    /// nothing at all, including deleting and renaming
    pub all: bool,
}

impl Locks {
    pub fn pixels_locked(&self) -> bool {
        self.all || self.pixels
    }

    pub fn position_locked(&self) -> bool {
        self.all || self.position
    }
}

/// a layer mask: white shows the layer, black hides it
#[derive(Clone, Debug)]
pub struct LayerMask {
    /// grey, with the fill value as the colour outside what was painted
    pub raster: Raster,
    pub enabled: bool,
    /// moves with the layer
    pub linked: bool,
}

#[derive(Clone, Debug)]
pub enum Content {
    Pixels(Raster),
    Group {
        children: Vec<Layer>,
        expanded: bool,
        /// children blend straight into what is below the group instead of into the group
        /// first. photoshop's default for groups.
        pass_through: bool,
    },
    /// changes the colours of everything below it
    Adjustment(crate::adjustments::Adjustment),
    /// a colour, gradient or pattern over the whole canvas
    Fill(crate::fills::FillLayer),
    /// placed pixels that keep their quality through transforms, with live filters
    Smart(Box<crate::smart::SmartObject>),
    /// type, kept as text and drawn into `pixels` whenever it changes
    Text {
        text: Box<crate::text::TextLayer>,
        pixels: Raster,
    },
    /// vector shapes, drawn into `pixels` whenever they change
    Shape {
        shape: Box<crate::vector::ShapeLayer>,
        pixels: Raster,
    },
}

/// a vector mask and the coverage drawn from it
#[derive(Clone, Debug)]
pub struct LayerVectorMask {
    pub mask: crate::vector::VectorMask,
    pub raster: Raster,
}

impl LayerVectorMask {
    pub fn new(mask: crate::vector::VectorMask, canvas: IRect) -> Self {
        let raster = mask.raster(canvas);
        Self { mask, raster }
    }
}

/// a group shown as an artboard: its own box on the canvas, with an optional background,
/// that clips what is inside it and exports on its own
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Artboard {
    pub rect: IRect,
    /// straight srgb, none for see through
    pub background: Option<[f32; 4]>,
}

/// a path kept in the paths panel
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedPath {
    pub id: u32,
    pub name: String,
    pub path: crate::vector::VectorPath,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    /// 0 to 1, applies to everything the layer draws
    pub opacity: f32,
    /// 0 to 1, applies to the layer's own pixels
    pub fill: f32,
    pub blend: BlendMode,
    pub locks: Locks,
    pub label: Label,
    /// shown only where the layer below it has pixels
    pub clipped: bool,
    pub mask: Option<LayerMask>,
    pub content: Content,
    /// what the layer was before it became pixels, eg "text" for imported type
    pub origin: Option<String>,
    pub vector_mask: Option<LayerVectorMask>,
    pub style: Option<crate::styles::LayerStyle>,
    /// groups only
    pub artboard: Option<Artboard>,
}

impl Layer {
    pub fn pixels(id: LayerId, name: impl Into<String>, raster: Raster) -> Self {
        Self {
            id,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            fill: 1.0,
            blend: BlendMode::Normal,
            locks: Locks::default(),
            label: Label::None,
            clipped: false,
            mask: None,
            content: Content::Pixels(raster),
            origin: None,
            vector_mask: None,
            style: None,
            artboard: None,
        }
    }

    pub fn group(id: LayerId, name: impl Into<String>, children: Vec<Layer>) -> Self {
        Self {
            content: Content::Group {
                children,
                expanded: true,
                pass_through: true,
            },
            ..Self::pixels(id, name, Raster::rgba(Depth::U8))
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self.content, Content::Group { .. })
    }

    /// the layer's own pixels: a pixel layer's, or what a text or shape layer last drew
    pub fn raster(&self) -> Option<&Raster> {
        match &self.content {
            Content::Pixels(r) => Some(r),
            Content::Text { pixels, .. } | Content::Shape { pixels, .. } => Some(pixels),
            _ => None,
        }
    }

    pub fn raster_mut(&mut self) -> Option<&mut Raster> {
        match &mut self.content {
            Content::Pixels(r) => Some(r),
            _ => None,
        }
    }

    pub fn children(&self) -> &[Layer] {
        match &self.content {
            Content::Group { children, .. } => children,
            _ => &[],
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Layer>> {
        match &mut self.content {
            Content::Group { children, .. } => Some(children),
            _ => None,
        }
    }

    pub fn smart(&self) -> Option<&crate::smart::SmartObject> {
        match &self.content {
            Content::Smart(s) => Some(s),
            _ => None,
        }
    }

    pub fn smart_mut(&mut self) -> Option<&mut crate::smart::SmartObject> {
        match &mut self.content {
            Content::Smart(s) => Some(s),
            _ => None,
        }
    }

    /// what kind of layer this is, for messages and the layers panel
    pub fn kind_name(&self) -> &'static str {
        match &self.content {
            Content::Pixels(_) => "layer",
            Content::Group { .. } => "group",
            Content::Adjustment(_) => "adjustment layer",
            Content::Fill(_) => "fill layer",
            Content::Smart(_) => "smart object",
            Content::Text { .. } => "text layer",
            Content::Shape { .. } => "shape layer",
        }
    }

    /// the pixels painting tools change, or why there are none
    pub fn paintable(&self) -> Result<&Raster, String> {
        match &self.content {
            Content::Pixels(r) => Ok(r),
            Content::Group { .. } => {
                Err("Groups have no pixels to paint. Pick a layer inside the group.".into())
            }
            _ => Err(format!(
                "{} is a {}. Rasterize it (Layer, Rasterize) to paint on its pixels, or paint on its mask.",
                self.name,
                self.kind_name()
            )),
        }
    }

    /// the area this layer draws into, groups included. fills and adjustments cover
    /// everything, which shows as an empty area here.
    pub fn bounds(&self) -> IRect {
        match &self.content {
            Content::Pixels(r) => r.content_bounds(),
            Content::Group { children, .. } => children
                .iter()
                .fold(IRect::EMPTY, |acc, c| acc.union(&c.bounds())),
            Content::Smart(s) => s.bounds(),
            Content::Text { pixels, .. } | Content::Shape { pixels, .. } => pixels.content_bounds(),
            Content::Adjustment(_) | Content::Fill(_) => IRect::EMPTY,
        }
    }
}

/// which part of the active layer tools change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EditTarget {
    #[default]
    Pixels,
    Mask,
    /// a smart object's filter mask
    FilterMask,
}

/// a guide line across the canvas, dragged out of a ruler
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Guide {
    pub id: u32,
    /// a vertical line at x, or a horizontal one at y
    pub vertical: bool,
    /// document pixels from the left or top edge
    pub position: f32,
}

/// a selection kept for later, photoshop's alpha channels
#[derive(Clone, Debug)]
pub struct SavedChannel {
    pub id: u32,
    pub name: String,
    pub raster: Raster,
}

/// how the document's values relate to colour
#[derive(Clone)]
pub struct DocColor {
    /// the profile the picture came with, kept so saving can embed it again
    pub icc: Option<Arc<Vec<u8>>>,
    pub name: Option<String>,
    source: SourceColor,
    luts: Arc<Mutex<HashMap<WorkingSpace, ColorTransform>>>,
}

impl std::fmt::Debug for DocColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocColor")
            .field("name", &self.name)
            .finish()
    }
}

impl Default for DocColor {
    fn default() -> Self {
        Self::srgb()
    }
}

impl DocColor {
    pub fn srgb() -> Self {
        Self {
            icc: None,
            name: None,
            source: SourceColor::Space(ColorSpace::SRGB),
            luts: Arc::default(),
        }
    }

    /// a document colour from an embedded profile. a profile that cannot be used falls
    /// back to srgb, which is what every browser does.
    pub fn from_icc(bytes: Vec<u8>) -> Self {
        match IccProfile::from_bytes(&bytes) {
            Ok(profile) => Self {
                name: Some(
                    profile
                        .description
                        .clone()
                        .unwrap_or_else(|| "Embedded profile".into()),
                ),
                source: SourceColor::Icc(profile),
                icc: Some(Arc::new(bytes)),
                luts: Arc::default(),
            },
            Err(err) => {
                tracing::warn!(%err, "treating a picture's colour profile as srgb");
                Self::srgb()
            }
        }
    }

    /// the transform from document values to a working space, built once per space
    pub fn to_working(&self, working: WorkingSpace) -> ColorTransform {
        let mut luts = self.luts.lock();
        if let Some(t) = luts.get(&working) {
            return t.clone();
        }
        let t = match &self.source {
            SourceColor::Space(space) => ColorTransform::between(*space, working.space()),
            SourceColor::Icc(icc) => icc.to_working(working).unwrap_or_else(|err| {
                tracing::warn!(%err, "falling back to srgb");
                ColorTransform::between(ColorSpace::SRGB, working.space())
            }),
        };
        luts.insert(working, t.clone());
        t
    }
}

#[derive(Clone, Debug)]
pub struct ImageDocument {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub depth: Depth,
    pub color: DocColor,
    /// pixels per inch, for printing
    pub resolution: f32,
    /// bottom to top
    pub layers: Vec<Layer>,
    pub active: Option<LayerId>,
    pub target: EditTarget,
    /// grey coverage, 1 selected. none means nothing is selected.
    pub selection: Option<Raster>,
    /// what "reselect" brings back
    pub last_selection: Option<Raster>,
    pub channels: Vec<SavedChannel>,
    /// the selection is shown as a red overlay and brushes paint it
    pub quick_mask: bool,
    pub guides: Vec<Guide>,
    /// the paths panel
    pub paths: Vec<SavedPath>,
    pub next_id: u32,
    /// set while the file is still loading in the background
    pub loading: Option<String>,
}

impl ImageDocument {
    /// a blank document with one layer, filled with `background` if given
    pub fn new(
        title: impl Into<String>,
        width: u32,
        height: u32,
        depth: Depth,
        background: Option<[f32; 4]>,
    ) -> Self {
        let mut doc = Self::empty(title, width, height, depth);
        let mut raster = Raster::rgba(depth);
        let name = if let Some(c) = background {
            if c[3] > 0.0 {
                raster.update(IRect::of_size(width, height), |_, _, px| {
                    px.copy_from_slice(&c)
                });
            }
            "Background"
        } else {
            "Layer 1"
        };
        let id = doc.new_id();
        doc.layers.push(Layer::pixels(id, name, raster));
        doc.active = Some(id);
        doc
    }

    /// a document with no layers, for loaders to fill in
    pub fn empty(title: impl Into<String>, width: u32, height: u32, depth: Depth) -> Self {
        Self {
            title: title.into(),
            width: width.max(1),
            height: height.max(1),
            depth,
            color: DocColor::srgb(),
            resolution: 72.0,
            layers: Vec::new(),
            active: None,
            target: EditTarget::Pixels,
            selection: None,
            last_selection: None,
            channels: Vec::new(),
            quick_mask: false,
            guides: Vec::new(),
            paths: Vec::new(),
            next_id: 1,
            loading: None,
        }
    }

    pub fn canvas(&self) -> IRect {
        IRect::of_size(self.width, self.height)
    }

    pub fn new_id(&mut self) -> LayerId {
        let id = LayerId(self.next_id);
        self.next_id += 1;
        id
    }

    /// photoshop blends 8 and 16 bit documents on gamma encoded values and 32 bit ones in
    /// linear light; matching it keeps blend modes looking the same
    pub fn working(&self) -> WorkingSpace {
        match self.depth {
            Depth::U8 | Depth::U16 => WorkingSpace::SRGB,
            Depth::F32 => WorkingSpace::LINEAR_SRGB,
        }
    }

    pub fn precision(&self) -> Precision {
        match self.depth {
            Depth::U8 => Precision::F16,
            Depth::U16 | Depth::F32 => Precision::F32,
        }
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        fn find(list: &[Layer], id: LayerId) -> Option<&Layer> {
            for l in list {
                if l.id == id {
                    return Some(l);
                }
                if let Some(found) = find(l.children(), id) {
                    return Some(found);
                }
            }
            None
        }
        find(&self.layers, id)
    }

    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        fn find(list: &mut [Layer], id: LayerId) -> Option<&mut Layer> {
            for l in list {
                if l.id == id {
                    return Some(l);
                }
                if let Some(children) = l.children_mut()
                    && let Some(found) = find(children, id)
                {
                    return Some(found);
                }
            }
            None
        }
        find(&mut self.layers, id)
    }

    pub fn active_layer(&self) -> Option<&Layer> {
        self.active.and_then(|id| self.layer(id))
    }

    /// where a layer is: its parent group (none for the top level) and index there
    pub fn locate(&self, id: LayerId) -> Option<(Option<LayerId>, usize)> {
        fn find(
            list: &[Layer],
            parent: Option<LayerId>,
            id: LayerId,
        ) -> Option<(Option<LayerId>, usize)> {
            for (i, l) in list.iter().enumerate() {
                if l.id == id {
                    return Some((parent, i));
                }
                if let Some(found) = find(l.children(), Some(l.id), id) {
                    return Some(found);
                }
            }
            None
        }
        find(&self.layers, None, id)
    }

    /// the list a layer lives in
    pub fn siblings_mut(&mut self, parent: Option<LayerId>) -> Option<&mut Vec<Layer>> {
        match parent {
            None => Some(&mut self.layers),
            Some(p) => self.layer_mut(p).and_then(Layer::children_mut),
        }
    }

    pub fn siblings(&self, parent: Option<LayerId>) -> Option<&[Layer]> {
        match parent {
            None => Some(&self.layers),
            Some(p) => self.layer(p).map(Layer::children),
        }
    }

    /// takes a layer out of the tree
    pub fn remove(&mut self, id: LayerId) -> Option<Layer> {
        let (parent, index) = self.locate(id)?;
        Some(self.siblings_mut(parent)?.remove(index))
    }

    /// puts a layer into a group (or the top level) at an index, clamped to the list
    pub fn insert(&mut self, parent: Option<LayerId>, index: usize, layer: Layer) -> bool {
        match self.siblings_mut(parent) {
            Some(list) => {
                let at = index.min(list.len());
                list.insert(at, layer);
                true
            }
            None => false,
        }
    }

    /// every layer, top level first, each followed by its children
    pub fn all_layers(&self) -> Vec<&Layer> {
        fn walk<'a>(list: &'a [Layer], out: &mut Vec<&'a Layer>) {
            for l in list {
                out.push(l);
                walk(l.children(), out);
            }
        }
        let mut out = Vec::new();
        walk(&self.layers, &mut out);
        out
    }

    /// true if `id` is `ancestor` or inside it
    pub fn is_inside(&self, id: LayerId, ancestor: LayerId) -> bool {
        if id == ancestor {
            return true;
        }
        self.layer(ancestor)
            .is_some_and(|a| a.children().iter().any(|c| self.is_inside(id, c.id)))
    }

    /// a name like "Layer 4" that no layer has yet
    pub fn fresh_name(&self, stem: &str) -> String {
        let names: Vec<&str> = self.all_layers().iter().map(|l| l.name.as_str()).collect();
        (1..)
            .map(|n| format!("{stem} {n}"))
            .find(|n| !names.contains(&n.as_str()))
            .unwrap_or_else(|| stem.to_owned())
    }

    /// the selection's coverage at a pixel, 1 everywhere when nothing is selected
    pub fn selected(&self, x: i32, y: i32) -> f32 {
        match &self.selection {
            Some(s) => s.get(x, y, 0),
            None => 1.0,
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// the area painting can touch: the selection's bounds, or everything
    pub fn edit_area(&self) -> Option<IRect> {
        self.selection.as_ref().map(Raster::content_bounds)
    }

    /// bytes of pixels, counting each shared tile once per raster
    pub fn bytes(&self) -> usize {
        let mut total = 0;
        for l in self.all_layers() {
            total += l.raster().map_or(0, Raster::bytes);
            total += l.mask.as_ref().map_or(0, |m| m.raster.bytes());
        }
        total + self.selection.as_ref().map_or(0, Raster::bytes)
    }
}

/// searches layers by name, kind and colour label, for the layers panel. `query` matches
/// names ignoring case; "group", "pixels", "text", "masked", "hidden" and label names also
/// match what they say. returns ids in panel order (top first).
pub fn search_layers(doc: &ImageDocument, query: &str) -> Vec<LayerId> {
    let q = query.trim().to_lowercase();
    let mut hits = Vec::new();
    fn walk(list: &[Layer], q: &str, hits: &mut Vec<LayerId>) {
        for l in list.iter().rev() {
            let label = format!("{:?}", l.label).to_lowercase();
            let matches = q.is_empty()
                || l.name.to_lowercase().contains(q)
                || (q == "group" && l.is_group())
                || (q == "pixels" && !l.is_group())
                || (q == "masked" && l.mask.is_some())
                || (q == "hidden" && !l.visible)
                || (q == "clipped" && l.clipped)
                || l.origin.as_deref() == Some(q)
                || (l.label != Label::None && label == q);
            if matches {
                hits.push(l.id);
            }
            walk(l.children(), q, hits);
        }
    }
    walk(&doc.layers, &q, &mut hits);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_are_found_moved_and_named() {
        let mut doc = ImageDocument::new("t", 64, 64, Depth::U8, Some([1.0; 4]));
        let a = doc.new_id();
        let b = doc.new_id();
        let g = doc.new_id();
        doc.layers.push(Layer::group(
            g,
            "Group 1",
            vec![Layer::pixels(a, "Layer 2", Raster::rgba(Depth::U8))],
        ));
        doc.layers
            .push(Layer::pixels(b, "Sky", Raster::rgba(Depth::U8)));
        assert_eq!(doc.locate(a), Some((Some(g), 0)));
        assert!(doc.is_inside(a, g));
        assert_eq!(doc.fresh_name("Layer"), "Layer 1");
        let moved = doc.remove(a).unwrap();
        assert!(doc.insert(None, 0, moved));
        assert_eq!(doc.locate(a), Some((None, 0)));
        assert_eq!(doc.all_layers().len(), 4);
        assert_eq!(search_layers(&doc, "sky"), [b]);
        assert_eq!(search_layers(&doc, "group"), [g]);
        assert_eq!(search_layers(&doc, "").len(), 4);
    }
}
