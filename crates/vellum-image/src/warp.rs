//! liquify and puppet warp. both bend a layer with a mesh: an even grid over the layer
//! whose points can land anywhere. the render engine's transform node draws a mesh on the
//! gpu (phase 5 built that for the warp transform), so dragging a liquify brush or a pin
//! previews at full speed, and committing renders the same mesh on the processor.
//!
//! liquify brushes move the grid points under them; puppet warp places every point with
//! moving least squares (schaefer, mcphail and warren 2006), which bends the layer around
//! its pins as rigidly as it can, like a puppet.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::IRect;
use vellum_gpu::graph::{Graph, Node, NodeData};
use vellum_gpu::nodes::TransformNode;
use vellum_gpu::nodes::transform::mesh_grid;

use crate::commands::Edit;
use crate::compose::RasterSource;
use crate::document::{Content, ImageDocument, LayerId};

/// a grid over `source` (x, y, w, h) with where each point lands, row by row
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Mesh {
    pub source: [f32; 4],
    pub cols: usize,
    pub rows: usize,
    pub points: Vec<[f32; 2]>,
}

/// the grid spacing in document pixels, coarser for big layers so meshes stay small
pub fn spacing_for(area: IRect) -> f32 {
    let long = area.w.max(area.h) as f32;
    (long / 200.0).clamp(8.0, 64.0)
}

