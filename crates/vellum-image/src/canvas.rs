//! changes to the whole canvas: flips and quarter turns, canvas size, image size, crop
//! (with straighten and perspective) and guides. flips and turns of layers live here too,
//! since they are the same exact pixel shuffles.
//!
//! flips, turns, canvas size and plain crops move whole pixels and never resample, so they
//! are lossless. image size, straightened crops and perspective crops resample through
//! the same transform node as free transform.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::IRect;

use crate::commands::Edit;
use crate::document::{Guide, ImageDocument, Layer};
use crate::raster::{Raster, tile_rect};
use crate::transform::{Interpolation, TransformSpec, quad_to_quad, transform_raster};

/// changes every raster in the document: layers, masks, the selection and saved ones
fn each_raster(
    doc: &mut ImageDocument,
    mut f: impl FnMut(&mut Raster, bool) -> Result<(), String>,
) -> Result<(), String> {
    fn walk(
        list: &mut [Layer],
        f: &mut dyn FnMut(&mut Raster, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        for l in list {
            if let Some(r) = l.raster_mut() {
                f(r, false)?;
            }
            if let Some(m) = &mut l.mask {
                f(&mut m.raster, true)?;
            }
            if let Some(children) = l.children_mut() {
                walk(children, f)?;
            }
        }
        Ok(())
    }
    walk(&mut doc.layers, &mut f)?;
    for r in doc
        .selection
        .iter_mut()
        .chain(doc.last_selection.iter_mut())
    {
        f(r, true)?;
    }
    for c in &mut doc.channels {
        f(&mut c.raster, true)?;
    }
    Ok(())
}

/// a flip or quarter turn, clockwise on screen
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Orient {
    FlipHorizontal,
    FlipVertical,
    Rotate90,
    Rotate180,
    Rotate270,
}

/// whole pixel maps: x' = a x + b y + c, y' = d x + e y + f
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ortho {
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    e: i32,
    f: i32,
}

impl Ortho {
    /// the map for `op` over a box of `w` by `h` at the origin
    fn of(op: Orient, w: i32, h: i32) -> Self {
        let m = |a, b, c, d, e, f| Ortho { a, b, c, d, e, f };
        match op {
            Orient::FlipHorizontal => m(-1, 0, w - 1, 0, 1, 0),
            Orient::FlipVertical => m(1, 0, 0, 0, -1, h - 1),
            Orient::Rotate90 => m(0, -1, h - 1, 1, 0, 0),
            Orient::Rotate180 => m(-1, 0, w - 1, 0, -1, h - 1),
            Orient::Rotate270 => m(0, 1, 0, -1, 0, w - 1),
        }
    }

    /// the same map for a box that starts at (x, y) instead of the origin, landing with
    /// the same centre
    fn around(op: Orient, b: IRect) -> Self {
        let o = Self::of(op, b.w as i32, b.h as i32);
        let turns = matches!(op, Orient::Rotate90 | Orient::Rotate270);
        let (nx, ny) = if turns {
            (
                b.x + (b.w as i32 - b.h as i32).div_euclid(2),
                b.y + (b.h as i32 - b.w as i32).div_euclid(2),
            )
        } else {
            (b.x, b.y)
        };
        // move into the box, map, move out to where the result box sits
        Ortho {
            c: o.c - o.a * b.x - o.b * b.y + nx,
            f: o.f - o.d * b.x - o.e * b.y + ny,
            ..o
        }
    }

    fn map(&self, x: i32, y: i32) -> (i32, i32) {
        (
            self.a * x + self.b * y + self.c,
            self.d * x + self.e * y + self.f,
        )
    }

    fn rect(&self, r: IRect) -> IRect {
        let (x0, y0) = self.map(r.x, r.y);
        let (x1, y1) = self.map(r.right() - 1, r.bottom() - 1);
        IRect::from_corners(x0.min(x1), y0.min(y1), x0.max(x1) + 1, y0.max(y1) + 1)
    }
}

/// a raster moved by a whole pixel map, exactly
fn remap(r: &Raster, m: Ortho) -> Raster {
    let src = r.tile_extent();
    let mut out = r.empty_like();
    if src.is_empty() {
        return out;
    }
    let n = r.channels().n();
    let values = r.read(src);
    let dst = m.rect(src);
    let mut moved = vec![r.fill(); values.len()];
    let dw = dst.w as usize;
    for y in src.y..src.bottom() {
        for x in src.x..src.right() {
            let s = (((y - src.y) as usize) * src.w as usize + (x - src.x) as usize) * n;
            let (tx, ty) = m.map(x, y);
            let d = (((ty - dst.y) as usize) * dw + (tx - dst.x) as usize) * n;
            moved[d..d + n].copy_from_slice(&values[s..s + n]);
        }
    }
    out.write(dst, &moved);
    out
}

/// flips or turns the whole canvas, every layer with it
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientCanvas {
    pub op: Orient,
}

impl Edit for OrientCanvas {
    const KIND: &'static str = "image.canvas.orient";
    const TITLE: &'static str = "Flip or rotate canvas";
    const DESCRIPTION: &'static str = "Flip the whole picture or turn it by a quarter or a half.";
    fn name(&self) -> String {
        match self.op {
            Orient::FlipHorizontal => "Flip canvas horizontal",
            Orient::FlipVertical => "Flip canvas vertical",
            Orient::Rotate90 => "Rotate 90° clockwise",
            Orient::Rotate180 => "Rotate 180°",
            Orient::Rotate270 => "Rotate 90° counter clockwise",
        }
        .to_owned()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let (w, h) = (doc.width as i32, doc.height as i32);
        let m = Ortho::of(self.op, w, h);
        each_raster(doc, |r, _| {
            *r = remap(r, m);
            Ok(())
        })?;
        let (wf, hf) = (w as f32, h as f32);
        for g in &mut doc.guides {
            let (vertical, position) = match (self.op, g.vertical) {
                (Orient::FlipHorizontal, true) | (Orient::Rotate180, true) => {
                    (true, wf - g.position)
                }
                (Orient::FlipVertical, false) | (Orient::Rotate180, false) => {
                    (false, hf - g.position)
                }
                (Orient::Rotate90, true) => (false, g.position),
                (Orient::Rotate90, false) => (true, hf - g.position),
                (Orient::Rotate270, true) => (false, wf - g.position),
                (Orient::Rotate270, false) => (true, g.position),
                _ => (g.vertical, g.position),
            };
            g.vertical = vertical;
            g.position = position;
        }
        if matches!(self.op, Orient::Rotate90 | Orient::Rotate270) {
            std::mem::swap(&mut doc.width, &mut doc.height);
        }
        // text, shapes and smart objects turn with the canvas and stay editable
        let m = match self.op {
            Orient::FlipHorizontal => [-1.0, 0.0, 0.0, 1.0, wf, 0.0],
            Orient::FlipVertical => [1.0, 0.0, 0.0, -1.0, 0.0, hf],
            Orient::Rotate180 => [-1.0, 0.0, 0.0, -1.0, wf, hf],
            Orient::Rotate90 => [0.0, 1.0, -1.0, 0.0, hf, 0.0],
            Orient::Rotate270 => [0.0, -1.0, 1.0, 0.0, 0.0, wf],
        };
        crate::vector_ops::transform_vectors(doc, &m);
        Ok(())
    }
}

/// flips or turns the active layer (or group) about the middle of its pixels
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientLayer {
    pub op: Orient,
}

impl Edit for OrientLayer {
    const KIND: &'static str = "image.layer.orient";
    const TITLE: &'static str = "Flip or rotate layer";
    const DESCRIPTION: &'static str = "Flip the current layer or turn it by a quarter or a half.";
    fn name(&self) -> String {
        match self.op {
            Orient::FlipHorizontal => "Flip layer horizontal",
            Orient::FlipVertical => "Flip layer vertical",
            _ => "Rotate layer",
        }
        .to_owned()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = doc.active.ok_or("Pick a layer first.")?;
        let layer = doc.layer(id).ok_or("Pick a layer first.")?;
        if layer.locks.all || layer.locks.position || layer.locks.pixels {
            return Err(format!(
                "{} is locked, so it cannot be flipped or turned.",
                layer.name
            ));
        }
        let b = layer.bounds();
        if b.is_empty() {
            return Err(format!("{} has no pixels to flip.", layer.name));
        }
        let m = Ortho::around(self.op, b);
        fn walk(l: &mut Layer, m: Ortho) {
            if let Some(r) = l.raster_mut() {
                *r = remap(r, m);
            }
            if let Some(mask) = &mut l.mask
                && mask.linked
            {
                mask.raster = remap(&mask.raster, m);
            }
            for c in l.children_mut().into_iter().flatten() {
                walk(c, m);
            }
        }
        if let Some(l) = doc.layer_mut(id) {
            walk(l, m);
        }
        Ok(())
    }
}

/// a raster with everything outside `rect` cleared to its fill value
pub fn clip_to(r: &Raster, rect: IRect) -> Raster {
    let mut out = r.clone();
    let fill = r.fill();
    for (tx, ty) in r.tile_keys() {
        let t = tile_rect(tx, ty);
        if rect.contains_rect(&t) {
            continue;
        }
        if t.intersect(&rect).is_empty() {
            out.set_tile(tx, ty, None);
            continue;
        }
        let gray = r.channels() == crate::raster::Channels::Gray;
        out.update(t, |x, y, px| {
            if !rect.contains(x, y) {
                if gray {
                    px[0] = fill;
                } else {
                    px.fill(0.0);
                }
            }
        });
    }
    out
}

fn shift_all(doc: &mut ImageDocument, dx: i32, dy: i32) -> Result<(), String> {
    each_raster(doc, |r, _| {
        *r = crate::commands::shifted(r, dx, dy);
        Ok(())
    })?;
    crate::vector_ops::transform_vectors(doc, &[1.0, 0.0, 0.0, 1.0, dx as f32, dy as f32]);
    for g in &mut doc.guides {
        g.position += if g.vertical { dx as f32 } else { dy as f32 };
    }
    Ok(())
}

fn check_size(w: u32, h: u32) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("The picture needs to be at least one pixel each way.".into());
    }
    if w > 300_000 || h > 300_000 || u64::from(w) * u64::from(h) > 4_000_000_000 {
        return Err("That is bigger than Vellum can hold. Try a smaller size.".into());
    }
    Ok(())
}

