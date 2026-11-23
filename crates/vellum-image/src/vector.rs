//! vector paths, shape layers and vector masks.
//!
//! a path is a list of subpaths of cubic bezier nodes (photoshop's anchor points with
//! their two handles). each subpath has an operation that says how it meets the ones
//! before it: combine, subtract, intersect or exclude, like photoshop's path operations.
//! the operations stay live: they are applied when the path is drawn, so a shape can be
//! changed at any time.
//!
//! paths are drawn on the processor into a grey coverage raster at document resolution,
//! filled with the non zero rule from four sub rows per pixel with exact horizontal
//! coverage (the same approach as the lasso). text uses the same filler for its glyphs.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::IRect;

use crate::raster::{Depth, Raster};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PathNode {
    pub at: [f32; 2],
    /// the handle the curve arrives along, absolute
    pub handle_in: [f32; 2],
    /// the handle the curve leaves along, absolute
    pub handle_out: [f32; 2],
    /// the two handles stay in line when one is dragged
    #[serde(default)]
    pub smooth: bool,
}

impl PathNode {
    /// a sharp corner with no handles
    pub fn corner(x: f32, y: f32) -> Self {
        Self {
            at: [x, y],
            handle_in: [x, y],
            handle_out: [x, y],
            smooth: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PathOp {
    #[default]
    Combine,
    Subtract,
    Intersect,
    Exclude,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubPath {
    pub nodes: Vec<PathNode>,
    pub closed: bool,
    #[serde(default)]
    pub op: PathOp,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VectorPath {
    pub subpaths: Vec<SubPath>,
}

/// a 2x3 matrix, rows first: x' = a x + c y + e, y' = b x + d y + f
pub type Affine = [f32; 6];

pub fn apply(m: &Affine, p: [f32; 2]) -> [f32; 2] {
    [
        m[0] * p[0] + m[2] * p[1] + m[4],
        m[1] * p[0] + m[3] * p[1] + m[5],
    ]
}

fn cubic(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], out: &mut Vec<[f64; 2]>) {
    let len = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let rough = len(p0, p1) + len(p1, p2) + len(p2, p3);
    if rough < 1e-3 || (p0 == p1 && p2 == p3) {
        out.push([f64::from(p3[0]), f64::from(p3[1])]);
        return;
    }
    let n = (rough / 1.5).ceil().clamp(1.0, 256.0) as usize;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push([
            f64::from(a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0]),
            f64::from(a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]),
        ]);
    }
}

impl SubPath {
    /// the subpath as points, curves cut into short straight pieces
    pub fn flatten(&self) -> Vec<[f64; 2]> {
        let n = self.nodes.len();
        let mut out = Vec::new();
        let Some(first) = self.nodes.first() else {
            return out;
        };
        out.push([f64::from(first.at[0]), f64::from(first.at[1])]);
        let segments = if self.closed { n } else { n.saturating_sub(1) };
        for i in 0..segments {
            let a = self.nodes[i];
            let b = self.nodes[(i + 1) % n];
            cubic(a.at, a.handle_out, b.handle_in, b.at, &mut out);
        }
        if self.closed && out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        out
    }
}

impl VectorPath {
    pub fn single(sub: SubPath) -> Self {
        Self {
            subpaths: vec![sub],
        }
    }

    pub fn transform(&mut self, m: &Affine) {
        for s in &mut self.subpaths {
            for n in &mut s.nodes {
                n.at = apply(m, n.at);
                n.handle_in = apply(m, n.handle_in);
                n.handle_out = apply(m, n.handle_out);
            }
        }
    }

    pub fn bounds(&self) -> IRect {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for s in &self.subpaths {
            for n in &s.nodes {
                for p in [n.at, n.handle_in, n.handle_out] {
                    x0 = x0.min(p[0]);
                    y0 = y0.min(p[1]);
                    x1 = x1.max(p[0]);
                    y1 = y1.max(p[1]);
                }
            }
        }
        if x0 > x1 {
            return IRect::EMPTY;
        }
        IRect::from_corners(
            x0.floor() as i32,
            y0.floor() as i32,
            x1.ceil() as i32,
            y1.ceil() as i32,
        )
    }

