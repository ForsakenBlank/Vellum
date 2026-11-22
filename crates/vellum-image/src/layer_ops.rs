//! commands for the layer kinds phase 6 added: adjustment and fill layers, smart objects
//! and their filters, destructive filters, and rasterizing. like every image command they
//! undo by keeping the document before and after (see [`crate::commands`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use vellum_gpu::graph::{Graph, Node};
use vellum_gpu::{BlendMode, IRect, ParamValue};

use crate::adjustments::{self, Adjustment, AutoKind, LutTable};
use crate::commands::Edit;
use crate::compose::{RasterSource, registry};
use crate::document::{Content, EditTarget, ImageDocument, Layer, LayerId, LayerMask};
use crate::fills::FillLayer;
use crate::flatten;
use crate::raster::{Depth, Raster};
use crate::smart::{self, SmartFilter, SmartObject};

fn find(doc: &mut ImageDocument, id: Option<LayerId>) -> Result<&mut Layer, String> {
    let id = id.or(doc.active).ok_or("Pick a layer first.")?;
    doc.layer_mut(id)
        .ok_or_else(|| "That layer is gone.".to_owned())
}

/// puts a new layer above the active one, makes it active, and masks it with the
/// selection if there is one (photoshop does the same for adjustment and fill layers)
fn place_above(doc: &mut ImageDocument, mut layer: Layer, mask_from_selection: bool) -> LayerId {
    if mask_from_selection && let Some(sel) = &doc.selection {
        let mut raster = sel.to_depth(doc.depth);
        raster.set_fill(0.0);
        layer.mask = Some(LayerMask {
            raster,
            enabled: true,
            linked: true,
        });
    }
    let id = layer.id;
    let (parent, index) = match doc.active.and_then(|a| doc.locate(a)) {
        Some((parent, i)) => (parent, i + 1),
        None => (None, doc.layers.len()),
    };
    doc.insert(parent, index, layer);
    doc.active = Some(id);
    doc.target = EditTarget::Pixels;
    id
}

fn blank(doc: &mut ImageDocument, name: &str, content: Content) -> Layer {
    let id = doc.new_id();
    let name = doc.fresh_name(name);
    Layer {
        content,
        ..Layer::pixels(id, name, Raster::rgba(doc.depth))
    }
}

/// how an adjustment layer's settings are worked out when it is made
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Derive {
    Auto(AutoKind),
    Equalize,
    Enhance,
}

/// adds an adjustment layer above the current layer
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AddAdjustment {
    pub kind: String,
    pub params: BTreeMap<String, ParamValue>,
    pub lut: Option<LutTable>,
    pub name: Option<String>,
    /// clip it to the layer below, so it only changes that layer
    pub clip: bool,
    pub derive: Option<Derive>,
}

impl Edit for AddAdjustment {
    const KIND: &'static str = "image.adjustment.add";
    const TITLE: &'static str = "New adjustment layer";
    const DESCRIPTION: &'static str =
        "Add an adjustment layer that changes the colours of the layers below.";
    fn name(&self) -> String {
        format!("New {} layer", adjustments::title_of(&self.kind))
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let (kind, title) = match self.derive {
            Some(Derive::Enhance) => ("effect.camera_raw".to_owned(), "Enhance"),
            Some(Derive::Auto(AutoKind::Tone)) => ("effect.curves".to_owned(), "Auto Tone"),
            Some(Derive::Auto(AutoKind::Contrast)) => ("effect.curves".to_owned(), "Auto Contrast"),
            Some(Derive::Auto(AutoKind::Color)) => ("effect.curves".to_owned(), "Auto Color"),
            Some(Derive::Equalize) => ("effect.curves".to_owned(), "Equalize"),
            None => (self.kind.clone(), adjustments::title_of(&self.kind)),
        };
        if registry().get(&kind).is_none() && kind != "color.lut" {
            return Err(format!("Vellum has no adjustment called {kind}."));
        }
        let mut adj = Adjustment::new(kind);
        adj.params = self.params.clone();
        adj.lut = self.lut.clone();
        if adj.kind == "color.lut" && adj.lut.is_none() {
            adj.lut = Some(LutTable {
                name: adjustments::LOOKS[0].into(),
                cube: adjustments::look_cube(adjustments::LOOKS[0]),
            });
        }
        if let Some(d) = self.derive {
            // work out the settings from what the new layer would sit on
            let mut probe = doc.clone();
            probe.selection = None;
            let l = blank(&mut probe, "probe", Content::Adjustment(adj.clone()));
            let pid = place_above(&mut probe, l, false);
            let below = adjustments::below(&probe, pid);
            let derived = match d {
                Derive::Auto(k) => adjustments::auto_curves(&below, k)?,
                Derive::Equalize => adjustments::equalize_curve(&below)?,
                Derive::Enhance => adjustments::enhance(&below)?,
            };
            adj.params.extend(derived);
        }
        if adj.kind == "effect.match_color" && !adj.params.contains_key("source_mean") {
            // matching against itself changes nothing until a source is picked
            let probe = doc.clone();
            let stats = adjustments::match_stats(&probe, &probe).unwrap_or_default();
            adj.params.extend(stats);
        }
        let name = self.name.clone().unwrap_or_else(|| title.to_owned());
        let mut layer = blank(doc, &name, Content::Adjustment(adj));
        layer.name = doc.fresh_name(&name);
        layer.clipped = self.clip;
        place_above(doc, layer, true);
        Ok(())
    }
}

