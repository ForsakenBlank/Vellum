//! free transform: scale, rotate, skew, distort, perspective and warp, for a layer, the
//! selected pixels of a layer, or the selection outline itself.
//!
//! a transform is described by where the corners of a box land (which covers everything
//! up to perspective) or by a warp: a bezier patch of sixteen control points. both become
//! one `TransformNode` in a render graph. the live preview puts that node into the
//! document's graph (see [`crate::compose`]) and committing renders the very same small
//! graph on the processor, so what is previewed is what lands in the document.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::engine::RenderRequest;
use vellum_gpu::graph::{Graph, Node, NodeId};
use vellum_gpu::nodes::{self, transform::mesh_grid};
use vellum_gpu::{BackendChoice, BlendMode, IRect, Precision, WorkingSpace, render_tiled};

use crate::commands::Edit;
use crate::compose::{Part, RasterSource};
use crate::document::{ImageDocument, Layer, LayerId};
use crate::paint;
use crate::raster::{Channels, Raster};

/// how pixels are picked when a picture is moved off the pixel grid
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Interpolation {
    /// hard pixels, for pixel art and exact turns
    Nearest,
    Bilinear,
    /// smooth and sharp, the default
    #[default]
    Bicubic,
}

impl Interpolation {
    pub fn id(self) -> &'static str {
        match self {
            Interpolation::Nearest => "nearest",
            Interpolation::Bilinear => "bilinear",
            Interpolation::Bicubic => "bicubic",
        }
    }
}

/// where a box of the document goes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TransformSpec {
    /// the box being transformed: x, y, width and height in document pixels
    pub source: [f32; 4],
    /// where its corners land: top left, top right, bottom right, bottom left
    pub corners: [[f32; 2]; 4],
    /// a warp instead of the corners: sixteen bezier control points, four rows of four
    #[serde(default)]
    pub warp: Option<Vec<[f32; 2]>>,
    /// any four points to map onto the corners instead of the box's own, which is how a
    /// perspective crop straightens a quad
    #[serde(default)]
    pub source_quad: Option<[[f32; 2]; 4]>,
    #[serde(default)]
    pub interpolation: Interpolation,
}

type M3 = [[f64; 3]; 3];