impl Mesh {
    /// a mesh that changes nothing over `area`
    pub fn identity(area: IRect, step: f32) -> Self {
        let cols = ((area.w as f32 / step).ceil() as usize + 1).max(2);
        let rows = ((area.h as f32 / step).ceil() as usize + 1).max(2);
        let w = step * (cols - 1) as f32;
        let h = step * (rows - 1) as f32;
        let source = [area.x as f32, area.y as f32, w, h];
        let points = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (c, r)))
            .map(|(c, r)| [source[0] + c as f32 * step, source[1] + r as f32 * step])
            .collect();
        Self {
            source,
            cols,
            rows,
            points,
        }
    }

    /// where grid point (c, r) starts
    pub fn home(&self, i: usize) -> [f32; 2] {
        let (c, r) = (i % self.cols, i / self.cols);
        [
            self.source[0] + self.source[2] * c as f32 / (self.cols - 1) as f32,
            self.source[1] + self.source[3] * r as f32 / (self.rows - 1) as f32,
        ]
    }

    pub fn is_identity(&self) -> bool {
        self.points.iter().enumerate().all(|(i, p)| {
            let h = self.home(i);
            (p[0] - h[0]).abs() < 1e-3 && (p[1] - h[1]).abs() < 1e-3
        })
    }

    /// the data the transform node reads
    pub fn data(&self) -> NodeData {
        mesh_grid(self.source, self.cols, self.rows, &self.points)
    }

    /// the transform node drawing `input` bent by this mesh
    pub fn node(&self) -> Node {
        Node::of(TransformNode)
            .param("interpolation", "bilinear")
            .data(self.data())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LiquifyTool {
    /// pushes pixels along with the brush
    #[default]
    ForwardWarp,
    /// pulls pixels in towards the middle of the brush
    Pucker,
    /// pushes pixels out from the middle of the brush
    Bloat,
    /// paints the picture back to how it was
    Reconstruct,
    /// evens out sharp bends
    Smooth,
    /// turns pixels clockwise around the brush
    Twirl,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct LiquifyBrush {
    pub tool: LiquifyTool,
    /// diameter in document pixels
    pub size: f32,
    /// 0 to 1, how much each step moves
    pub pressure: f32,
    /// 0 to 1, how soft the edge is
    pub density: f32,
}

impl Default for LiquifyBrush {
    fn default() -> Self {
        Self {
            tool: LiquifyTool::ForwardWarp,
            size: 100.0,
            pressure: 0.5,
            density: 0.5,
        }
    }
}

impl Mesh {
    /// one step of a liquify brush at `at`, which moved by `delta` since the last step
    pub fn liquify(&mut self, brush: &LiquifyBrush, at: [f32; 2], delta: [f32; 2]) {
        let radius = (brush.size / 2.0).max(1.0);
        let rate = brush.pressure.clamp(0.0, 1.0);
        let soft = brush.density.clamp(0.0, 1.0);
        let homes: Vec<[f32; 2]> = (0..self.points.len()).map(|i| self.home(i)).collect();
        let before = self.points.clone();
        for (i, p) in self.points.iter_mut().enumerate() {
            let d = (p[0] - at[0]).hypot(p[1] - at[1]);
            if d >= radius {
                continue;
            }
            let t = d / radius;
            // a smooth bump, flatter in the middle when the density is low
            let w = (1.0 - t * t).powf(1.0 + soft * 2.0) * rate;
            match brush.tool {
                LiquifyTool::ForwardWarp => {
                    p[0] += delta[0] * w;
                    p[1] += delta[1] * w;
                }
                LiquifyTool::Pucker | LiquifyTool::Bloat => {
                    let k = if brush.tool == LiquifyTool::Pucker {
                        0.1
                    } else {
                        -0.1
                    };
                    p[0] += (at[0] - p[0]) * w * k;
                    p[1] += (at[1] - p[1]) * w * k;
                }
                LiquifyTool::Reconstruct => {
                    p[0] += (homes[i][0] - p[0]) * w * 0.3;
                    p[1] += (homes[i][1] - p[1]) * w * 0.3;
                }
                LiquifyTool::Smooth => {
                    let (c, r) = (i % self.cols, i / self.cols);
                    let mut acc = [0.0; 2];
                    let mut n = 0.0;
                    for (dc, dr) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let (nc, nr) = (c as i32 + dc, r as i32 + dr);
                        if nc < 0 || nr < 0 || nc >= self.cols as i32 || nr >= self.rows as i32 {
                            continue;
                        }
                        let j = nr as usize * self.cols + nc as usize;
                        // smooth the displacement, not the position
                        acc[0] += before[j][0] - homes[j][0];
                        acc[1] += before[j][1] - homes[j][1];
                        n += 1.0;
                    }
                    let want = [homes[i][0] + acc[0] / n, homes[i][1] + acc[1] / n];
                    p[0] += (want[0] - p[0]) * w * 0.5;
                    p[1] += (want[1] - p[1]) * w * 0.5;
                }
                LiquifyTool::Twirl => {
                    let (s, c) = (w * 0.08).sin_cos();
                    let (dx, dy) = (p[0] - at[0], p[1] - at[1]);
                    p[0] = at[0] + dx * c - dy * s;
                    p[1] = at[1] + dx * s + dy * c;
                }
            }
        }
    }
}

/// a puppet warp pin: dragged from where it was put to where it is now
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Pin {
    pub from: [f32; 2],
    pub to: [f32; 2],
}

/// where `v` lands when the pins move, by rigid moving least squares
pub fn puppet_point(pins: &[Pin], v: [f32; 2]) -> [f32; 2] {
    match pins {
        [] => return v,
        [one] => {
            return [
                v[0] + one.to[0] - one.from[0],
                v[1] + one.to[1] - one.from[1],
            ];
        }
        _ => {}
    }
    let mut ws = Vec::with_capacity(pins.len());
    for p in pins {
        let d2 = (p.from[0] - v[0]).powi(2) + (p.from[1] - v[1]).powi(2);
        if d2 < 1e-8 {
            return p.to;
        }
        ws.push(1.0 / d2);
    }
    let wsum: f32 = ws.iter().sum();
    let mut ps = [0.0; 2];
    let mut qs = [0.0; 2];
    for (p, w) in pins.iter().zip(&ws) {
        for k in 0..2 {
            ps[k] += p.from[k] * w / wsum;
            qs[k] += p.to[k] * w / wsum;
        }
    }
    let d = [v[0] - ps[0], v[1] - ps[1]];
    let mut fr = [0.0; 2];
    for (p, w) in pins.iter().zip(&ws) {
        let (px, py) = (p.from[0] - ps[0], p.from[1] - ps[1]);
        let (qx, qy) = (p.to[0] - qs[0], p.to[1] - qs[1]);
        let a = px * d[0] + py * d[1];
        let b = px * d[1] - py * d[0];
        fr[0] += w * (qx * a - qy * b);
        fr[1] += w * (qx * b + qy * a);
    }
    let len = fr[0].hypot(fr[1]);
    let dl = d[0].hypot(d[1]);
    if len < 1e-12 {
        return [v[0] + qs[0] - ps[0], v[1] + qs[1] - ps[1]];
    }
    [dl * fr[0] / len + qs[0], dl * fr[1] / len + qs[1]]
}

impl Mesh {
    /// places every grid point for these pins
    pub fn puppet(&mut self, pins: &[Pin]) {
        for i in 0..self.points.len() {
            self.points[i] = puppet_point(pins, self.home(i));
        }
    }
}

/// bends a layer's pixels with a mesh (what liquify and puppet warp commit)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshWarp {
    pub id: Option<LayerId>,
    /// "Liquify" or "Puppet warp", for the history
    pub title: String,
    pub mesh: Mesh,
}