/// changes an adjustment layer's settings. slider drags merge into one step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SetAdjustment {
    pub id: Option<LayerId>,
    /// values to change; others stay
    pub params: BTreeMap<String, ParamValue>,
    /// replace every value instead, eg when a preset is picked
    pub replace: bool,
    pub lut: Option<LutTable>,
}

impl Edit for SetAdjustment {
    const KIND: &'static str = "image.adjustment.set";
    const TITLE: &'static str = "Change adjustment";
    const DESCRIPTION: &'static str = "Change an adjustment layer's settings.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let layer = find(doc, self.id)?;
        let Content::Adjustment(adj) = &mut layer.content else {
            return Err(format!("{} is not an adjustment layer.", layer.name));
        };
        if self.replace {
            adj.params = self.params.clone();
        } else {
            adj.params.extend(self.params.clone());
        }
        if let Some(lut) = &self.lut {
            adjustments::parse_cube(&lut.cube)?;
            adj.lut = Some(lut.clone());
        }
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        !self.replace
            && !next.replace
            && self.id == next.id
            && self.lut.is_none()
            && next.lut.is_none()
            && self.params.keys().eq(next.params.keys())
    }
}

/// works out an existing adjustment layer's settings from what is below it
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeriveAdjustment {
    pub id: Option<LayerId>,
    pub derive: Derive,
}

impl Edit for DeriveAdjustment {
    const KIND: &'static str = "image.adjustment.auto";
    const TITLE: &'static str = "Auto";
    const DESCRIPTION: &'static str = "Work out an adjustment's settings from the picture.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.or(doc.active).ok_or("Pick a layer first.")?;
        let below = adjustments::below(doc, id);
        let layer = find(doc, Some(id))?;
        let Content::Adjustment(adj) = &mut layer.content else {
            return Err(format!("{} is not an adjustment layer.", layer.name));
        };
        let derived = match (self.derive, adj.kind.as_str()) {
            (Derive::Enhance, "effect.camera_raw") => adjustments::enhance(&below)?,
            (Derive::Auto(k), "effect.curves") => adjustments::auto_curves(&below, k)?,
            (Derive::Equalize, "effect.curves") => adjustments::equalize_curve(&below)?,
            (_, "effect.match_color") => adjustments::match_stats(&below, &below)?,
            _ => {
                return Err(format!(
                    "Auto settings work on Curves and Camera Raw layers, not {}.",
                    adjustments::title_of(&adj.kind)
                ));
            }
        };
        adj.params.extend(derived);
        Ok(())
    }
}

/// sets match colour's source to another layer's look
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchColorFrom {
    pub id: LayerId,
    pub source: LayerId,
}

impl Edit for MatchColorFrom {
    const KIND: &'static str = "image.adjustment.match_from";
    const TITLE: &'static str = "Match colour";
    const DESCRIPTION: &'static str = "Copy the overall colour of another layer.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let src_layer = doc.layer(self.source).ok_or("That layer is gone.")?.clone();
        let mut src = doc.clone();
        src.layers = vec![Layer {
            visible: true,
            ..src_layer
        }];
        let target = adjustments::below(doc, self.id);
        let stats = adjustments::match_stats(&src, &target)?;
        let layer = find(doc, Some(self.id))?;
        let Content::Adjustment(adj) = &mut layer.content else {
            return Err("Match colour needs a Match Color adjustment layer.".into());
        };
        adj.params.extend(stats);
        Ok(())
    }
}