/// changes the canvas around the picture without scaling it
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasSize {
    pub width: u32,
    pub height: u32,
    /// where the picture stays: 0 left or top, 0.5 the middle, 1 right or bottom
    #[serde(default = "middle")]
    pub anchor: [f32; 2],
    /// straight srgb colour for the new area of the bottom layer, none for see through
    #[serde(default)]
    pub fill: Option<[f32; 4]>,
}

fn middle() -> [f32; 2] {
    [0.5, 0.5]
}

impl Edit for CanvasSize {
    const KIND: &'static str = "image.canvas.size";
    const TITLE: &'static str = "Canvas size";
    const DESCRIPTION: &'static str =
        "Add or take away canvas around the picture without scaling it.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        check_size(self.width, self.height)?;
        if (self.width, self.height) == (doc.width, doc.height) {
            return Err("The canvas is already that size.".into());
        }
        let ax = self.anchor[0].clamp(0.0, 1.0);
        let ay = self.anchor[1].clamp(0.0, 1.0);
        let dx = ((self.width as f32 - doc.width as f32) * ax).round() as i32;
        let dy = ((self.height as f32 - doc.height as f32) * ay).round() as i32;
        let old = IRect::new(dx, dy, doc.width, doc.height);
        shift_all(doc, dx, dy)?;
        doc.width = self.width;
        doc.height = self.height;
        if let Some(c) = self.fill.filter(|c| c[3] > 0.0) {
            let canvas = doc.canvas();
            let color = crate::paint::doc_color(doc, [c[0], c[1], c[2]]);
            if let Some(r) = doc.layers.first_mut().and_then(Layer::raster_mut) {
                r.update(canvas, |x, y, px| {
                    if !old.contains(x, y) && px[3] == 0.0 {
                        px[..3].copy_from_slice(&color);
                        px[3] = c[3];
                    }
                });
            }
        }
        Ok(())
    }
}