    /// how much of each pixel the path covers, its operations applied in order
    pub fn coverage(&self, clip: IRect) -> Raster {
        let mut acc: Option<Raster> = None;
        for s in &self.subpaths {
            let contour = s.flatten();
            let c = fill(&[contour], FillRule::NonZero, clip);
            acc = Some(match (acc, s.op) {
                (None, _) => c,
                (Some(a), op) => combine(&a, &c, op, clip),
            });
        }
        acc.unwrap_or_else(|| Raster::gray(Depth::U8, 0.0))
    }
}

/// meets two coverages with a path operation
pub fn combine(a: &Raster, b: &Raster, op: PathOp, clip: IRect) -> Raster {
    let area = match op {
        PathOp::Intersect => a.content_bounds().intersect(&b.content_bounds()),
        PathOp::Subtract => a.content_bounds(),
        _ => a.content_bounds().union(&b.content_bounds()),
    }
    .intersect(&clip);
    let mut out = Raster::gray(Depth::U8, 0.0);
    out.update(area, |x, y, v| {
        let (p, q) = (a.get(x, y, 0), b.get(x, y, 0));
        v[0] = match op {
            PathOp::Combine => p + q - p * q,
            PathOp::Subtract => p * (1.0 - q),
            PathOp::Intersect => p * q,
            PathOp::Exclude => p + q - 2.0 * p * q,
        };
    });
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// fills closed contours into a grey coverage raster, clipped to `clip`
pub fn fill(contours: &[Vec<[f64; 2]>], rule: FillRule, clip: IRect) -> Raster {
    let mut out = Raster::gray(Depth::U8, 0.0);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in contours {
        for p in c {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
    }
    if x0 > x1 {
        return out;
    }
    let bounds = IRect::from_corners(
        x0.floor() as i32,
        y0.floor() as i32,
        x1.ceil() as i32,
        y1.ceil() as i32,
    )
    .intersect(&clip);
    if bounds.is_empty() {
        return out;
    }
    let edges: Vec<([f64; 2], [f64; 2])> = contours
        .iter()
        .filter(|c| c.len() >= 3)
        .flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()])))
        .collect();
    const SUBS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let rows: Vec<Vec<(f64, f64)>> = (bounds.y..bounds.bottom())
        .into_par_iter()
        .map(|row| {
            let mut spans = Vec::new();
            let mut crossings: Vec<(f64, i32)> = Vec::new();
            for sub in SUBS {
                let sy = f64::from(row) + sub;
                crossings.clear();
                for (a, b) in &edges {
                    if (a[1] <= sy && b[1] > sy) || (b[1] <= sy && a[1] > sy) {
                        let t = (sy - a[1]) / (b[1] - a[1]);
                        let dir = if b[1] > a[1] { 1 } else { -1 };
                        crossings.push((a[0] + t * (b[0] - a[0]), dir));
                    }
                }
                crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
                let mut winding = 0;
                for pair in crossings.windows(2) {
                    winding += pair[0].1;
                    let inside = match rule {
                        FillRule::NonZero => winding != 0,
                        FillRule::EvenOdd => winding % 2 != 0,
                    };
                    if inside && pair[1].0 > pair[0].0 {
                        spans.push((pair[0].0, pair[1].0));
                    }
                }
            }
            spans
        })
        .collect();
    out.update(bounds, |x, y, v| {
        let row = &rows[(y - bounds.y) as usize];
        let (a0, a1) = (f64::from(x), f64::from(x) + 1.0);
        let mut c = 0.0;
        for &(s, e) in row {
            c += (a1.min(e) - a0.max(s)).max(0.0);
        }
        v[0] = (c as f32 / SUBS.len() as f32).min(1.0);
    });
    out
}