/// adds a fill layer
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddFill {
    pub fill: FillLayer,
}

impl Edit for AddFill {
    const KIND: &'static str = "image.fill.add";
    const TITLE: &'static str = "New fill layer";
    const DESCRIPTION: &'static str = "Add a layer filled with a colour, gradient or pattern.";
    fn name(&self) -> String {
        format!("New {}", self.fill.title())
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let title = self.fill.title();
        let layer = blank(doc, title, Content::Fill(self.fill.clone()));
        place_above(doc, layer, true);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFill {
    pub id: Option<LayerId>,
    pub fill: FillLayer,
}

impl Edit for SetFill {
    const KIND: &'static str = "image.fill.set";
    const TITLE: &'static str = "Change fill";
    const DESCRIPTION: &'static str = "Change a fill layer's colour, gradient or pattern.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let layer = find(doc, self.id)?;
        let Content::Fill(f) = &mut layer.content else {
            return Err(format!("{} is not a fill layer.", layer.name));
        };
        *f = self.fill.clone();
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        self.id == next.id
            && std::mem::discriminant(&self.fill) == std::mem::discriminant(&next.fill)
    }
}

/// what a layer looks like on its own, as pixels in document coordinates: its content
/// with everything but its own look switched off
fn render_alone(doc: &ImageDocument, layer: &Layer) -> Result<Raster, String> {
    let mut solo = doc.clone();
    solo.quick_mask = false;
    solo.layers = vec![Layer {
        visible: true,
        opacity: 1.0,
        fill: 1.0,
        blend: BlendMode::Normal,
        clipped: false,
        mask: None,
        // the style and vector mask stay on the layer, so they are not drawn into it
        vector_mask: None,
        style: None,
        ..layer.clone()
    }];
    let rect = match &layer.content {
        Content::Fill(_) => doc.canvas(),
        _ => layer.bounds().union(&IRect::EMPTY),
    };
    flatten::merged(&solo, rect, doc.depth)
}

/// turns a layer (or group) into a smart object, keeping its pixels and look
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConvertToSmart {
    pub id: Option<LayerId>,
}

impl Edit for ConvertToSmart {
    const KIND: &'static str = "image.smart.convert";
    const TITLE: &'static str = "Convert to smart object";
    const DESCRIPTION: &'static str =
        "Keep the layer's pixels safe through transforms and filters.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.or(doc.active).ok_or("Pick a layer first.")?;
        let layer = doc.layer(id).ok_or("That layer is gone.")?.clone();
        if matches!(layer.content, Content::Smart(_)) {
            return Err(format!("{} is already a smart object.", layer.name));
        }
        if matches!(layer.content, Content::Adjustment(_)) {
            return Err("Adjustment layers have no pixels to keep in a smart object.".into());
        }
        let pixels = match &layer.content {
            Content::Pixels(r) => r.clone(),
            _ => render_alone(doc, &layer)?,
        };
        let b = pixels.content_bounds();
        if b.is_empty() {
            return Err(format!(
                "{} is empty, so there is nothing to keep.",
                layer.name
            ));
        }
        let source = crate::commands::shifted(&crate::canvas::clip_to(&pixels, b), -b.x, -b.y);
        let object = SmartObject::new(source, b.w, b.h, [b.x as f32, b.y as f32]);
        let target = doc.layer_mut(id).ok_or("That layer is gone.")?;
        target.content = Content::Smart(Box::new(object));
        Ok(())
    }
}

/// places pixels (usually a file) as a new smart object, embedded or linked
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaceSmart {
    pub path: Option<String>,
    pub name: Option<String>,
    pub linked: bool,
    /// the pixels, read before the command runs so the document is never locked on a file
    #[serde(skip)]
    pub pixels: Option<(Raster, u32, u32)>,
}