fn mul(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn det(m: &M3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

pub fn invert(m: &M3) -> Option<M3> {
    let d = det(m);
    if d.abs() < 1e-12 {
        return None;
    }
    let c =
        |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    Some([
        [c(1, 1, 2, 2) / d, -c(0, 1, 2, 2) / d, c(0, 1, 1, 2) / d],
        [-c(1, 0, 2, 2) / d, c(0, 0, 2, 2) / d, -c(0, 0, 1, 2) / d],
        [c(1, 0, 2, 1) / d, -c(0, 0, 2, 1) / d, c(0, 0, 1, 1) / d],
    ])
}

/// a point through a matrix, or none when it lands behind the viewer of a perspective
pub fn apply(m: &M3, x: f64, y: f64) -> Option<(f64, f64)> {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    if w <= 1e-9 {
        return None;
    }
    Some((
        (m[0][0] * x + m[0][1] * y + m[0][2]) / w,
        (m[1][0] * x + m[1][1] * y + m[1][2]) / w,
    ))
}

/// the projective map from the unit square onto a quad (top left, top right, bottom
/// right, bottom left), heckbert's closed form. none for a quad that folds over.
fn square_to_quad(q: &[[f64; 2]; 4]) -> Option<M3> {
    let [p0, p1, p2, p3] = *q;
    let sx = p0[0] - p1[0] + p2[0] - p3[0];
    let sy = p0[1] - p1[1] + p2[1] - p3[1];
    let m = if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
        [
            [p1[0] - p0[0], p3[0] - p0[0], p0[0]],
            [p1[1] - p0[1], p3[1] - p0[1], p0[1]],
            [0.0, 0.0, 1.0],
        ]
    } else {
        let (dx1, dx2) = (p1[0] - p2[0], p3[0] - p2[0]);
        let (dy1, dy2) = (p1[1] - p2[1], p3[1] - p2[1]);
        let d = dx1 * dy2 - dx2 * dy1;
        if d.abs() < 1e-12 {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / d;
        let h = (dx1 * sy - sx * dy1) / d;
        [
            [p1[0] - p0[0] + g * p1[0], p3[0] - p0[0] + h * p3[0], p0[0]],
            [p1[1] - p0[1] + g * p1[1], p3[1] - p0[1] + h * p3[1], p0[1]],
            [g, h, 1.0],
        ]
    };
    // the quad must keep its winding everywhere, or the picture would fold through itself
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let ws: Vec<f64> = corners
        .iter()
        .map(|(u, v)| m[2][0] * u + m[2][1] * v + m[2][2])
        .collect();
    if ws.iter().any(|w| *w <= 1e-9) || det(&m).abs() < 1e-12 {
        return None;
    }
    Some(m)
}

/// the map from one quad to another
pub fn quad_to_quad(from: &[[f64; 2]; 4], to: &[[f64; 2]; 4]) -> Option<M3> {
    let a = square_to_quad(from)?;
    let b = square_to_quad(to)?;
    Some(mul(&b, &invert(&a)?))
}

fn rect_quad(r: [f32; 4]) -> [[f64; 2]; 4] {
    let [x, y, w, h] = r.map(f64::from);
    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

impl TransformSpec {
    /// a transform that changes nothing
    pub fn identity(source: IRect) -> Self {
        let s = [
            source.x as f32,
            source.y as f32,
            source.w as f32,
            source.h as f32,
        ];
        let q = rect_quad(s).map(|p| [p[0] as f32, p[1] as f32]);
        Self {
            source: s,
            corners: q,
            warp: None,
            source_quad: None,
            interpolation: Interpolation::Bicubic,
        }
    }

    /// the forward matrix, document to document, for the corners
    pub fn matrix(&self) -> Result<M3, String> {
        if self.source[2] <= 0.0 || self.source[3] <= 0.0 {
            return Err("There is nothing to transform.".into());
        }
        let to = self.corners.map(|p| [f64::from(p[0]), f64::from(p[1])]);
        let from = match self.source_quad {
            Some(q) => q.map(|p| [f64::from(p[0]), f64::from(p[1])]),
            None => rect_quad(self.source),
        };
        quad_to_quad(&from, &to).ok_or_else(|| {
            "That shape folds over itself. Move the corners so the box stays the right way out."
                .to_owned()
        })
    }

    pub fn is_identity(&self) -> bool {
        self.warp.is_none()
            && self.source_quad.is_none()
            && rect_quad(self.source)
                .iter()
                .zip(&self.corners)
                .all(|(a, b)| {
                    (a[0] - f64::from(b[0])).abs() < 1e-4 && (a[1] - f64::from(b[1])).abs() < 1e-4
                })
    }

    /// the 16 warp control points, row by row
    fn warp_points(&self) -> Option<&[[f32; 2]]> {
        self.warp.as_deref().filter(|w| w.len() == 16)
    }

    /// a point of the warp patch at (u, v) in [0, 1]
    pub fn warp_at(points: &[[f32; 2]], u: f32, v: f32) -> [f32; 2] {
        let b = |t: f32| {
            let s = 1.0 - t;
            [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
        };
        let (bu, bv) = (b(u), b(v));
        let mut out = [0f32; 2];
        for (j, wv) in bv.iter().enumerate() {
            for (i, wu) in bu.iter().enumerate() {
                let p = points[j * 4 + i];
                out[0] += p[0] * wu * wv;
                out[1] += p[1] * wu * wv;
            }
        }
        out
    }

    /// the box around where the transformed `input` lands
    pub fn output_rect(&self, input: IRect) -> Result<IRect, String> {
        if input.is_empty() {
            return Ok(IRect::EMPTY);
        }
        let pts: Vec<(f64, f64)> = if let Some(w) = self.warp_points() {
            // the patch stays inside the hull of its control points
            w.iter()
                .map(|p| (f64::from(p[0]), f64::from(p[1])))
                .collect()
        } else {
            let m = self.matrix()?;
            let r = IRect::new(input.x, input.y, input.w, input.h);
            let corners = [
                (r.x, r.y),
                (r.right(), r.y),
                (r.right(), r.bottom()),
                (r.x, r.bottom()),
            ];
            corners
                .iter()
                .map(|&(x, y)| {
                    apply(&m, f64::from(x), f64::from(y))
                        .ok_or("That shape reaches past the horizon.")
                })
                .collect::<Result<_, _>>()?
        };
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let lim = 1e7;
        if x1 - x0 > lim || y1 - y0 > lim {
            return Err("That would make the picture far too big.".into());
        }
        Ok(IRect::from_corners(
            x0.floor() as i32 - 2,
            y0.floor() as i32 - 2,
            x1.ceil() as i32 + 2,
            y1.ceil() as i32 + 2,
        ))
    }

    /// the transform node itself, reading from `input`
    pub fn node(&self, g: &mut Graph, input: NodeId) -> Result<NodeId, String> {
        let node = Node::of(nodes::TransformNode).param("interpolation", self.interpolation.id());
        let node = match self.warp_points() {
            Some(points) => {
                const N: usize = 33;
                let mut mesh = Vec::with_capacity(N * N);
                for j in 0..N {
                    for i in 0..N {
                        let (u, v) = (i as f32 / (N - 1) as f32, j as f32 / (N - 1) as f32);
                        mesh.push(Self::warp_at(points, u, v));
                    }
                }
                node.data(mesh_grid(self.source, N, N, &mesh))
            }
            None => {
                let m = self.matrix()?;
                let flat: [f32; 9] = [
                    m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2],
                ]
                .map(|v| v as f32);
                node.param("matrix", flat)
            }
        };
        Ok(g.add(node.input(0, input)))
    }
}

/// a colour raster's pixels transformed, with only the selected part moving when
/// `selection` is given: the rest stays where it was and the moved part lands over it
pub fn pixels_node(
    g: &mut Graph,
    pixels: NodeId,
    selection: Option<NodeId>,
    spec: &TransformSpec,
) -> Result<NodeId, String> {
    let Some(sel) = selection else {
        return spec.node(g, pixels);
    };
    let lifted = g.add(Node::of(nodes::MaskNode).input(0, pixels).input(1, sel));
    let rest = g.add(
        Node::of(nodes::MaskNode)
            .param("invert", true)
            .input(0, pixels)
            .input(1, sel),
    );
    let moved = spec.node(g, lifted)?;
    Ok(g.add(
        Node::of(nodes::BlendNode)
            .param("mode", nodes::blend::mode_param(BlendMode::Normal))
            .input(0, rest)
            .input(1, moved),
    ))
}

/// a grey raster (mask or selection) transformed, as opaque grey with `fill` wherever
/// nothing lands
pub fn gray_node(
    g: &mut Graph,
    gray: NodeId,
    fill: f32,
    spec: &TransformSpec,
) -> Result<NodeId, String> {
    let moved = spec.node(g, gray)?;
    let under = g.add(Node::of(nodes::SolidNode).param("color", [fill, fill, fill, 1.0]));
    Ok(g.add(
        Node::of(nodes::BlendNode)
            .param("mode", nodes::blend::mode_param(BlendMode::Normal))
            .input(0, under)
            .input(1, moved),
    ))
}

/// renders `node` over `rect` on the processor into a raster like `like`. colour comes
/// back straight; grey reads the red channel, which `gray_node` made opaque.
pub fn render_into(
    g: &Graph,
    node: NodeId,
    rect: IRect,
    like: &Raster,
    working: WorkingSpace,
) -> Result<Raster, String> {
    let mut out = like.empty_like();
    if rect.is_empty() {
        return Ok(out);
    }
    let gray = like.channels() == Channels::Gray;
    let req = RenderRequest::new(node, rect)
        .working(working)
        .precision(Precision::F32)
        .backend(BackendChoice::Cpu);
    render_tiled(crate::flatten::engine(), g, &req, 1024, |r, img| {
        let values: Vec<f32> = if gray {
            img.data.iter().map(|p| p[0]).collect()
        } else {
            img.data
                .iter()
                .flat_map(|p| {
                    let a = p[3];
                    if a > 0.0 {
                        [p[0] / a, p[1] / a, p[2] / a, a]
                    } else {
                        [0.0; 4]
                    }
                })
                .collect()
        };
        out.write(r, &values);
        Ok(())
    })
    .map_err(|err| format!("The transform could not be drawn: {err}"))?;
    Ok(out)
}

fn source_node(g: &mut Graph, raster: &Raster) -> NodeId {
    g.add(Node::of(nodes::ImageNode).source(std::sync::Arc::new(RasterSource::new(raster.clone()))))
}

/// a whole raster mapped through `spec`, clipped to `clip` if given
pub fn transform_raster(
    raster: &Raster,
    spec: &TransformSpec,
    selection: Option<&Raster>,
    working: WorkingSpace,
    clip: Option<IRect>,
) -> Result<Raster, String> {
    let content = raster.content_bounds();
    if content.is_empty() {
        return Ok(raster.clone());
    }
    let mut g = Graph::new();
    let src = source_node(&mut g, raster);
    let (node, moved) = if raster.channels() == Channels::Gray {
        (
            gray_node(&mut g, src, raster.fill(), spec)?,
            spec.output_rect(content),
        )
    } else {
        let sel = selection.map(|s| source_node(&mut g, s));
        let node = pixels_node(&mut g, src, sel, spec)?;
        let from = selection.map_or(content, |s| s.content_bounds().intersect(&content));
        // with a selection, the pixels that stay put are part of the result too
        let moved = spec.output_rect(from).map(|r| {
            if selection.is_some() {
                r.union(&content)
            } else {
                r
            }
        });
        (node, moved)
    };
    // a perspective can throw far corners past the horizon; then only the clip is drawn
    let rect = match (moved, clip) {
        (Ok(r), Some(c)) => r.intersect(&c),
        (Ok(r), None) => r,
        (Err(_), Some(c)) => c,
        (Err(e), None) => return Err(e),
    };
    render_into(&g, node, rect, raster, working)
}

/// what a free transform changes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TransformTarget {
    /// the active layer (its mask when that is being edited), or only its selected
    /// pixels when there is a selection
    #[default]
    Layer,
    /// the selection outline, not the pixels
    Selection,
}

/// the parts a transform changes, and whether the selected pixels float on their own
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub parts: Vec<Part>,
    pub lift: bool,
}

fn pixel_layers(layer: &Layer, out: &mut Vec<Part>) {
    if layer.raster().is_some() {
        out.push(Part::Pixels(layer.id));
        if layer.mask.as_ref().is_some_and(|m| m.linked) {
            out.push(Part::Mask(layer.id));
        }
    }
    for c in layer.children() {
        pixel_layers(c, out);
    }
}

/// works out what Ctrl+T would change in `doc`
pub fn plan(doc: &ImageDocument, target: TransformTarget) -> Result<Plan, String> {
    if target == TransformTarget::Selection {
        if doc.selection.is_none() {
            return Err("Select something first, then transform the selection.".into());
        }
        return Ok(Plan {
            parts: vec![Part::Selection],
            lift: false,
        });
    }
    let id: LayerId = doc.active.ok_or("Pick a layer to transform first.")?;
    let layer = doc.layer(id).ok_or("Pick a layer to transform first.")?;
    if layer.locks.all || layer.locks.position || layer.locks.pixels {
        return Err(format!(
            "{} is locked, so it cannot be transformed.",
            layer.name
        ));
    }
    if doc.target == crate::document::EditTarget::Mask && layer.mask.is_some() {
        return Ok(Plan {
            parts: vec![Part::Mask(id)],
            lift: false,
        });
    }
    if layer.is_group() {
        let mut parts = Vec::new();
        pixel_layers(layer, &mut parts);
        return Ok(Plan { parts, lift: false });
    }
    if doc.selection.is_some() {
        return Ok(Plan {
            parts: vec![Part::Pixels(id), Part::Selection],
            lift: true,
        });
    }
    let mut parts = Vec::new();
    pixel_layers(layer, &mut parts);
    Ok(Plan { parts, lift: false })
}

/// the box Ctrl+T starts from: the selection, or the layer's pixels
pub fn start_box(doc: &ImageDocument, target: TransformTarget) -> Result<IRect, String> {
    let plan = plan(doc, target)?;
    let mut out = IRect::EMPTY;
    if plan.lift || target == TransformTarget::Selection {
        return Ok(doc
            .selection
            .as_ref()
            .map(Raster::content_bounds)
            .unwrap_or(IRect::EMPTY));
    }
    for p in &plan.parts {
        out = out.union(
            &paint::part_raster(doc, p)
                .map(Raster::content_bounds)
                .unwrap_or(IRect::EMPTY),
        );
    }
    Ok(out)
}

/// every new raster the transform makes, ready to put in place
pub fn prepare(
    doc: &ImageDocument,
    target: TransformTarget,
    spec: &TransformSpec,
) -> Result<Vec<(Part, Raster, Raster)>, String> {
    let plan = plan(doc, target)?;
    let working = doc.working();
    let selection = if plan.lift {
        doc.selection.clone()
    } else {
        None
    };
    let mut out = Vec::new();
    for part in plan.parts {
        let base = paint::raster_for(doc, &part)?;
        let spec = match part {
            // outlines and masks never need more than smooth edges
            Part::Selection | Part::Mask(_) if spec.interpolation == Interpolation::Bicubic => {
                TransformSpec {
                    interpolation: Interpolation::Bilinear,
                    ..spec.clone()
                }
            }
            _ => spec.clone(),
        };
        let result = match part {
            Part::Pixels(_) => transform_raster(&base, &spec, selection.as_ref(), working, None)?,
            _ => transform_raster(&base, &spec, None, working, None)?,
        };
        out.push((part, base, result));
    }
    Ok(out)
}

/// free transform, warp and transform selection, as one undoable step
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    pub spec: TransformSpec,
    #[serde(default)]
    pub target: TransformTarget,
    /// the results worked out before the command ran, with the rasters they came from
    #[serde(skip)]
    pub prepared: Option<Vec<(Part, Raster, Raster)>>,
}