/// scales the whole picture to a new size
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub interpolation: Interpolation,
    /// pixels per inch to record, if it changes
    #[serde(default)]
    pub resolution: Option<f32>,
}

/// a spec that maps the canvas through `m`, which a projective map is fixed by
fn spec_for(
    canvas: IRect,
    m: &[[f64; 3]; 3],
    interpolation: Interpolation,
) -> Result<TransformSpec, String> {
    let mut spec = TransformSpec::identity(canvas);
    spec.interpolation = interpolation;
    for c in &mut spec.corners {
        let (x, y) = crate::transform::apply(m, f64::from(c[0]), f64::from(c[1]))
            .ok_or("That shape reaches past the horizon.")?;
        *c = [x as f32, y as f32];
    }
    Ok(spec)
}

/// every raster mapped through `spec`; masks and selections are smoothed bilinearly
fn transform_all(
    doc: &mut ImageDocument,
    spec: &TransformSpec,
    clip: Option<IRect>,
) -> Result<(), String> {
    let working = doc.working();
    let gray_spec = TransformSpec {
        interpolation: match spec.interpolation {
            Interpolation::Nearest => Interpolation::Nearest,
            _ => Interpolation::Bilinear,
        },
        ..spec.clone()
    };
    each_raster(doc, |r, gray| {
        let s = if gray { &gray_spec } else { spec };
        *r = transform_raster(r, s, None, working, clip)?;
        Ok(())
    })
}