/// how far a point is from a line segment
fn segment_distance(p: [f32; 2], a: [f64; 2], b: [f64; 2]) -> f32 {
    let (ax, ay, bx, by) = (a[0] as f32, a[1] as f32, b[0] as f32, b[1] as f32);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - ax) * dx + (p[1] - ay) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - ax - t * dx).hypot(p[1] - ay - t * dy)
}

/// how much of each pixel a line of `width` along the polylines covers, from the distance
/// of the pixel's centre to the nearest segment, with a pixel of anti aliasing
pub fn stroke_lines(lines: &[(Vec<[f64; 2]>, bool)], width: f32, clip: IRect) -> Raster {
    let half = (width / 2.0).max(0.25);
    let mut out = Raster::gray(Depth::U8, 0.0);
    for (pts, closed) in lines {
        let n = pts.len();
        if n == 0 {
            continue;
        }
        let segs = if *closed { n } else { n - 1 };
        for i in 0..segs.max(1) {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            let r = IRect::from_corners(
                (a[0].min(b[0]) as f32 - half - 1.0).floor() as i32,
                (a[1].min(b[1]) as f32 - half - 1.0).floor() as i32,
                (a[0].max(b[0]) as f32 + half + 1.0).ceil() as i32,
                (a[1].max(b[1]) as f32 + half + 1.0).ceil() as i32,
            )
            .intersect(&clip);
            out.update(r, |x, y, v| {
                let d = segment_distance([x as f32 + 0.5, y as f32 + 0.5], a, b);
                let c = (half - d + 0.5).clamp(0.0, 1.0);
                v[0] = v[0].max(c);
            });
        }
    }
    out
}

// ---- shapes ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "type")]
#[ts(export)]
pub enum Geometry {
    /// a rectangle with live corner radii, top left, top right, bottom right, bottom left
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
    },
    Ellipse {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// a regular polygon, or a star when `star` is under 1 (the inner radius's share)
    Polygon {
        cx: f32,
        cy: f32,
        radius: f32,
        sides: u32,
        star: f32,
        /// degrees
        rotation: f32,
    },
    /// a straight line drawn as a filled bar, with optional arrowheads
    Line {
        from: [f32; 2],
        to: [f32; 2],
        weight: f32,
        arrow_start: bool,
        arrow_end: bool,
    },
    /// one of the built in custom shapes, fitted to a box
    Custom {
        name: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Path {
        path: VectorPath,
    },
}

/// a cubic handle length that makes a quarter circle
const KAPPA: f32 = 0.552_284_8;

fn closed(nodes: Vec<PathNode>) -> SubPath {
    SubPath {
        nodes,
        closed: true,
        op: PathOp::Combine,
    }
}

fn polygon(points: &[[f32; 2]]) -> SubPath {
    closed(
        points
            .iter()
            .map(|p| PathNode::corner(p[0], p[1]))
            .collect(),
    )
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> SubPath {
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    let limit = w.min(h) / 2.0;
    let r = radii.map(|v| v.clamp(0.0, limit));
    if r.iter().all(|v| *v <= 0.0) {
        return polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]]);
    }
    let k = KAPPA;
    let mut nodes = Vec::new();
    // each corner adds the point where its curve starts and where it ends, clockwise
    let corners = [
        ([x, y], [1.0, 0.0], [0.0, 1.0], r[0]),
        ([x + w, y], [0.0, 1.0], [-1.0, 0.0], r[1]),
        ([x + w, y + h], [-1.0, 0.0], [0.0, -1.0], r[2]),
        ([x, y + h], [0.0, -1.0], [1.0, 0.0], r[3]),
    ];
    for (c, out_dir, in_dir, rr) in corners {
        if rr <= 0.0 {
            nodes.push(PathNode::corner(c[0], c[1]));
            continue;
        }
        // the curve runs from the point on the incoming edge to the one on the outgoing
        let start = [c[0] + in_dir[0] * rr, c[1] + in_dir[1] * rr];
        let end = [c[0] + out_dir[0] * rr, c[1] + out_dir[1] * rr];
        nodes.push(PathNode {
            at: start,
            handle_in: start,
            handle_out: [start[0] - in_dir[0] * rr * k, start[1] - in_dir[1] * rr * k],
            smooth: false,
        });
        nodes.push(PathNode {
            at: end,
            handle_in: [end[0] - out_dir[0] * rr * k, end[1] - out_dir[1] * rr * k],
            handle_out: end,
            smooth: false,
        });
    }
    closed(nodes)
}