impl Edit for PlaceSmart {
    const KIND: &'static str = "image.smart.place";
    const TITLE: &'static str = "Place";
    const DESCRIPTION: &'static str = "Place a picture as a smart object.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if self.pixels.is_none()
            && let Some(path) = &self.path
        {
            self.pixels = Some(read_picture(path, doc.depth)?);
        }
        let (source, w, h) = self.pixels.clone().ok_or("There is no picture to place.")?;
        // centred, shrunk to fit the canvas like photoshop's place
        let k = (doc.width as f32 / w as f32)
            .min(doc.height as f32 / h as f32)
            .min(1.0);
        let mut object = SmartObject::new(source, w, h, [0.0, 0.0]);
        object.transform = [
            k,
            0.0,
            0.0,
            k,
            ((doc.width as f32 - w as f32 * k) / 2.0).round(),
            ((doc.height as f32 - h as f32 * k) / 2.0).round(),
        ];
        if self.linked {
            object.linked = self.path.clone();
        }
        let name = self.name.clone().or_else(|| {
            self.path.as_deref().map(|p| {
                std::path::Path::new(p)
                    .file_stem()
                    .map_or_else(|| p.to_owned(), |s| s.to_string_lossy().into_owned())
            })
        });
        let mut layer = blank(
            doc,
            name.as_deref().unwrap_or("Placed"),
            Content::Smart(Box::new(object)),
        );
        if let Some(n) = name {
            layer.name = n;
        }
        place_above(doc, layer, false);
        Ok(())
    }
}

/// reads a picture file as straight colour at `depth`, for placing
pub fn read_picture(path: &str, depth: Depth) -> Result<(Raster, u32, u32), String> {
    let img = image::open(path).map_err(|err| format!("{path} could not be opened: {err}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((Raster::from_rgba8(w, h, &rgba, depth), w, h))
}

/// swaps a smart object's pixels: an updated linked file, or new contents
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReplaceContents {
    pub id: Option<LayerId>,
    /// read from here; none re-reads a linked object's own file
    pub path: Option<String>,
    #[serde(skip)]
    pub pixels: Option<(Raster, u32, u32)>,
}

impl Edit for ReplaceContents {
    const KIND: &'static str = "image.smart.replace";
    const TITLE: &'static str = "Replace contents";
    const DESCRIPTION: &'static str =
        "Swap a smart object's pixels, or update a linked one from its file.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let depth = doc.depth;
        let layer = find(doc, self.id)?;
        let name = layer.name.clone();
        let object = layer
            .smart_mut()
            .ok_or_else(|| format!("{name} is not a smart object."))?;
        let path = self.path.clone().or_else(|| object.linked.clone());
        if self.pixels.is_none() {
            let p = path
                .as_deref()
                .ok_or("Pick a file to place in the smart object.")?;
            self.pixels = Some(read_picture(p, depth)?);
        }
        let (source, w, h) = self.pixels.clone().ok_or("There is no picture to use.")?;
        // keep the size it was shown at, so a bigger file lands in the same box
        let sx = object.width as f32 / w as f32;
        let sy = object.height as f32 / h as f32;
        let [a, b, c, d, e, f] = object.transform;
        object.transform = [a * sx, b * sx, c * sy, d * sy, e, f];
        object.source = source;
        object.width = w;
        object.height = h;
        if object.linked.is_some() && self.path.is_some() {
            object.linked = self.path.clone();
        }
        Ok(())
    }
}

/// turns a layer into plain pixels: smart objects, fills and (with styles) everything
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Rasterize {
    pub id: Option<LayerId>,
}

impl Edit for Rasterize {
    const KIND: &'static str = "image.layer.rasterize";
    const TITLE: &'static str = "Rasterize layer";
    const DESCRIPTION: &'static str =
        "Turn a smart object, fill, text or shape layer into plain pixels.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.or(doc.active).ok_or("Pick a layer first.")?;
        let layer = doc.layer(id).ok_or("That layer is gone.")?.clone();
        match layer.content {
            Content::Pixels(_) => return Err(format!("{} is already pixels.", layer.name)),
            Content::Group { .. } => {
                return Err("Groups cannot be rasterized; merge the group instead.".into());
            }
            Content::Adjustment(_) => {
                return Err("Adjustment layers have no pixels of their own to keep.".into());
            }
            _ => {}
        }
        let pixels = render_alone(doc, &layer)?;
        let target = doc.layer_mut(id).ok_or("That layer is gone.")?;
        target.content = Content::Pixels(pixels);
        Ok(())
    }
}

/// adds a filter to a smart object
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AddSmartFilter {
    pub id: Option<LayerId>,
    /// the filter gallery entry
    pub entry: String,
    pub params: BTreeMap<String, ParamValue>,
}