impl Edit for ImageSize {
    const KIND: &'static str = "image.image.size";
    const TITLE: &'static str = "Image size";
    const DESCRIPTION: &'static str = "Scale the whole picture to a new size.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        check_size(self.width, self.height)?;
        if let Some(res) = self.resolution.filter(|r| r.is_finite() && *r > 0.0) {
            doc.resolution = res.clamp(1.0, 100_000.0);
        }
        if (self.width, self.height) == (doc.width, doc.height) {
            return if self.resolution.is_some() {
                Ok(())
            } else {
                Err("The picture is already that size.".into())
            };
        }
        let sx = f64::from(self.width) / f64::from(doc.width);
        let sy = f64::from(self.height) / f64::from(doc.height);
        let m = [[sx, 0.0, 0.0], [0.0, sy, 0.0], [0.0, 0.0, 1.0]];
        let spec = spec_for(doc.canvas(), &m, self.interpolation)?;
        transform_all(doc, &spec, None)?;
        for g in &mut doc.guides {
            g.position *= if g.vertical { sx as f32 } else { sy as f32 };
        }
        doc.width = self.width;
        doc.height = self.height;
        crate::vector_ops::transform_vectors(doc, &[sx as f32, 0.0, 0.0, sy as f32, 0.0, 0.0]);
        Ok(())
    }
}

/// crops to a box, which may be turned (straighten) or be any four corners (perspective)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    /// the box kept: x, y, width and height in document pixels, before turning
    pub rect: [f32; 4],
    /// degrees the box is turned clockwise about its middle
    #[serde(default)]
    pub angle: f32,
    /// four corners (top left, top right, bottom right, bottom left) that become the
    /// corners of the result, for perspective crop; `rect` then gives the result's size
    #[serde(default)]
    pub perspective: Option<[[f32; 2]; 4]>,
    /// throw away pixels outside the crop, like photoshop's default
    #[serde(default = "yes")]
    pub delete_outside: bool,
    #[serde(default)]
    pub interpolation: Interpolation,
}