fn ellipse(x: f32, y: f32, w: f32, h: f32) -> SubPath {
    let (rx, ry) = (w.abs() / 2.0, h.abs() / 2.0);
    let (cx, cy) = (x.min(x + w) + rx, y.min(y + h) + ry);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    closed(vec![
        PathNode {
            at: [cx, cy - ry],
            handle_in: [cx - kx, cy - ry],
            handle_out: [cx + kx, cy - ry],
            smooth: true,
        },
        PathNode {
            at: [cx + rx, cy],
            handle_in: [cx + rx, cy - ky],
            handle_out: [cx + rx, cy + ky],
            smooth: true,
        },
        PathNode {
            at: [cx, cy + ry],
            handle_in: [cx + kx, cy + ry],
            handle_out: [cx - kx, cy + ry],
            smooth: true,
        },
        PathNode {
            at: [cx - rx, cy],
            handle_in: [cx - rx, cy + ky],
            handle_out: [cx - rx, cy - ky],
            smooth: true,
        },
    ])
}

/// the built in custom shapes, drawn in a 0 to 1 box
pub const CUSTOM_SHAPES: &[&str] = &[
    "Heart",
    "Arrow",
    "Speech bubble",
    "Check mark",
    "Lightning",
    "Cross",
];

fn custom(name: &str, x: f32, y: f32, w: f32, h: f32) -> VectorPath {
    let map = |p: [f32; 2]| [x + p[0] * w, y + p[1] * h];
    let poly = |pts: &[[f32; 2]]| polygon(&pts.iter().map(|p| map(*p)).collect::<Vec<_>>());
    match name {
        "Heart" => {
            let node = |at: [f32; 2], i: [f32; 2], o: [f32; 2]| PathNode {
                at: map(at),
                handle_in: map(i),
                handle_out: map(o),
                smooth: false,
            };
            VectorPath::single(closed(vec![
                node([0.5, 0.25], [0.35, 0.0], [0.65, 0.0]),
                node([1.0, 0.3], [1.0, 0.0], [1.0, 0.6]),
                node([0.5, 1.0], [0.75, 0.8], [0.25, 0.8]),
                node([0.0, 0.3], [0.0, 0.6], [0.0, 0.0]),
            ]))
        }
        "Arrow" => VectorPath::single(poly(&[
            [0.0, 0.35],
            [0.6, 0.35],
            [0.6, 0.1],
            [1.0, 0.5],
            [0.6, 0.9],
            [0.6, 0.65],
            [0.0, 0.65],
        ])),
        "Speech bubble" => {
            let mut body = rounded_rect(x, y, w, h * 0.75, [w.min(h) * 0.15; 4]);
            body.op = PathOp::Combine;
            let mut tail = poly(&[[0.2, 0.7], [0.45, 0.7], [0.15, 1.0]]);
            tail.op = PathOp::Combine;
            VectorPath {
                subpaths: vec![body, tail],
            }
        }
        "Check mark" => VectorPath::single(poly(&[
            [0.0, 0.55],
            [0.15, 0.4],
            [0.38, 0.62],
            [0.85, 0.1],
            [1.0, 0.25],
            [0.38, 0.9],
        ])),
        "Lightning" => VectorPath::single(poly(&[
            [0.55, 0.0],
            [0.15, 0.55],
            [0.45, 0.55],
            [0.35, 1.0],
            [0.85, 0.4],
            [0.55, 0.4],
            [0.7, 0.0],
        ])),
        _ => VectorPath::single(poly(&[
            [0.35, 0.0],
            [0.65, 0.0],
            [0.65, 0.35],
            [1.0, 0.35],
            [1.0, 0.65],
            [0.65, 0.65],
            [0.65, 1.0],
            [0.35, 1.0],
            [0.35, 0.65],
            [0.0, 0.65],
            [0.0, 0.35],
            [0.35, 0.35],
        ])),
    }
}