impl Edit for AddSmartFilter {
    const KIND: &'static str = "image.smart.filter.add";
    const TITLE: &'static str = "Add smart filter";
    const DESCRIPTION: &'static str = "Add a filter to a smart object that stays editable.";
    fn name(&self) -> String {
        smart::gallery_entry(&self.entry).map_or_else(|| Self::TITLE.into(), |e| e.title)
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let entry = smart::gallery_entry(&self.entry)
            .ok_or_else(|| format!("Vellum has no filter called {}.", self.entry))?;
        let layer = find(doc, self.id)?;
        let name = layer.name.clone();
        let object = layer
            .smart_mut()
            .ok_or_else(|| format!("{name} is not a smart object. Convert it first."))?;
        let mut params = entry.params.clone();
        params.extend(self.params.clone());
        if !params.keys().any(|k| k == "center" || k == "position") {
            // the gallery places points on the object; keep them where it put them
            let b = object.bounds();
            let mid = [b.x as f32 + b.w as f32 / 2.0, b.y as f32 + b.h as f32 / 2.0];
            if let Some(n) = registry().get(&entry.kind) {
                for spec in n.info().params.iter().filter(|s| !s.hidden) {
                    if matches!(spec.id, "center" | "position") {
                        params.insert(spec.id.into(), ParamValue::Point(mid));
                    }
                }
            }
        }
        let fid = object.next_filter.max(1);
        object.next_filter = fid + 1;
        object.filters.push(SmartFilter {
            id: fid,
            entry: entry.id,
            kind: entry.kind,
            params,
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
        });
        if object.filter_mask.is_none() {
            object.filter_mask = Some(Raster::gray(doc_depth_hint(), 1.0));
        }
        Ok(())
    }
}

/// filter masks are 8 bit, like photoshop's
fn doc_depth_hint() -> Depth {
    Depth::U8
}

/// changes one smart filter
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SetSmartFilter {
    pub id: Option<LayerId>,
    pub filter: u32,
    pub params: BTreeMap<String, ParamValue>,
    pub visible: Option<bool>,
    pub opacity: Option<f32>,
    pub blend: Option<BlendMode>,
    /// move it to this position in the list, 0 first
    pub index: Option<usize>,
    pub remove: bool,
    /// shows or hides every filter at once
    pub all_visible: Option<bool>,
}

impl Edit for SetSmartFilter {
    const KIND: &'static str = "image.smart.filter.set";
    const TITLE: &'static str = "Change smart filter";
    const DESCRIPTION: &'static str = "Change, reorder, hide or remove a smart filter.";
    fn name(&self) -> String {
        if self.remove {
            "Remove smart filter".into()
        } else {
            Self::TITLE.into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let layer = find(doc, self.id)?;
        let object = layer
            .smart_mut()
            .ok_or("That layer is not a smart object.")?;
        if let Some(v) = self.all_visible {
            object.filters_visible = v;
        }
        if self.filter == 0 {
            return Ok(());
        }
        let i = object
            .filters
            .iter()
            .position(|f| f.id == self.filter)
            .ok_or("That filter is gone.")?;
        if self.remove {
            object.filters.remove(i);
            if object.filters.is_empty() {
                object.filter_mask = None;
            }
            return Ok(());
        }
        let f = &mut object.filters[i];
        f.params.extend(self.params.clone());
        if let Some(v) = self.visible {
            f.visible = v;
        }
        if let Some(o) = self.opacity {
            f.opacity = o.clamp(0.0, 1.0);
        }
        if let Some(b) = self.blend {
            f.blend = b;
        }
        if let Some(to) = self.index {
            let f = object.filters.remove(i);
            let to = to.min(object.filters.len());
            object.filters.insert(to, f);
        }
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        self.id == next.id
            && self.filter == next.filter
            && !self.remove
            && !next.remove
            && self.index.is_none()
            && next.index.is_none()
            && self.params.keys().eq(next.params.keys())
            && self.opacity.is_some() == next.opacity.is_some()
    }
}

/// runs a filter on the current layer's pixels for good, inside the selection. on a smart
/// object it adds a smart filter instead, like photoshop.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApplyFilter {
    pub entry: String,
    pub params: BTreeMap<String, ParamValue>,
}

