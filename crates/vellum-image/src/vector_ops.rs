//! commands for text layers, shape layers, vector masks and saved paths, and how those
//! layers follow moves, transforms and canvas changes.
//!
//! text and shapes keep their description and redraw their pixels whenever it changes, so
//! scaling a text layer up keeps it sharp. anything that moves or reshapes the document
//! passes its transform here ([`transform_layer`]) instead of resampling their pixels.

use serde::{Deserialize, Serialize};
use vellum_gpu::IRect;

use crate::commands::Edit;
use crate::document::{
    Content, EditTarget, ImageDocument, Layer, LayerId, LayerVectorMask, SavedPath,
};
use crate::fonts::FontRef;
use crate::raster::{Depth, Raster};
use crate::selection::{self, Combine};
use crate::text::TextLayer;
use crate::vector::{Affine, ShapeLayer, VectorMask, VectorPath};

/// straight srgb to the document's straight values
pub fn to_doc(doc: &ImageDocument) -> impl Fn([f32; 4]) -> [f32; 4] + use<> {
    let probe = doc.clone();
    move |c| {
        let rgb = crate::paint::doc_color(&probe, [c[0], c[1], c[2]]);
        [rgb[0], rgb[1], rgb[2], c[3]]
    }
}

/// the area text may draw into: the canvas and a canvas's worth around it
fn text_clip(canvas: IRect) -> IRect {
    canvas.expand(canvas.w.max(canvas.h) as i32)
}

/// draws a text or shape layer's pixels again from its description
pub fn redraw(
    layer: &mut Layer,
    depth: Depth,
    canvas: IRect,
    to_doc: &dyn Fn([f32; 4]) -> [f32; 4],
) {
    match &mut layer.content {
        Content::Text { text, pixels } => {
            let drawn = crate::text::draw(text, depth, text_clip(canvas), to_doc);
            // keep the old pixels when the font is missing, so nothing changes by surprise
            if !(drawn.missing_font && !pixels.is_empty() && text.missing_font) {
                *pixels = drawn.pixels;
            }
            text.missing_font = drawn.missing_font;
        }
        Content::Shape { shape, pixels } => *pixels = shape.render(depth, to_doc),
        _ => {}
    }
}

/// moves, scales, turns or skews a layer's vector content and redraws it. pixel layers
/// are left alone (their caller resamples them). returns true if it handled the layer.
pub fn transform_layer(
    layer: &mut Layer,
    m: &Affine,
    depth: Depth,
    canvas: IRect,
    to_doc: &dyn Fn([f32; 4]) -> [f32; 4],
) -> bool {
    if let Some(v) = &mut layer.vector_mask {
        v.mask.path.transform(m);
        v.raster = v.mask.raster(canvas);
    }
    let handled = match &mut layer.content {
        Content::Text { text, .. } => {
            text.then(m);
            true
        }
        Content::Shape { shape, .. } => {
            shape.transform(m);
            true
        }
        Content::Smart(s) => {
            s.then(*m);
            true
        }
        _ => false,
    };
    redraw(layer, depth, canvas, to_doc);
    handled
}

/// applies a transform to every vector backed layer in the document, groups included
pub fn transform_vectors(doc: &mut ImageDocument, m: &Affine) {
    let f = to_doc(doc);
    let (depth, canvas) = (doc.depth, doc.canvas());
    fn walk(
        list: &mut [Layer],
        m: &Affine,
        depth: Depth,
        canvas: IRect,
        f: &dyn Fn([f32; 4]) -> [f32; 4],
    ) {
        for l in list {
            transform_layer(l, m, depth, canvas, f);
            if let Some(c) = l.children_mut() {
                walk(c, m, depth, canvas, f);
            }
        }
    }
    walk(&mut doc.layers, m, depth, canvas, &f);
    for p in &mut doc.paths {
        p.path.transform(m);
    }
}

/// turns every text, shape and smart object layer into pixels where it stands, before
/// something that bends the whole document (a perspective crop)
pub fn rasterize_vectors(doc: &mut ImageDocument) -> Result<(), String> {
    let ids: Vec<LayerId> = doc
        .all_layers()
        .into_iter()
        .filter(|l| {
            matches!(
                l.content,
                Content::Text { .. } | Content::Shape { .. } | Content::Smart(_)
            )
        })
        .map(|l| l.id)
        .collect();
    for id in ids {
        crate::commands::Edit::run(&mut crate::layer_ops::Rasterize { id: Some(id) }, doc)?;
    }
    Ok(())
}