fn yes() -> bool {
    true
}

impl Crop {
    /// the corners of the turned box, in document pixels
    pub fn corners(&self) -> [[f64; 2]; 4] {
        if let Some(q) = self.perspective {
            return q.map(|p| [f64::from(p[0]), f64::from(p[1])]);
        }
        let [x, y, w, h] = self.rect.map(f64::from);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (s, c) = f64::from(self.angle).to_radians().sin_cos();
        [(-w, -h), (w, -h), (w, h), (-w, h)].map(|(dx, dy)| {
            let (dx, dy) = (dx / 2.0, dy / 2.0);
            [cx + dx * c - dy * s, cy + dx * s + dy * c]
        })
    }
}

impl Edit for Crop {
    const KIND: &'static str = "image.crop";
    const TITLE: &'static str = "Crop";
    const DESCRIPTION: &'static str =
        "Cut the picture down to a box, straightening it or fixing its perspective on the way.";
    fn name(&self) -> String {
        if self.perspective.is_some() {
            "Perspective crop".into()
        } else {
            "Crop".into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let w = self.rect[2].round().max(0.0) as u32;
        let h = self.rect[3].round().max(0.0) as u32;
        check_size(w, h)?;
        let canvas = IRect::of_size(w, h);
        let turned = self.angle.abs() > 1e-4 || self.perspective.is_some();
        if turned {
            let to = [
                [0.0, 0.0],
                [f64::from(w), 0.0],
                [f64::from(w), f64::from(h)],
                [0.0, f64::from(h)],
            ];
            let from = self.corners();
            let m = quad_to_quad(&from, &to).ok_or(
                "That crop folds over itself. Move its corners so it stays the right way out.",
            )?;
            // a straightened crop is a turn and a move, which vector layers can follow; a
            // perspective crop bends them, so they become pixels first
            let affine = m[2][0].abs() < 1e-9 && m[2][1].abs() < 1e-9;
            if !affine {
                crate::vector_ops::rasterize_vectors(doc)?;
            }
            let mut spec = TransformSpec::identity(canvas);
            spec.interpolation = self.interpolation;
            spec.source_quad = Some(from.map(|p| [p[0] as f32, p[1] as f32]));
            transform_all(doc, &spec, self.delete_outside.then_some(canvas))?;
            // guides would no longer line up with anything
            doc.guides.clear();
            if affine {
                let n = m[2][2];
                let a = [
                    (m[0][0] / n) as f32,
                    (m[1][0] / n) as f32,
                    (m[0][1] / n) as f32,
                    (m[1][1] / n) as f32,
                    (m[0][2] / n) as f32,
                    (m[1][2] / n) as f32,
                ];
                doc.width = w;
                doc.height = h;
                crate::vector_ops::transform_vectors(doc, &a);
            }
        } else {
            let dx = -(self.rect[0].round() as i32);
            let dy = -(self.rect[1].round() as i32);
            shift_all(doc, dx, dy)?;
            if self.delete_outside {
                each_raster(doc, |r, _| {
                    *r = clip_to(r, canvas);
                    Ok(())
                })?;
            }
            doc.guides.retain(|g| {
                g.position >= 0.0 && g.position <= if g.vertical { w as f32 } else { h as f32 }
            });
        }
        doc.width = w;
        doc.height = h;
        if doc
            .selection
            .as_ref()
            .is_some_and(|s| s.content_bounds().is_empty())
        {
            doc.selection = None;
        }
        Ok(())
    }
}

/// adds a guide
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddGuide {
    pub vertical: bool,
    pub position: f32,
}

impl Edit for AddGuide {
    const KIND: &'static str = "image.guide.add";
    const TITLE: &'static str = "New guide";
    const DESCRIPTION: &'static str = "Add a guide line across the canvas.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if !self.position.is_finite() {
            return Err("A guide needs a position.".into());
        }
        let id = doc.guides.iter().map(|g| g.id).max().unwrap_or(0) + 1;
        doc.guides.push(Guide {
            id,
            vertical: self.vertical,
            position: self.position,
        });
        Ok(())
    }
}