impl Edit for ApplyFilter {
    const KIND: &'static str = "image.filter.apply";
    const TITLE: &'static str = "Filter";
    const DESCRIPTION: &'static str = "Run a filter on the current layer.";
    fn name(&self) -> String {
        smart::gallery_entry(&self.entry).map_or_else(|| Self::TITLE.into(), |e| e.title)
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = doc.active.ok_or("Pick a layer first.")?;
        if doc.layer(id).is_some_and(|l| l.smart().is_some()) {
            return AddSmartFilter {
                id: Some(id),
                entry: self.entry.clone(),
                params: self.params.clone(),
            }
            .run(doc);
        }
        let entry = smart::gallery_entry(&self.entry)
            .ok_or_else(|| format!("Vellum has no filter called {}.", self.entry))?;
        let part = crate::paint::active_part(doc)?;
        let base = crate::paint::raster_for(doc, &part)?;
        let mut params = entry.params.clone();
        params.extend(self.params.clone());
        let out = filter_raster(doc, &base, &entry.kind, &params)?;
        let out = match &doc.selection {
            Some(sel) => through_selection(&base, &out, sel),
            None => out,
        };
        crate::paint::set_part(doc, &part, out);
        Ok(())
    }
}

/// a raster run through a filter node, over the canvas and wherever it has pixels
pub fn filter_raster(
    doc: &ImageDocument,
    base: &Raster,
    kind: &str,
    params: &BTreeMap<String, ParamValue>,
) -> Result<Raster, String> {
    let canvas = doc.canvas();
    let area = base.content_bounds();
    let mut g = Graph::new();
    let gray = base.channels() == crate::raster::Channels::Gray;
    let src = g.add(
        Node::of(vellum_gpu::nodes::ImageNode)
            .source(std::sync::Arc::new(RasterSource::new(base.clone()))),
    );
    let anchor = if area.is_empty() { canvas } else { area };
    let node = smart::filter_node(kind, params, anchor, canvas, registry())
        .ok_or_else(|| format!("Vellum has no filter node called {kind}."))?;
    let top = g.add(node.input(0, src));
    let rect = canvas.union(&area);
    let rendered = flatten::node_to_raster(&g, top, rect, base.depth(), doc.working())?;
    if !gray {
        return Ok(rendered);
    }
    // masks and the quick mask are grey: keep the filtered brightness
    let mut out = Raster::gray(base.depth(), base.fill());
    let px = rendered.read(rect);
    let values: Vec<f32> = px
        .chunks_exact(4)
        .map(|p| {
            (0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]) * p[3] + base.fill() * (1.0 - p[3])
        })
        .collect();
    out.write(rect, &values);
    Ok(out)
}

/// `after` where the selection is, `before` elsewhere, faded across soft edges
fn through_selection(before: &Raster, after: &Raster, sel: &Raster) -> Raster {
    let mut out = before.clone();
    let rect = before.content_bounds().union(&after.content_bounds());
    let n = before.channels().n();
    out.update(rect, |x, y, px| {
        let s = sel.get(x, y, 0);
        if s <= 0.0 {
            return;
        }
        for (k, v) in px.iter_mut().enumerate().take(n) {
            *v += (after.get(x, y, k) - *v) * s;
        }
    });
    out
}

/// adds an empty (white) filter mask to a smart object, or targets it for painting
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TargetFilterMask {
    pub id: Option<LayerId>,
}

impl Edit for TargetFilterMask {
    const KIND: &'static str = "image.smart.filter_mask";
    const TITLE: &'static str = "Edit filter mask";
    const DESCRIPTION: &'static str = "Paint where a smart object's filters show.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.or(doc.active).ok_or("Pick a layer first.")?;
        let layer = find(doc, Some(id))?;
        let object = layer
            .smart_mut()
            .ok_or("Only smart objects have filter masks.")?;
        if object.filter_mask.is_none() {
            object.filter_mask = Some(Raster::gray(Depth::U8, 1.0));
        }
        doc.active = Some(id);
        doc.target = EditTarget::FilterMask;
        Ok(())
    }
}

/// sets, pastes or clears layer styles. one layer while editing, several when pasting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SetStyle {
    /// none means the active layer
    pub ids: Vec<LayerId>,
    /// none clears the style
    pub style: Option<crate::styles::LayerStyle>,
    /// pasting names the step differently in the history
    pub paste: bool,
}