fn insert_above(doc: &mut ImageDocument, layer: Layer) {
    let id = layer.id;
    let (parent, index) = match doc.active.and_then(|a| doc.locate(a)) {
        Some((parent, i)) => (parent, i + 1),
        None => (None, doc.layers.len()),
    };
    doc.insert(parent, index, layer);
    doc.active = Some(id);
    doc.target = EditTarget::Pixels;
}

fn find(doc: &mut ImageDocument, id: Option<LayerId>) -> Result<&mut Layer, String> {
    let id = id.or(doc.active).ok_or("Pick a layer first.")?;
    doc.layer_mut(id)
        .ok_or_else(|| "That layer is gone.".to_owned())
}

/// a short name for a text layer from its first words, like photoshop
fn text_name(t: &TextLayer) -> String {
    let first: String = t
        .text
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(30)
        .collect();
    if first.trim().is_empty() {
        "Text".into()
    } else {
        first.trim().to_owned()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddText {
    pub text: TextLayer,
}

impl Edit for AddText {
    const KIND: &'static str = "image.text.add";
    const TITLE: &'static str = "New text";
    const DESCRIPTION: &'static str = "Add a text layer.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = doc.new_id();
        let mut layer = Layer {
            content: Content::Text {
                text: Box::new(self.text.clone()),
                pixels: Raster::rgba(doc.depth),
            },
            ..Layer::pixels(id, text_name(&self.text), Raster::rgba(doc.depth))
        };
        redraw(&mut layer, doc.depth, doc.canvas(), &to_doc(doc));
        insert_above(doc, layer);
        Ok(())
    }
}

/// changes a text layer's words or settings. typing and slider drags merge into one step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetText {
    pub id: Option<LayerId>,
    pub text: TextLayer,
}

impl Edit for SetText {
    const KIND: &'static str = "image.text.set";
    const TITLE: &'static str = "Edit text";
    const DESCRIPTION: &'static str = "Change a text layer's words, font or layout.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let (depth, canvas, f) = (doc.depth, doc.canvas(), to_doc(doc));
        let layer = find(doc, self.id)?;
        let auto_name =
            matches!(&layer.content, Content::Text { text, .. } if text_name(text) == layer.name);
        let Content::Text { text, .. } = &mut layer.content else {
            return Err(format!("{} is not a text layer.", layer.name));
        };
        **text = self.text.clone();
        if auto_name {
            layer.name = text_name(&self.text);
        }
        redraw(layer, depth, canvas, &f);
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        self.id == next.id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddShape {
    pub shape: ShapeLayer,
    pub name: Option<String>,
}

impl Edit for AddShape {
    const KIND: &'static str = "image.shape.add";
    const TITLE: &'static str = "New shape";
    const DESCRIPTION: &'static str = "Add a shape layer.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if self.shape.items.is_empty() {
            return Err("Draw a shape first.".into());
        }
        let id = doc.new_id();
        let name = self.name.clone().unwrap_or_else(|| doc.fresh_name("Shape"));
        let mut layer = Layer {
            content: Content::Shape {
                shape: Box::new(self.shape.clone()),
                pixels: Raster::rgba(doc.depth),
            },
            ..Layer::pixels(id, name, Raster::rgba(doc.depth))
        };
        redraw(&mut layer, doc.depth, doc.canvas(), &to_doc(doc));
        insert_above(doc, layer);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetShape {
    pub id: Option<LayerId>,
    pub shape: ShapeLayer,
}

impl Edit for SetShape {
    const KIND: &'static str = "image.shape.set";
    const TITLE: &'static str = "Edit shape";
    const DESCRIPTION: &'static str = "Change a shape layer's paths, fill or stroke.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let (depth, canvas, f) = (doc.depth, doc.canvas(), to_doc(doc));
        let layer = find(doc, self.id)?;
        let Content::Shape { shape, .. } = &mut layer.content else {
            return Err(format!("{} is not a shape layer.", layer.name));
        };
        **shape = self.shape.clone();
        redraw(layer, depth, canvas, &f);
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        self.id == next.id
    }
}

/// adds, changes or removes a layer's vector mask
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetVectorMask {
    pub id: Option<LayerId>,
    pub mask: Option<VectorMask>,
}