/// moves a guide, or removes it when it is dragged back onto the ruler
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveGuide {
    pub id: u32,
    /// none removes the guide
    pub position: Option<f32>,
}

impl Edit for MoveGuide {
    const KIND: &'static str = "image.guide.move";
    const TITLE: &'static str = "Move guide";
    const DESCRIPTION: &'static str = "Move a guide, or remove it.";
    fn name(&self) -> String {
        if self.position.is_some() {
            "Move guide".into()
        } else {
            "Delete guide".into()
        }
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let i = doc
            .guides
            .iter()
            .position(|g| g.id == self.id)
            .ok_or("That guide is gone.")?;
        match self.position.filter(|p| p.is_finite()) {
            Some(p) => doc.guides[i].position = p,
            None => {
                doc.guides.remove(i);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClearGuides {}

impl Edit for ClearGuides {
    const KIND: &'static str = "image.guide.clear";
    const TITLE: &'static str = "Clear guides";
    const DESCRIPTION: &'static str = "Remove every guide.";
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if doc.guides.is_empty() {
            return Err("There are no guides to clear.".into());
        }
        doc.guides.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Depth;

    fn marked(w: u32, h: u32) -> ImageDocument {
        let mut doc = ImageDocument::new("t", w, h, Depth::U8, None);
        // one red pixel near the top left corner
        let r = doc.layers[0].raster_mut().unwrap();
        r.write(IRect::new(1, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
        doc
    }

    fn red_at(doc: &ImageDocument) -> (i32, i32) {
        let b = doc.layers[0].raster().unwrap().content_bounds();
        (b.x, b.y)
    }

    #[test]
    fn quarter_turns_and_flips_land_pixels_exactly() {
        for (op, size, at) in [
            (Orient::FlipHorizontal, (10, 6), (8, 0)),
            (Orient::FlipVertical, (10, 6), (1, 5)),
            (Orient::Rotate90, (6, 10), (5, 1)),
            (Orient::Rotate180, (10, 6), (8, 5)),
            (Orient::Rotate270, (6, 10), (0, 8)),
        ] {
            let mut doc = marked(10, 6);
            doc.guides.push(Guide {
                id: 1,
                vertical: true,
                position: 2.0,
            });
            OrientCanvas { op }.run(&mut doc).unwrap();
            assert_eq!((doc.width, doc.height), size, "{op:?}");
            assert_eq!(red_at(&doc), at, "{op:?}");
        }
        // four quarter turns come back to where they started
        let mut doc = marked(10, 6);
        for _ in 0..4 {
            OrientCanvas {
                op: Orient::Rotate90,
            }
            .run(&mut doc)
            .unwrap();
        }
        assert_eq!(red_at(&doc), (1, 0));
    }

    #[test]
    fn a_turned_guide_follows_the_picture() {
        let mut doc = marked(10, 6);
        doc.guides.push(Guide {
            id: 1,
            vertical: false,
            position: 2.0,
        });
        OrientCanvas {
            op: Orient::Rotate90,
        }
        .run(&mut doc)
        .unwrap();
        assert!(doc.guides[0].vertical);
        assert_eq!(doc.guides[0].position, 4.0);
    }

    #[test]
    fn layer_flips_stay_in_place() {
        let mut doc = ImageDocument::new("t", 20, 20, Depth::U8, None);
        let r = doc.layers[0].raster_mut().unwrap();
        r.write(IRect::new(4, 4, 3, 2), &[1.0; 24]);
        r.write(IRect::new(4, 4, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
        OrientLayer {
            op: Orient::FlipHorizontal,
        }
        .run(&mut doc)
        .unwrap();
        let r = doc.layers[0].raster().unwrap();
        assert_eq!(r.content_bounds(), IRect::new(4, 4, 3, 2));
        assert_eq!(r.get(6, 4, 1), 0.0);
        OrientLayer {
            op: Orient::Rotate90,
        }
        .run(&mut doc)
        .unwrap();
        let b = doc.layers[0].raster().unwrap().content_bounds();
        assert_eq!((b.w, b.h), (2, 3));
    }

    #[test]
    fn canvas_size_anchors_and_fills() {
        let mut doc = marked(10, 6);
        CanvasSize {
            width: 20,
            height: 10,
            anchor: [1.0, 0.0],
            fill: Some([0.0, 0.0, 1.0, 1.0]),
        }
        .run(&mut doc)
        .unwrap();
        let r = doc.layers[0].raster().unwrap();
        assert_eq!(r.get(11, 0, 0), 1.0, "the picture moved right");
        assert_eq!(r.get(0, 0, 2), 1.0, "the new area is blue");
        assert_eq!(
            r.get(15, 3, 3),
            0.0,
            "the old see through area stayed see through"
        );
    }

    #[test]
    fn crops_shift_clip_and_straighten() {
        let mut doc = ImageDocument::new("t", 40, 30, Depth::U8, Some([0.0, 1.0, 0.0, 1.0]));
        Crop {
            rect: [10.0, 5.0, 20.0, 10.0],
            angle: 0.0,
            perspective: None,
            delete_outside: true,
            interpolation: Interpolation::Bicubic,
        }
        .run(&mut doc)
        .unwrap();
        assert_eq!((doc.width, doc.height), (20, 10));
        assert_eq!(
            doc.layers[0].raster().unwrap().content_bounds(),
            IRect::new(0, 0, 20, 10)
        );
        // a turned crop inside an opaque picture stays opaque in its middle
        let mut doc = ImageDocument::new("t", 64, 64, Depth::U8, Some([0.0, 1.0, 0.0, 1.0]));
        Crop {
            rect: [16.0, 16.0, 32.0, 32.0],
            angle: 10.0,
            perspective: None,
            delete_outside: true,
            interpolation: Interpolation::Bilinear,
        }
        .run(&mut doc)
        .unwrap();
        let r = doc.layers[0].raster().unwrap();
        assert!(r.get(16, 16, 3) > 0.99);
        assert!(r.content_bounds().w <= 32);
    }

    #[test]
    fn image_size_scales_everything() {
        let mut doc = ImageDocument::new("t", 40, 20, Depth::U8, Some([1.0; 4]));
        doc.guides.push(Guide {
            id: 1,
            vertical: true,
            position: 10.0,
        });
        ImageSize {
            width: 20,
            height: 10,
            interpolation: Interpolation::Bilinear,
            resolution: None,
        }
        .run(&mut doc)
        .unwrap();
        assert_eq!((doc.width, doc.height), (20, 10));
        assert_eq!(doc.guides[0].position, 5.0);
        let b = doc.layers[0].raster().unwrap().content_bounds();
        assert!(b.w <= 21 && b.h <= 11, "{b:?}");
    }

    #[test]
    fn guides_add_move_and_go() {
        let mut doc = ImageDocument::new("t", 10, 10, Depth::U8, None);
        AddGuide {
            vertical: true,
            position: 3.0,
        }
        .run(&mut doc)
        .unwrap();
        AddGuide {
            vertical: false,
            position: 4.0,
        }
        .run(&mut doc)
        .unwrap();
        MoveGuide {
            id: 1,
            position: Some(5.0),
        }
        .run(&mut doc)
        .unwrap();
        assert_eq!(doc.guides[0].position, 5.0);
        MoveGuide {
            id: 2,
            position: None,
        }
        .run(&mut doc)
        .unwrap();
        assert_eq!(doc.guides.len(), 1);
        ClearGuides {}.run(&mut doc).unwrap();
        assert!(ClearGuides {}.run(&mut doc).is_err());
    }
}