impl Geometry {
    pub fn to_path(&self) -> VectorPath {
        match self {
            Geometry::Rect { x, y, w, h, radii } => {
                VectorPath::single(rounded_rect(*x, *y, *w, *h, *radii))
            }
            Geometry::Ellipse { x, y, w, h } => VectorPath::single(ellipse(*x, *y, *w, *h)),
            Geometry::Polygon {
                cx,
                cy,
                radius,
                sides,
                star,
                rotation,
            } => {
                let n = (*sides).clamp(3, 100) as usize;
                let star = star.clamp(0.01, 1.0);
                let count = if star < 1.0 { n * 2 } else { n };
                let pts: Vec<[f32; 2]> = (0..count)
                    .map(|i| {
                        let a = (rotation - 90.0).to_radians()
                            + std::f32::consts::TAU * i as f32 / count as f32;
                        let r = if star < 1.0 && i % 2 == 1 {
                            radius * star
                        } else {
                            *radius
                        };
                        [cx + a.cos() * r, cy + a.sin() * r]
                    })
                    .collect();
                VectorPath::single(polygon(&pts))
            }
            Geometry::Line {
                from,
                to,
                weight,
                arrow_start,
                arrow_end,
            } => {
                let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                let len = dx.hypot(dy).max(1e-3);
                let (ux, uy) = (dx / len, dy / len);
                let (nx, ny) = (-uy * weight / 2.0, ux * weight / 2.0);
                let mut path = VectorPath::single(polygon(&[
                    [from[0] + nx, from[1] + ny],
                    [to[0] + nx, to[1] + ny],
                    [to[0] - nx, to[1] - ny],
                    [from[0] - nx, from[1] - ny],
                ]));
                let head = |tip: [f32; 2], dir: f32| {
                    let size = weight.max(1.0) * 4.0;
                    let back = [tip[0] - ux * size * dir, tip[1] - uy * size * dir];
                    polygon(&[
                        tip,
                        [back[0] + uy * size * 0.5, back[1] - ux * size * 0.5],
                        [back[0] - uy * size * 0.5, back[1] + ux * size * 0.5],
                    ])
                };
                if *arrow_end {
                    path.subpaths.push(head(*to, 1.0));
                }
                if *arrow_start {
                    path.subpaths.push(head(*from, -1.0));
                }
                path
            }
            Geometry::Custom { name, x, y, w, h } => custom(name, *x, *y, *w, *h),
            Geometry::Path { path } => path.clone(),
        }
    }