impl Edit for MeshWarp {
    const KIND: &'static str = "image.warp.mesh";
    const TITLE: &'static str = "Liquify";
    const DESCRIPTION: &'static str = "Bend a layer's pixels, as liquify and puppet warp do.";
    fn name(&self) -> String {
        self.title.clone()
    }
    fn run(&mut self, doc: &mut ImageDocument) -> Result<(), String> {
        let id = self.id.or(doc.active).ok_or("Pick a layer first.")?;
        let layer = doc.layer(id).ok_or("That layer is gone.")?;
        if layer.locks.pixels_locked() {
            return Err(format!("{} has its pixels locked.", layer.name));
        }
        let raster = layer.paintable()?.clone();
        if self.mesh.points.len() != self.mesh.cols * self.mesh.rows || self.mesh.cols < 2 {
            return Err("The warp's mesh is damaged.".into());
        }
        let mut g = Graph::new();
        let src = g.add(
            Node::of(vellum_gpu::nodes::ImageNode)
                .source(std::sync::Arc::new(RasterSource::new(raster.clone()))),
        );
        let top = g.add(self.mesh.node().input(0, src));
        let reach = self.mesh.points.iter().fold(IRect::EMPTY, |acc, p| {
            acc.union(&IRect::new(
                p[0].floor() as i32 - 1,
                p[1].floor() as i32 - 1,
                3,
                3,
            ))
        });
        let rect = reach.union(&raster.content_bounds());
        let out = crate::flatten::node_to_raster(&g, top, rect, raster.depth(), doc.working())?;
        if let Some(l) = doc.layer_mut(id)
            && let Content::Pixels(r) = &mut l.content
        {
            *r = out;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Layer;
    use crate::raster::Depth;

    #[test]
    fn identity_meshes_change_nothing() {
        let m = Mesh::identity(IRect::new(10, 20, 100, 50), 16.0);
        assert!(m.is_identity());
        assert!(m.cols >= 8 && m.rows >= 5);
    }

    #[test]
    fn brushes_move_points_the_right_way() {
        let area = IRect::of_size(200, 200);
        let at = [100.0, 100.0];
        let mut m = Mesh::identity(area, 10.0);
        m.liquify(&LiquifyBrush::default(), at, [10.0, 0.0]);
        let i = 10 * m.cols + 10;
        assert!(m.points[i][0] > 100.0);
        let mut p = Mesh::identity(area, 10.0);
        let j = 10 * p.cols + 12;
        p.liquify(
            &LiquifyBrush {
                tool: LiquifyTool::Pucker,
                ..Default::default()
            },
            at,
            [0.0; 2],
        );
        assert!(p.points[j][0] < 120.0);
        let mut b = Mesh::identity(area, 10.0);
        b.liquify(
            &LiquifyBrush {
                tool: LiquifyTool::Bloat,
                ..Default::default()
            },
            at,
            [0.0; 2],
        );
        assert!(b.points[j][0] > 120.0);
        for _ in 0..40 {
            b.liquify(
                &LiquifyBrush {
                    tool: LiquifyTool::Reconstruct,
                    pressure: 1.0,
                    ..Default::default()
                },
                at,
                [0.0; 2],
            );
        }
        assert!((b.points[j][0] - 120.0).abs() < 0.5);
    }

    #[test]
    fn puppet_pins_bend_around_and_hold() {
        let still = Pin {
            from: [0.0, 0.0],
            to: [0.0, 0.0],
        };
        let moved = Pin {
            from: [100.0, 0.0],
            to: [100.0, 40.0],
        };
        let pins = [still, moved];
        assert_eq!(puppet_point(&pins, [0.0, 0.0]), [0.0, 0.0]);
        assert_eq!(puppet_point(&pins, [100.0, 0.0]), [100.0, 40.0]);
        let mid = puppet_point(&pins, [50.0, 0.0]);
        assert!(mid[1] > 5.0 && mid[1] < 35.0, "{mid:?}");
        // the same move for every pin is a plain shift
        let both = [
            Pin {
                from: [0.0, 0.0],
                to: [5.0, 5.0],
            },
            Pin {
                from: [10.0, 0.0],
                to: [15.0, 5.0],
            },
        ];
        let p = puppet_point(&both, [3.0, 7.0]);
        assert!((p[0] - 8.0).abs() < 1e-3 && (p[1] - 12.0).abs() < 1e-3);
    }

    #[test]
    fn a_mesh_warp_moves_pixels() {
        let mut doc = ImageDocument::new("t", 64, 64, Depth::U8, None);
        let id = doc.layers[0].id;
        doc.layer_mut(id)
            .and_then(Layer::raster_mut)
            .expect("raster")
            .update(IRect::new(20, 20, 8, 8), |_, _, p| {
                p.copy_from_slice(&[1.0, 0.0, 0.0, 1.0])
            });
        let mut mesh = Mesh::identity(IRect::of_size(64, 64), 8.0);
        for p in &mut mesh.points {
            p[0] += 10.0;
        }
        let mut cmd = MeshWarp {
            id: Some(id),
            title: "Liquify".into(),
            mesh,
        };
        cmd.run(&mut doc).expect("warp");
        let r = doc.layers[0].raster().expect("raster");
        assert!(r.get(33, 24, 3) > 0.9, "moved right");
        assert!(r.get(21, 24, 3) < 0.1, "gone from where it was");
    }
}