impl Edit for Transform {
    const KIND: &'static str = "image.transform.apply";
    const TITLE: &'static str = "Free transform";
    const DESCRIPTION: &'static str =
        "Scale, turn, skew, distort or warp a layer, the selected pixels or the selection.";
    fn name(&self) -> String {
        match (self.target, self.spec.warp.is_some()) {
            (TransformTarget::Selection, _) => "Transform selection",
            (_, true) => "Warp",
            _ => "Free transform",
        }
        .to_owned()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        if self.spec.is_identity() {
            return Err("The transform does not change anything.".into());
        }
        let ready = self.prepared.take().filter(|parts| {
            parts.iter().all(|(part, base, _)| {
                paint::raster_for(doc, part).is_ok_and(|now| now.same_as(base))
            })
        });
        let parts = match ready {
            Some(p) => p,
            None => prepare(doc, self.target, &self.spec)?,
        };
        for (part, _, result) in parts {
            paint::set_part(doc, &part, result);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Depth;

    #[test]
    fn corners_give_the_matching_matrix() {
        let mut spec = TransformSpec::identity(IRect::new(10, 20, 100, 50));
        assert!(spec.is_identity());
        // scale by two about the top left corner
        spec.corners = [[10.0, 20.0], [210.0, 20.0], [210.0, 120.0], [10.0, 120.0]];
        let m = spec.matrix().unwrap();
        let (x, y) = apply(&m, 60.0, 45.0).unwrap();
        assert!((x - 110.0).abs() < 1e-6 && (y - 70.0).abs() < 1e-6);
        // a perspective quad maps its own corners exactly
        spec.corners = [[0.0, 0.0], [100.0, 10.0], [90.0, 80.0], [5.0, 60.0]];
        let m = spec.matrix().unwrap();
        for (s, d) in rect_quad(spec.source).iter().zip(&spec.corners) {
            let (x, y) = apply(&m, s[0], s[1]).unwrap();
            assert!((x - f64::from(d[0])).abs() < 1e-6 && (y - f64::from(d[1])).abs() < 1e-6);
        }
        // a bow tie folds over and is refused
        spec.corners = [[0.0, 0.0], [100.0, 100.0], [100.0, 0.0], [0.0, 100.0]];
        assert!(spec.matrix().is_err());
    }

    #[test]
    fn a_flat_warp_matches_its_corners() {
        let spec = TransformSpec::identity(IRect::new(0, 0, 30, 30));
        let pts: Vec<[f32; 2]> = (0..16)
            .map(|k| [(k % 4) as f32 * 10.0, (k / 4) as f32 * 10.0])
            .collect();
        let p = TransformSpec::warp_at(&pts, 0.5, 0.25);
        assert!((p[0] - 15.0).abs() < 1e-4 && (p[1] - 7.5).abs() < 1e-4);
        assert!(
            spec.output_rect(IRect::new(0, 0, 30, 30))
                .unwrap()
                .contains_rect(&IRect::new(0, 0, 30, 30))
        );
    }

    #[test]
    fn moving_selected_pixels_leaves_a_hole_and_keeps_the_rest() {
        let mut doc = ImageDocument::new("t", 64, 64, Depth::U8, Some([1.0, 0.0, 0.0, 1.0]));
        let mut sel = Raster::gray(Depth::U8, 0.0);
        sel.write(IRect::new(0, 0, 16, 16), &[1.0; 256]);
        doc.selection = Some(sel);
        let mut spec = TransformSpec::identity(IRect::new(0, 0, 16, 16));
        spec.interpolation = Interpolation::Nearest;
        spec.corners = [[32.0, 32.0], [48.0, 32.0], [48.0, 48.0], [32.0, 48.0]];
        let mut edit = Transform {
            spec,
            target: TransformTarget::Layer,
            prepared: None,
        };
        edit.run(&mut doc).unwrap();
        let r = doc.layers[0].raster().unwrap();
        assert_eq!(r.get(4, 4, 3), 0.0, "the selected pixels left");
        assert_eq!(r.get(40, 40, 0), 1.0, "and landed here");
        assert_eq!(r.get(60, 5, 3), 1.0, "the rest stayed");
        let moved = doc.selection.as_ref().unwrap().content_bounds();
        assert_eq!(
            moved,
            IRect::new(32, 32, 16, 16),
            "the selection went along"
        );
    }

    #[test]
    fn masks_keep_their_fill_outside_what_moved() {
        let mut mask = Raster::gray(Depth::U8, 1.0);
        mask.write(IRect::new(0, 0, 10, 10), &[0.0; 100]);
        let mut spec = TransformSpec::identity(IRect::new(0, 0, 10, 10));
        spec.corners = [[20.0, 0.0], [30.0, 0.0], [30.0, 10.0], [20.0, 10.0]];
        spec.interpolation = Interpolation::Nearest;
        let out = transform_raster(&mask, &spec, None, WorkingSpace::SRGB, None).unwrap();
        assert_eq!(out.get(25, 5, 0), 0.0);
        assert_eq!(out.get(5, 5, 0), 1.0);
        assert_eq!(out.get(200, 200, 0), 1.0);
    }
}