impl Edit for SetVectorMask {
    const KIND: &'static str = "image.vector_mask.set";
    const TITLE: &'static str = "Vector mask";
    const DESCRIPTION: &'static str = "Hide parts of a layer with a path.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let canvas = doc.canvas();
        let layer = find(doc, self.id)?;
        if matches!(layer.content, Content::Adjustment(_))
            && self.mask.is_none()
            && layer.vector_mask.is_none()
        {
            return Ok(());
        }
        layer.vector_mask = self.mask.clone().map(|m| LayerVectorMask::new(m, canvas));
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        self.id == next.id && self.mask.is_some() && next.mask.is_some()
    }
}

/// points every text layer that uses one family at another, eg for a missing font
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceFont {
    pub from: String,
    pub to: FontRef,
}

impl Edit for ReplaceFont {
    const KIND: &'static str = "image.text.replace_font";
    const TITLE: &'static str = "Replace font";
    const DESCRIPTION: &'static str = "Use another font wherever this document uses one.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let (depth, canvas, f) = (doc.depth, doc.canvas(), to_doc(doc));
        let mut count = 0;
        fn walk(
            list: &mut [Layer],
            r: &ReplaceFont,
            depth: Depth,
            canvas: IRect,
            f: &dyn Fn([f32; 4]) -> [f32; 4],
            count: &mut usize,
        ) {
            for l in list {
                if let Content::Text { text, .. } = &mut l.content
                    && text.font.family.eq_ignore_ascii_case(&r.from)
                {
                    text.font.family = r.to.family.clone();
                    if r.to.weight != 0 {
                        text.font.weight = r.to.weight;
                    }
                    // a replaced font always redraws, even over the old pixels
                    text.missing_font = false;
                    *count += 1;
                    redraw(l, depth, canvas, f);
                }
                if let Some(c) = l.children_mut() {
                    walk(c, r, depth, canvas, f, count);
                }
            }
        }
        walk(&mut doc.layers, self, depth, canvas, &f, &mut count);
        if count == 0 {
            return Err(format!("No text in this document uses {}.", self.from));
        }
        Ok(())
    }
}

/// draws every text layer again, eg after a missing font was downloaded
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefreshText {}

impl Edit for RefreshText {
    const KIND: &'static str = "image.text.refresh";
    const TITLE: &'static str = "Update text";
    const DESCRIPTION: &'static str = "Draw text layers again with the fonts now installed.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let (depth, canvas, f) = (doc.depth, doc.canvas(), to_doc(doc));
        fn walk(list: &mut [Layer], depth: Depth, canvas: IRect, f: &dyn Fn([f32; 4]) -> [f32; 4]) {
            for l in list {
                if let Content::Text { text, .. } = &mut l.content {
                    text.missing_font = false;
                    redraw(l, depth, canvas, f);
                }
                if let Some(c) = l.children_mut() {
                    walk(c, depth, canvas, f);
                }
            }
        }
        walk(&mut doc.layers, depth, canvas, &f);
        Ok(())
    }
}