    pub fn transform(&mut self, m: &Affine) {
        // anything but a plain move or scale turns into a path so it keeps its shape
        let simple = m[1] == 0.0 && m[2] == 0.0 && m[0] > 0.0 && m[3] > 0.0;
        match self {
            Geometry::Rect { x, y, w, h, radii } if simple => {
                [*x, *y] = apply(m, [*x, *y]);
                *w *= m[0];
                *h *= m[3];
                for r in radii.iter_mut() {
                    *r *= m[0].min(m[3]);
                }
            }
            Geometry::Ellipse { x, y, w, h } | Geometry::Custom { x, y, w, h, .. } if simple => {
                [*x, *y] = apply(m, [*x, *y]);
                *w *= m[0];
                *h *= m[3];
            }
            Geometry::Line {
                from, to, weight, ..
            } if simple => {
                *from = apply(m, *from);
                *to = apply(m, *to);
                *weight *= m[0].min(m[3]);
            }
            Geometry::Path { path } => path.transform(m),
            _ => {
                let mut path = self.to_path();
                path.transform(m);
                *self = Geometry::Path { path };
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StrokeAlign {
    Inside,
    #[default]
    Center,
    Outside,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShapeStroke {
    /// straight srgb
    pub color: [f32; 4],
    pub width: f32,
    #[serde(default)]
    pub align: StrokeAlign,
}

/// one part of a shape layer and how it meets the parts before it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShapeItem {
    pub geometry: Geometry,
    #[serde(default)]
    pub op: PathOp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShapeLayer {
    pub items: Vec<ShapeItem>,
    /// straight srgb fill, none for no fill
    pub fill: Option<[f32; 4]>,
    #[serde(default)]
    pub stroke: Option<ShapeStroke>,
}

impl ShapeLayer {
    pub fn path(&self) -> VectorPath {
        let mut out = VectorPath::default();
        for item in &self.items {
            let mut p = item.geometry.to_path();
            if let Some(first) = p.subpaths.first_mut() {
                first.op = item.op;
            }
            out.subpaths.extend(p.subpaths);
        }
        out
    }

    pub fn transform(&mut self, m: &Affine) {
        for item in &mut self.items {
            item.geometry.transform(m);
        }
        if let Some(s) = &mut self.stroke {
            s.width *= (m[0] * m[3] - m[1] * m[2]).abs().sqrt();
        }
    }

    /// the shape drawn into straight colour at `depth`. colours are document values.
    pub fn render(&self, depth: Depth, to_doc: &dyn Fn([f32; 4]) -> [f32; 4]) -> Raster {
        let path = self.path();
        let reach = self.stroke.as_ref().map_or(0.0, |s| s.width) as i32 + 2;
        let clip = path.bounds().expand(reach);
        let cover = path.coverage(clip);
        let stroke = self.stroke.as_ref().map(|s| {
            let lines: Vec<(Vec<[f64; 2]>, bool)> = path
                .subpaths
                .iter()
                .map(|sp| (sp.flatten(), sp.closed))
                .collect();
            let width = match s.align {
                StrokeAlign::Center => s.width,
                _ => s.width * 2.0,
            };
            let line = stroke_lines(&lines, width, clip);
            let mut out = Raster::gray(Depth::U8, 0.0);
            out.update(clip, |x, y, v| {
                let l = line.get(x, y, 0);
                let f = cover.get(x, y, 0);
                v[0] = match s.align {
                    StrokeAlign::Center => l,
                    StrokeAlign::Inside => l * f,
                    StrokeAlign::Outside => l * (1.0 - f),
                };
            });
            (out, to_doc(s.color))
        });
        let fill = self.fill.map(to_doc);
        let mut out = Raster::rgba(depth);
        out.update(clip, |x, y, px| {
            let mut c = [0.0f32; 4];
            if let Some(f) = fill {
                let a = cover.get(x, y, 0) * f[3];
                c = [f[0], f[1], f[2], a];
            }
            if let Some((line, col)) = &stroke {
                let a = line.get(x, y, 0) * col[3];
                if a > 0.0 {
                    let out_a = a + c[3] * (1.0 - a);
                    for k in 0..3 {
                        c[k] = (col[k] * a + c[k] * c[3] * (1.0 - a)) / out_a.max(1e-6);
                    }
                    c[3] = out_a;
                }
            }
            px.copy_from_slice(&c);
        });
        out
    }
}

/// a vector mask: the layer shows inside the path
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VectorMask {
    pub path: VectorPath,
    pub enabled: bool,
    /// show outside the path instead
    #[serde(default)]
    pub invert: bool,
}

impl VectorMask {
    /// the mask as grey coverage, 1 shows the layer. `canvas` bounds an inverted mask.
    pub fn raster(&self, canvas: IRect) -> Raster {
        let clip = self.path.bounds().union(&canvas);
        let c = self.path.coverage(clip);
        if !self.invert {
            return c;
        }
        let mut out = Raster::gray(Depth::U8, 1.0);
        out.update(c.content_bounds(), |x, y, v| v[0] = 1.0 - c.get(x, y, 0));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(r: &Raster) -> f32 {
        let b = r.content_bounds();
        let mut s = 0.0;
        for y in b.y..b.bottom() {
            for x in b.x..b.right() {
                s += r.get(x, y, 0);
            }
        }
        s
    }

    #[test]
    fn shapes_cover_the_right_area() {
        let big = IRect::new(-10, -10, 400, 400);
        let rect = Geometry::Rect {
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 50.0,
            radii: [0.0; 4],
        };
        assert!((area(&rect.to_path().coverage(big)) - 5000.0).abs() < 1.0);
        let round = Geometry::Rect {
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 50.0,
            radii: [10.0; 4],
        };
        let expect = 5000.0 - 4.0 * (100.0 - std::f32::consts::PI * 25.0);
        assert!((area(&round.to_path().coverage(big)) - expect).abs() < 8.0);
        let circle = Geometry::Ellipse {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let a = area(&circle.to_path().coverage(big));
        assert!((a - std::f32::consts::PI * 2500.0).abs() / a < 0.01, "{a}");
    }

    #[test]
    fn path_operations_combine_like_sets() {
        let big = IRect::new(0, 0, 200, 200);
        let sq = |x: f32, op| ShapeItem {
            geometry: Geometry::Rect {
                x,
                y: 0.0,
                w: 100.0,
                h: 100.0,
                radii: [0.0; 4],
            },
            op,
        };
        let layer = |op| ShapeLayer {
            items: vec![sq(0.0, PathOp::Combine), sq(50.0, op)],
            fill: Some([1.0; 4]),
            stroke: None,
        };
        let a = |op| area(&layer(op).path().coverage(big));
        assert!((a(PathOp::Combine) - 15000.0).abs() < 2.0);
        assert!((a(PathOp::Subtract) - 5000.0).abs() < 2.0);
        assert!((a(PathOp::Intersect) - 5000.0).abs() < 2.0);
        assert!((a(PathOp::Exclude) - 10000.0).abs() < 2.0);
    }

    #[test]
    fn strokes_and_stars_draw() {
        let star = Geometry::Polygon {
            cx: 50.0,
            cy: 50.0,
            radius: 40.0,
            sides: 5,
            star: 0.5,
            rotation: 0.0,
        };
        let layer = ShapeLayer {
            items: vec![ShapeItem {
                geometry: star,
                op: PathOp::Combine,
            }],
            fill: None,
            stroke: Some(ShapeStroke {
                color: [1.0, 0.0, 0.0, 1.0],
                width: 4.0,
                align: StrokeAlign::Center,
            }),
        };
        let r = layer.render(Depth::U8, &|c| c);
        assert!(r.get(50, 50, 3) < 0.01, "hollow");
        assert!(r.get(50, 10, 3) > 0.9, "the top point is stroked");
        let line = Geometry::Line {
            from: [0.0, 10.0],
            to: [100.0, 10.0],
            weight: 4.0,
            arrow_start: false,
            arrow_end: true,
        };
        assert!(line.to_path().subpaths.len() == 2);
    }

    #[test]
    fn transforms_keep_simple_shapes_live() {
        let mut g = Geometry::Rect {
            x: 10.0,
            y: 10.0,
            w: 10.0,
            h: 10.0,
            radii: [2.0; 4],
        };
        g.transform(&[2.0, 0.0, 0.0, 2.0, 5.0, 0.0]);
        assert!(matches!(g, Geometry::Rect { x, w, .. } if x == 25.0 && w == 20.0));
        g.transform(&[0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
        assert!(matches!(g, Geometry::Path { .. }));
        for name in CUSTOM_SHAPES {
            let c = Geometry::Custom {
                name: (*name).into(),
                x: 0.0,
                y: 0.0,
                w: 50.0,
                h: 50.0,
            };
            assert!(
                area(&c.to_path().coverage(IRect::of_size(60, 60))) > 200.0,
                "{name}"
            );
        }
    }

    #[test]
    fn inverted_vector_masks_show_outside() {
        let m = VectorMask {
            path: Geometry::Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                radii: [0.0; 4],
            }
            .to_path(),
            enabled: true,
            invert: true,
        };
        let r = m.raster(IRect::of_size(20, 20));
        assert!(r.get(5, 5, 0) < 0.01 && r.get(15, 15, 0) > 0.99);
    }
}