impl Edit for SetStyle {
    const KIND: &'static str = "image.style.set";
    const TITLE: &'static str = "Layer style";
    const DESCRIPTION: &'static str = "Change, paste or clear a layer's style.";
    fn name(&self) -> String {
        match (&self.style, self.paste) {
            (None, _) => "Clear layer style".into(),
            (_, true) => "Paste layer style".into(),
            _ => Self::TITLE.into(),
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let ids = if self.ids.is_empty() {
            vec![doc.active.ok_or("Pick a layer first.")?]
        } else {
            self.ids.clone()
        };
        for id in ids {
            let layer = doc.layer_mut(id).ok_or("That layer is gone.")?;
            if matches!(layer.content, Content::Adjustment(_)) && self.style.is_some() {
                return Err("Adjustment layers cannot have a style.".into());
            }
            layer.style = self
                .style
                .clone()
                .filter(|s| *s != crate::styles::LayerStyle::default());
        }
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        !self.paste
            && !next.paste
            && self.ids == next.ids
            && self.style.is_some()
            && next.style.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry as commands;
    use vellum_core::command::CommandRecord;
    use vellum_core::history::{History, HistoryLimits};

    fn run(
        h: &mut History<ImageDocument>,
        doc: &mut ImageDocument,
        kind: &str,
        params: serde_json::Value,
    ) {
        let cmd = commands()
            .instantiate(&CommandRecord::new(kind, params))
            .expect("command");
        h.execute(cmd, doc).expect(kind);
    }

    fn photo() -> ImageDocument {
        let mut doc = ImageDocument::new("t", 64, 48, Depth::U8, Some([0.5, 0.5, 0.5, 1.0]));
        let id = doc.layers[0].id;
        doc.layer_mut(id)
            .and_then(Layer::raster_mut)
            .expect("raster")
            .update(IRect::of_size(64, 48), |x, y, px| {
                px.copy_from_slice(&[x as f32 / 63.0, y as f32 / 47.0, 0.4, 1.0]);
            });
        doc.active = Some(id);
        doc
    }

    fn pixel(doc: &ImageDocument, x: i32, y: i32) -> [f32; 4] {
        flatten::colors(doc, IRect::new(x, y, 1, 1))
            .expect("colors")
            .get(x, y)
    }

    #[test]
    fn adjustment_layers_change_what_is_below_and_undo() {
        let mut doc = photo();
        let mut h = History::new(HistoryLimits::default());
        let before = pixel(&doc, 10, 10);
        run(
            &mut h,
            &mut doc,
            "image.adjustment.add",
            serde_json::json!({ "kind": "effect.invert" }),
        );
        let after = pixel(&doc, 10, 10);
        assert!(
            (after[0] - (1.0 - before[0])).abs() < 0.01,
            "{before:?} {after:?}"
        );
        let id = doc.active.expect("active");
        assert!(matches!(
            doc.layer(id).expect("layer").content,
            Content::Adjustment(_)
        ));
        run(
            &mut h,
            &mut doc,
            "image.layer.props",
            serde_json::json!({ "id": id, "opacity": 0.5 }),
        );
        let half = pixel(&doc, 10, 10);
        assert!((half[0] - 0.5 * (before[0] + after[0])).abs() < 0.02);
        h.undo(&mut doc).expect("undo");
        h.undo(&mut doc).expect("undo");
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(pixel(&doc, 10, 10), before);
    }

    #[test]
    fn auto_and_enhance_make_layers() {
        let mut doc = photo();
        let mut h = History::new(HistoryLimits::default());
        for derive in [
            serde_json::json!({ "auto": "tone" }),
            serde_json::json!({ "auto": "contrast" }),
            serde_json::json!({ "auto": "color" }),
            serde_json::json!("equalize"),
            serde_json::json!("enhance"),
        ] {
            run(
                &mut h,
                &mut doc,
                "image.adjustment.add",
                serde_json::json!({ "derive": derive }),
            );
        }
        assert_eq!(doc.layers.len(), 6);
        assert!(
            doc.layers
                .iter()
                .skip(1)
                .all(|l| matches!(l.content, Content::Adjustment(_)))
        );
    }

    #[test]
    fn fills_and_smart_objects_round_trip_the_format() {
        let mut doc = photo();
        let mut h = History::new(HistoryLimits::default());
        run(
            &mut h,
            &mut doc,
            "image.smart.convert",
            serde_json::json!({}),
        );
        run(
            &mut h,
            &mut doc,
            "image.smart.filter.add",
            serde_json::json!({ "entry": "blur.gaussian" }),
        );
        run(
            &mut h,
            &mut doc,
            "image.fill.add",
            serde_json::json!({ "fill": { "type": "solid", "color": [1.0, 0.0, 0.0, 1.0] } }),
        );
        let (state, files) = crate::format::describe(&doc);
        let back = crate::format::assemble(state, &|name| {
            let r = files.iter().find(|f| f.0 == name).expect("file");
            let mut bytes = Vec::new();
            crate::format::write_raster(&r.1, &mut bytes)?;
            Ok(bytes)
        })
        .expect("assemble");
        assert!(back.layers[0].smart().is_some_and(|s| s.filters.len() == 1));
        assert!(matches!(back.layers[1].content, Content::Fill(_)));
        assert_eq!(pixel(&back, 5, 5), pixel(&doc, 5, 5));
    }

    #[test]
    fn smart_objects_keep_quality_through_shrinking() {
        let mut doc = photo();
        let mut h = History::new(HistoryLimits::default());
        run(
            &mut h,
            &mut doc,
            "image.smart.convert",
            serde_json::json!({}),
        );
        let original = pixel(&doc, 40, 30);
        let id = doc.active.expect("active");
        doc.layer_mut(id)
            .and_then(Layer::smart_mut)
            .expect("smart")
            .then([0.25, 0.0, 0.0, 0.25, 0.0, 0.0]);
        doc.layer_mut(id)
            .and_then(Layer::smart_mut)
            .expect("smart")
            .then([4.0, 0.0, 0.0, 4.0, 0.0, 0.0]);
        let back = pixel(&doc, 40, 30);
        for k in 0..3 {
            assert!((back[k] - original[k]).abs() < 0.01);
        }
        run(
            &mut h,
            &mut doc,
            "image.layer.rasterize",
            serde_json::json!({}),
        );
        assert!(doc.layers[0].raster().is_some());
    }

    #[test]
    fn styles_draw_outside_the_layer_and_paste_onto_others() {
        let mut doc = ImageDocument::new("t", 64, 64, Depth::U8, Some([1.0, 1.0, 1.0, 1.0]));
        let mut h = History::new(HistoryLimits::default());
        let id = doc.new_id();
        let mut r = Raster::rgba(Depth::U8);
        r.update(IRect::new(16, 16, 20, 20), |_, _, p| {
            p.copy_from_slice(&[1.0, 0.0, 0.0, 1.0])
        });
        doc.layers.push(Layer::pixels(id, "red", r));
        doc.active = Some(id);
        let before = pixel(&doc, 38, 38);
        let style = crate::styles::LayerStyle {
            drop_shadow: Some(crate::styles::Shadow {
                angle: 135.0,
                distance: 6.0,
                size: 2.0,
                ..crate::styles::default_shadow()
            }),
            stroke: Some(crate::styles::default_stroke()),
            ..Default::default()
        };
        run(
            &mut h,
            &mut doc,
            "image.style.set",
            serde_json::json!({ "style": style }),
        );
        let after = pixel(&doc, 38, 38);
        assert!(
            after[0] < before[0] - 0.2,
            "the shadow darkens below right: {after:?}"
        );
        assert!(
            pixel(&doc, 14, 25)[0] < 0.1,
            "the stroke is black just outside"
        );
        assert!(
            pixel(&doc, 25, 25)[1] < 0.1,
            "the layer is still red inside"
        );
        // fill at zero hides the pixels but keeps the style
        run(
            &mut h,
            &mut doc,
            "image.layer.props",
            serde_json::json!({ "id": id, "fill": 0.0 }),
        );
        assert!(pixel(&doc, 25, 25)[1] > 0.9 && pixel(&doc, 14, 25)[0] < 0.1);
        let base = doc.layers[0].id;
        run(
            &mut h,
            &mut doc,
            "image.style.set",
            serde_json::json!({ "ids": [base], "style": style, "paste": true }),
        );
        assert!(doc.layers[0].style.is_some());
        run(
            &mut h,
            &mut doc,
            "image.style.set",
            serde_json::json!({ "ids": [id] }),
        );
        assert!(doc.layer(id).expect("l").style.is_none());
    }

    #[test]
    fn destructive_filters_respect_the_selection() {
        let mut doc = photo();
        let mut h = History::new(HistoryLimits::default());
        run(
            &mut h,
            &mut doc,
            "image.select.shape",
            serde_json::json!({ "shape": { "type": "rect", "x": 0.0, "y": 0.0, "w": 32.0, "h": 48.0 } }),
        );
        let outside = pixel(&doc, 50, 20);
        run(
            &mut h,
            &mut doc,
            "image.filter.apply",
            serde_json::json!({ "entry": "stylize.find_edges" }),
        );
        assert_eq!(pixel(&doc, 50, 20), outside);
        assert!(pixel(&doc, 10, 20) != outside);
    }
}