/// the families text layers ask for that are not installed
pub fn missing_fonts(doc: &ImageDocument) -> Vec<String> {
    let mut out: Vec<String> = doc
        .all_layers()
        .into_iter()
        .filter_map(|l| match &l.content {
            Content::Text { text, .. } if crate::fonts::find(&text.font).is_none() => {
                Some(text.font.family.clone())
            }
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// keeps a path in the paths panel, or replaces one
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePath {
    pub id: Option<u32>,
    pub name: Option<String>,
    pub path: VectorPath,
}

impl Edit for SavePath {
    const KIND: &'static str = "image.path.save";
    const TITLE: &'static str = "Save path";
    const DESCRIPTION: &'static str = "Keep a path in the paths panel.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if let Some(p) = self
            .id
            .and_then(|id| doc.paths.iter_mut().find(|p| p.id == id))
        {
            p.path = self.path.clone();
            if let Some(n) = &self.name {
                p.name = n.clone();
            }
            return Ok(());
        }
        let id = doc.paths.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        self.id = Some(id);
        doc.paths.push(SavedPath {
            id,
            name: self.name.clone().unwrap_or_else(|| format!("Path {id}")),
            path: self.path.clone(),
        });
        Ok(())
    }
    fn continues(&self, next: &Self) -> bool {
        self.id.is_some() && self.id == next.id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovePath {
    pub id: u32,
}

impl Edit for RemovePath {
    const KIND: &'static str = "image.path.remove";
    const TITLE: &'static str = "Delete path";
    const DESCRIPTION: &'static str = "Remove a path from the paths panel.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let before = doc.paths.len();
        doc.paths.retain(|p| p.id != self.id);
        if doc.paths.len() == before {
            return Err("That path is gone.".into());
        }
        Ok(())
    }
}

/// turns a path into a selection, like photoshop's make selection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathToSelection {
    pub path: VectorPath,
    #[serde(default)]
    pub mode: Combine,
    #[serde(default)]
    pub feather: f32,
}

impl Edit for PathToSelection {
    const KIND: &'static str = "image.path.select";
    const TITLE: &'static str = "Make selection";
    const DESCRIPTION: &'static str = "Select what a path encloses.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let canvas = doc.canvas();
        let cover = self.path.coverage(canvas);
        let mut cover = Some(cover);
        if self.feather > 0.0
            && let Some(c) = &cover
        {
            cover = selection::feather(c, canvas, self.feather)?;
        }
        let new = cover.unwrap_or_else(|| Raster::gray(Depth::U8, 0.0));
        let combined = selection::combine_masks(doc.selection.as_ref(), &new, canvas, self.mode);
        if let Some(old) = doc.selection.take() {
            doc.last_selection = Some(old);
        }
        doc.selection = combined;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry;
    use crate::vector::{Geometry, PathOp, ShapeItem};
    use vellum_core::command::CommandRecord;
    use vellum_core::history::{History, HistoryLimits};

    fn run(
        h: &mut History<ImageDocument>,
        d: &mut ImageDocument,
        kind: &str,
        p: serde_json::Value,
    ) {
        let cmd = registry()
            .instantiate(&CommandRecord::new(kind, p))
            .expect(kind);
        h.execute(cmd, d).expect(kind);
    }

    fn square(x: f32) -> ShapeLayer {
        ShapeLayer {
            items: vec![ShapeItem {
                geometry: Geometry::Rect {
                    x,
                    y: 10.0,
                    w: 20.0,
                    h: 20.0,
                    radii: [0.0; 4],
                },
                op: PathOp::Combine,
            }],
            fill: Some([1.0, 0.0, 0.0, 1.0]),
            stroke: None,
        }
    }

    #[test]
    fn shape_layers_draw_move_and_save() {
        let mut d = ImageDocument::new("t", 64, 64, Depth::U8, Some([1.0; 4]));
        let mut h = History::new(HistoryLimits::default());
        run(
            &mut h,
            &mut d,
            "image.shape.add",
            serde_json::json!({ "shape": square(10.0) }),
        );
        let id = d.active.expect("active");
        assert_eq!(d.layer(id).expect("l").bounds(), IRect::new(10, 10, 20, 20));
        run(
            &mut h,
            &mut d,
            "image.layer.offset",
            serde_json::json!({ "ids": [id], "dx": 5, "dy": 0 }),
        );
        let l = d.layer(id).expect("l");
        assert_eq!(l.bounds(), IRect::new(15, 10, 20, 20));
        assert!(
            matches!(&l.content, Content::Shape { shape, .. } if matches!(shape.items[0].geometry, Geometry::Rect { x, .. } if x == 15.0))
        );
        let (state, files) = crate::format::describe(&d);
        let back = crate::format::assemble(state, &|name| {
            let r = files.iter().find(|f| f.0 == name).expect("file");
            let mut b = Vec::new();
            crate::format::write_raster(&r.1, &mut b)?;
            Ok(b)
        })
        .expect("assemble");
        assert!(matches!(
            back.layer(id).expect("l").content,
            Content::Shape { .. }
        ));
        run(
            &mut h,
            &mut d,
            "image.layer.rasterize",
            serde_json::json!({}),
        );
        assert!(matches!(
            d.layer(id).expect("l").content,
            Content::Pixels(_)
        ));
    }

    #[test]
    fn vector_masks_hide_and_paths_select() {
        let mut d = ImageDocument::new("t", 64, 64, Depth::U8, Some([1.0, 0.0, 0.0, 1.0]));
        let mut h = History::new(HistoryLimits::default());
        let id = d.layers[0].id;
        d.active = Some(id);
        let path = Geometry::Ellipse {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
        }
        .to_path();
        let mask = VectorMask {
            path: path.clone(),
            enabled: true,
            invert: false,
        };
        run(
            &mut h,
            &mut d,
            "image.vector_mask.set",
            serde_json::json!({ "id": id, "mask": mask }),
        );
        let px = |d: &ImageDocument, x, y| {
            crate::flatten::colors(d, IRect::new(x, y, 1, 1))
                .expect("c")
                .get(x, y)
        };
        assert!(px(&d, 16, 16)[3] > 0.99 && px(&d, 50, 50)[3] < 0.01);
        run(
            &mut h,
            &mut d,
            "image.path.save",
            serde_json::json!({ "path": path }),
        );
        let saved = d.paths[0].path.clone();
        run(
            &mut h,
            &mut d,
            "image.path.select",
            serde_json::json!({ "path": saved }),
        );
        assert!(
            d.selection
                .as_ref()
                .is_some_and(|s| s.get(16, 16, 0) > 0.99)
        );
    }

    #[test]
    fn text_layers_redraw_and_report_missing_fonts() {
        if crate::fonts::resolve(&FontRef::default()).is_none() {
            return;
        }
        let mut d = ImageDocument::new("t", 200, 80, Depth::U8, Some([1.0; 4]));
        let mut h = History::new(HistoryLimits::default());
        let t = TextLayer::point("Hello", 10.0, 50.0, 30.0);
        run(
            &mut h,
            &mut d,
            "image.text.add",
            serde_json::json!({ "text": t }),
        );
        let id = d.active.expect("active");
        assert_eq!(d.layer(id).expect("l").name, "Hello");
        let small = d.layer(id).expect("l").bounds();
        let mut bigger = t.clone();
        bigger.size = 60.0;
        bigger.text = "Hello there".into();
        run(
            &mut h,
            &mut d,
            "image.text.set",
            serde_json::json!({ "id": id, "text": bigger }),
        );
        let l = d.layer(id).expect("l");
        assert_eq!(l.name, "Hello there");
        assert!(l.bounds().w > small.w * 2);
        let mut gone = t.clone();
        gone.font.family = "No Such Family Anywhere".into();
        run(
            &mut h,
            &mut d,
            "image.text.add",
            serde_json::json!({ "text": gone }),
        );
        assert_eq!(
            missing_fonts(&d),
            vec!["No Such Family Anywhere".to_owned()]
        );
        run(
            &mut h,
            &mut d,
            "image.text.replace_font",
            serde_json::json!({ "from": "No Such Family Anywhere", "to": FontRef::default() }),
        );
        assert!(missing_fonts(&d).is_empty());
    }

    #[test]
    fn free_transform_keeps_shapes_live_and_bends_them_for_perspective() {
        let mut d = ImageDocument::new("t", 128, 128, Depth::U8, Some([1.0; 4]));
        let mut h = History::new(HistoryLimits::default());
        run(
            &mut h,
            &mut d,
            "image.shape.add",
            serde_json::json!({ "shape": square(10.0) }),
        );
        let id = d.active.expect("active");
        // twice the size about the top left corner
        let spec = serde_json::json!({
            "source": [10.0, 10.0, 20.0, 20.0],
            "corners": [[10.0, 10.0], [50.0, 10.0], [50.0, 50.0], [10.0, 50.0]],
        });
        run(
            &mut h,
            &mut d,
            "image.transform.apply",
            serde_json::json!({ "spec": spec }),
        );
        let l = d.layer(id).expect("l");
        assert!(matches!(l.content, Content::Shape { .. }), "still a shape");
        assert_eq!(l.bounds(), IRect::new(10, 10, 40, 40));
        let spec = serde_json::json!({
            "source": [10.0, 10.0, 40.0, 40.0],
            "corners": [[10.0, 10.0], [50.0, 20.0], [50.0, 40.0], [10.0, 50.0]],
        });
        run(
            &mut h,
            &mut d,
            "image.transform.apply",
            serde_json::json!({ "spec": spec }),
        );
        assert!(
            matches!(d.layer(id).expect("l").content, Content::Pixels(_)),
            "perspective rasterizes"
        );
        h.undo(&mut d).expect("undo");
        assert!(matches!(
            d.layer(id).expect("l").content,
            Content::Shape { .. }
        ));
    }
}
