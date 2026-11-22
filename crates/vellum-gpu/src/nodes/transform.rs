//! moves, scales, rotates, skews and distorts an image with a 3x3 matrix, which covers
//! perspective too. sampling is nearest, bilinear or bicubic (catmull-rom), and shrinking
//! takes several samples per pixel so fine detail does not shimmer.
//!
//! with a [`NodeData`] attached (see [`mesh_grid`]) the node warps along a mesh instead of
//! the matrix: the data is a grid over the output that says where each grid point reads
//! from, and pixels between grid points interpolate it. that keeps a warp as cheap per
//! pixel as a matrix on both backends.

use std::hash::Hasher;

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{
    NodeCategory, NodeCx, NodeData, NodeInfo, NodeKind, ONE_INPUT, Out, input_bounds,
};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};
use crate::rect::{Bounds, IRect};

pub struct TransformNode;

pub const INTERPOLATION: &[ChoiceOption] = &[
    ChoiceOption {
        id: "nearest",
        label: "Nearest neighbour",
    },
    ChoiceOption {
        id: "bilinear",
        label: "Bilinear",
    },
    ChoiceOption {
        id: "bicubic",
        label: "Bicubic",
    },
];

static INFO: NodeInfo = NodeInfo {
    id: "transform",
    title: "Transform",
    category: NodeCategory::Transform,
    description: "Moves, scales, rotates, skews or distorts an image.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::matrix("matrix", "Matrix"),
        ParamSpec::choice("interpolation", "Interpolation", INTERPOLATION, "bicubic"),
    ],
};

/// the largest number of samples per axis taken when shrinking
const MAX_TAPS: i32 = 8;

const KERNEL: Kernel = Kernel {
    name: "transform",
    body: r#"
fn map_point(c: vec2<f32>) -> vec2<f32> {
    let r0 = param(0u).xyz;
    let r1 = param(1u).xyz;
    let r2 = param(2u).xyz;
    let h = vec3<f32>(c, 1.0);
    let w = dot(r2, h);
    if (abs(w) < 1e-9) { return vec2<f32>(-1e9); }
    return vec2<f32>(dot(r0, h), dot(r1, h)) / w;
}

fn cubic_w(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    );
}

fn sample_at(q: vec2<f32>) -> vec4<f32> {
    let mode = u32(param(3u).x);
    if (mode == 0u) {
        return load0(vec2<i32>(floor(q)));
    }
    let s = q - 0.5;
    let i = vec2<i32>(floor(s));
    let f = s - floor(s);
    if (mode == 1u) {
        let a = mix(load0(i), load0(i + vec2<i32>(1, 0)), f.x);
        let b = mix(load0(i + vec2<i32>(0, 1)), load0(i + vec2<i32>(1, 1)), f.x);
        return mix(a, b, f.y);
    }
    let wx = cubic_w(f.x);
    let wy = cubic_w(f.y);
    var acc = vec4<f32>(0.0);
    for (var y = 0; y < 4; y = y + 1) {
        var row = vec4<f32>(0.0);
        for (var x = 0; x < 4; x = x + 1) {
            row = row + load0(i + vec2<i32>(x - 1, y - 1)) * wx[x];
        }
        acc = acc + row * wy[y];
    }
    // catmull-rom overshoots; keep colour within alpha so premultiplied values stay valid
    let a = clamp(acc.a, 0.0, 1.0e9);
    return vec4<f32>(clamp(acc.rgb, vec3<f32>(0.0), vec3<f32>(max(a, 0.0) * 65504.0)), a);
}

fn mesh_vertex(i: i32, j: i32, cols: i32) -> vec2<f32> {
    let k = 9 + 2 * (j * cols + i);
    return vec2<f32>(data[k], data[k + 1]);
}

// where a document point reads from, interpolated from the grid in the data buffer
fn mesh_src(d: vec2<f32>) -> vec2<f32> {
    let step = data[2];
    let cols = i32(data[3]);
    let rows = i32(data[4]);
    let gx = clamp((d.x - data[0]) / step, 0.0, f32(cols - 1));
    let gy = clamp((d.y - data[1]) / step, 0.0, f32(rows - 1));
    let ix = min(i32(floor(gx)), cols - 2);
    let iy = min(i32(floor(gy)), rows - 2);
    let fx = gx - f32(ix);
    let fy = gy - f32(iy);
    let a = mix(mesh_vertex(ix, iy, cols), mesh_vertex(ix + 1, iy, cols), fx);
    let b = mix(mesh_vertex(ix, iy + 1, cols), mesh_vertex(ix + 1, iy + 1, cols), fx);
    return mix(a, b, fy);
}

fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = vec2<f32>(p) + 0.5;
    var q: vec2<f32>;
    var dx: vec2<f32>;
    var dy: vec2<f32>;
    if (param(3u).y > 0.5) {
        let s = param(3u).z;
        let d = c * s;
        let m = mesh_src(d);
        if (m.x < data[5] - 1.0 || m.y < data[6] - 1.0 || m.x > data[5] + data[7] + 1.0 || m.y > data[6] + data[8] + 1.0) {
            return vec4<f32>(0.0);
        }
        q = m / s;
        dx = mesh_src(d + vec2<f32>(s, 0.0)) / s - q;
        dy = mesh_src(d + vec2<f32>(0.0, s)) / s - q;
    } else {
        q = map_point(c);
        dx = map_point(c + vec2<f32>(1.0, 0.0)) - q;
        dy = map_point(c + vec2<f32>(0.0, 1.0)) - q;
    }
    var nx = 1;
    var ny = 1;
    if (u32(param(3u).x) != 0u) {
        nx = clamp(i32(ceil(length(dx))), 1, 8);
        ny = clamp(i32(ceil(length(dy))), 1, 8);
    }
    var acc = vec4<f32>(0.0);
    for (var j = 0; j < ny; j = j + 1) {
        for (var i = 0; i < nx; i = i + 1) {
            let u = (f32(i) + 0.5) / f32(nx) - 0.5;
            let v = (f32(j) + 0.5) / f32(ny) - 0.5;
            acc = acc + sample_at(q + dx * u + dy * v);
        }
    }
    return acc / f32(nx * ny);
}
"#,
};

type M3 = [[f64; 3]; 3];

fn mat(m: [f32; 9]) -> M3 {
    [
        [f64::from(m[0]), f64::from(m[1]), f64::from(m[2])],
        [f64::from(m[3]), f64::from(m[4]), f64::from(m[5])],
        [f64::from(m[6]), f64::from(m[7]), f64::from(m[8])],
    ]
}

fn invert(m: &M3) -> Option<M3> {
    let c = crate::color::Mat3(*m);
    if c.determinant().abs() < 1e-12 {
        return None;
    }
    Some(c.inverse().0)
}

/// the matrix at a mip level: level pixels in, level pixels out
fn at_level(m: &M3, level: u32) -> M3 {
    let s = 1.0 / (1u64 << level.min(30)) as f64;
    let mut r = *m;
    r[0][2] *= s;
    r[1][2] *= s;
    r[2][0] /= s;
    r[2][1] /= s;
    r
}

fn apply(m: &M3, x: f64, y: f64) -> Option<(f64, f64)> {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    if w <= 1e-9 {
        return None;
    }
    Some((
        (m[0][0] * x + m[0][1] * y + m[0][2]) / w,
        (m[1][0] * x + m[1][1] * y + m[1][2]) / w,
    ))
}

/// the box around a rectangle mapped through `m`, or none if part of it lands behind the
/// viewer of a perspective matrix
fn map_rect(m: &M3, r: IRect) -> Option<IRect> {
    let corners = [
        (r.x, r.y),
        (r.right(), r.y),
        (r.x, r.bottom()),
        (r.right(), r.bottom()),
    ];
    let mut x0 = f64::INFINITY;
    let mut y0 = f64::INFINITY;
    let mut x1 = f64::NEG_INFINITY;
    let mut y1 = f64::NEG_INFINITY;
    for (x, y) in corners {
        let (px, py) = apply(m, f64::from(x), f64::from(y))?;
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    let lim = f64::from(i32::MAX / 4);
    // f32 matrices put a 90 degree turn's corners a hair off whole pixels; snap those
    // instead of growing the box by a pixel
    const SNAP: f64 = 1e-4;
    Some(IRect::from_corners(
        (x0 + SNAP).floor().clamp(-lim, lim) as i32,
        (y0 + SNAP).floor().clamp(-lim, lim) as i32,
        (x1 - SNAP).ceil().clamp(-lim, lim) as i32,
        (y1 - SNAP).ceil().clamp(-lim, lim) as i32,
    ))
}

/// how far past the output region a perspective that crosses the horizon may read
const PAST_HORIZON: i32 = 4096;

struct Setup<'a> {
    forward: M3,
    inverse: Option<M3>,
    mode: u32,
    mesh: Option<Mesh<'a>>,
}

fn setup<'a>(cx: &NodeCx<'a>) -> Setup<'a> {
    let m = at_level(&mat(cx.params.matrix("matrix")), cx.level);
    Setup {
        inverse: invert(&m),
        forward: m,
        mode: cx.params.choice_index("interpolation"),
        mesh: cx.data().and_then(|d| Mesh::read(&d.values)),
    }
}

/// floats before the grid: origin x and y, step, columns, rows, source box x, y, w, h
const MESH_HEADER: usize = 9;

/// the inverse grid of a warp, as [`mesh_grid`] lays it out
struct Mesh<'a> {
    values: &'a [f32],
    origin: [f32; 2],
    step: f32,
    cols: usize,
    rows: usize,
    source: [f32; 4],
}

impl<'a> Mesh<'a> {
    fn read(values: &'a [f32]) -> Option<Self> {
        let head = values.get(..MESH_HEADER)?;
        let (cols, rows) = (head[3] as usize, head[4] as usize);
        if cols < 2 || rows < 2 || head[2] <= 0.0 || values.len() < MESH_HEADER + cols * rows * 2 {
            return None;
        }
        Some(Self {
            values,
            origin: [head[0], head[1]],
            step: head[2],
            cols,
            rows,
            source: [head[5], head[6], head[7], head[8]],
        })
    }

    fn vertex(&self, i: usize, j: usize) -> [f32; 2] {
        let k = MESH_HEADER + 2 * (j * self.cols + i);
        [self.values[k], self.values[k + 1]]
    }

    /// the same f32 maths as `mesh_src` in the kernel
    fn src(&self, d: [f32; 2]) -> [f32; 2] {
        let gx = ((d[0] - self.origin[0]) / self.step).clamp(0.0, (self.cols - 1) as f32);
        let gy = ((d[1] - self.origin[1]) / self.step).clamp(0.0, (self.rows - 1) as f32);
        let ix = (gx.floor() as usize).min(self.cols - 2);
        let iy = (gy.floor() as usize).min(self.rows - 2);
        let (fx, fy) = (gx - ix as f32, gy - iy as f32);
        let mix =
            |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let a = mix(self.vertex(ix, iy), self.vertex(ix + 1, iy), fx);
        let b = mix(self.vertex(ix, iy + 1), self.vertex(ix + 1, iy + 1), fx);
        mix(a, b, fy)
    }

    fn inside_source(&self, m: [f32; 2]) -> bool {
        let [x, y, w, h] = self.source;
        m[0] >= x - 1.0 && m[1] >= y - 1.0 && m[0] <= x + w + 1.0 && m[1] <= y + h + 1.0
    }

    /// the grid's extent at a mip level
    fn bounds(&self, scale: f32) -> IRect {
        let x1 = self.origin[0] + self.step * (self.cols - 1) as f32;
        let y1 = self.origin[1] + self.step * (self.rows - 1) as f32;
        IRect::from_corners(
            (self.origin[0] / scale).floor() as i32,
            (self.origin[1] / scale).floor() as i32,
            (x1 / scale).ceil() as i32,
            (y1 / scale).ceil() as i32,
        )
    }

    fn source_at(&self, scale: f32) -> IRect {
        let [x, y, w, h] = self.source;
        IRect::from_corners(
            (x / scale).floor() as i32,
            (y / scale).floor() as i32,
            ((x + w) / scale).ceil() as i32,
            ((y + h) / scale).ceil() as i32,
        )
    }
}

/// a warp for the transform node: `points` is a `cols` by `rows` grid saying where each
/// point of an even grid over the `source` box (x, y, w, h) lands, row by row. the result
/// turns that around into a grid over the output saying where each point reads from,
/// which is what the node interpolates. cells of the mesh are split into triangles and
/// mapped exactly; grid points just outside the mesh are extrapolated so edges stay sharp.
pub fn mesh_grid(source: [f32; 4], cols: usize, rows: usize, points: &[[f32; 2]]) -> NodeData {
    let cols = cols.max(2);
    let rows = rows.max(2);
    if points.len() < cols * rows {
        return NodeData::new(Vec::new());
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &points[..cols * rows] {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return NodeData::new(Vec::new());
    }
    // at most about 600 points a side, and never finer than two pixels
    let step = ((x1 - x0).max(y1 - y0) / 600.0).max(2.0).ceil();
    let ox = (x0 / step).floor() * step - 2.0 * step;
    let oy = (y0 / step).floor() * step - 2.0 * step;
    let gc = (((x1 - ox) / step).ceil() as usize + 3).max(2);
    let gr = (((y1 - oy) / step).ceil() as usize + 3).max(2);
    let mut grid: Vec<Option<[f32; 2]>> = vec![None; gc * gr];
    let src_at = |i: usize, j: usize| {
        [
            source[0] + source[2] * i as f32 / (cols - 1) as f32,
            source[1] + source[3] * j as f32 / (rows - 1) as f32,
        ]
    };
    for j in 0..rows - 1 {
        for i in 0..cols - 1 {
            let quad = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            for tri in [[0, 1, 2], [0, 2, 3]] {
                let d = tri.map(|k| points[quad[k].1 * cols + quad[k].0]);
                let s = tri.map(|k| src_at(quad[k].0, quad[k].1));
                let area = (d[1][0] - d[0][0]) * (d[2][1] - d[0][1])
                    - (d[2][0] - d[0][0]) * (d[1][1] - d[0][1]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let lo_x = d.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
                let hi_x = d.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
                let lo_y = d.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
                let hi_y = d.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
                let gi0 = ((lo_x - ox) / step).floor().max(0.0) as usize;
                let gi1 = (((hi_x - ox) / step).ceil() as usize).min(gc - 1);
                let gj0 = ((lo_y - oy) / step).floor().max(0.0) as usize;
                let gj1 = (((hi_y - oy) / step).ceil() as usize).min(gr - 1);
                for gj in gj0..=gj1 {
                    for gi in gi0..=gi1 {
                        let p = [ox + gi as f32 * step, oy + gj as f32 * step];
                        let w1 = ((p[0] - d[0][0]) * (d[2][1] - d[0][1])
                            - (d[2][0] - d[0][0]) * (p[1] - d[0][1]))
                            / area;
                        let w2 = ((d[1][0] - d[0][0]) * (p[1] - d[0][1])
                            - (p[0] - d[0][0]) * (d[1][1] - d[0][1]))
                            / area;
                        let w0 = 1.0 - w1 - w2;
                        const E: f32 = -1e-4;
                        if w0 >= E && w1 >= E && w2 >= E {
                            grid[gj * gc + gi] = Some([
                                s[0][0] * w0 + s[1][0] * w1 + s[2][0] * w2,
                                s[0][1] * w0 + s[1][1] * w1 + s[2][1] * w2,
                            ]);
                        }
                    }
                }
            }
        }
    }
    // two rings of linear extrapolation past the mesh's edge, so cells that straddle it
    // interpolate sensibly and the source box test cuts the edge cleanly
    for _ in 0..2 {
        let before = grid.clone();
        for gj in 0..gr {
            for gi in 0..gc {
                if before[gj * gc + gi].is_some() {
                    continue;
                }
                let mut sum = [0f32; 2];
                let mut n = 0;
                for (di, dj) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                    let at = |k: i64| {
                        let (x, y) = (gi as i64 + di * k, gj as i64 + dj * k);
                        if x < 0 || y < 0 || x >= gc as i64 || y >= gr as i64 {
                            None
                        } else {
                            before[y as usize * gc + x as usize]
                        }
                    };
                    if let (Some(a), Some(b)) = (at(1), at(2)) {
                        sum[0] += 2.0 * a[0] - b[0];
                        sum[1] += 2.0 * a[1] - b[1];
                        n += 1;
                    }
                }
                if n > 0 {
                    grid[gj * gc + gi] = Some([sum[0] / n as f32, sum[1] / n as f32]);
                }
            }
        }
    }
    let mut values = Vec::with_capacity(MESH_HEADER + gc * gr * 2);
    values.extend_from_slice(&[
        ox, oy, step, gc as f32, gr as f32, source[0], source[1], source[2], source[3],
    ]);
    for v in grid {
        // far outside the source, so the box test leaves these see through
        values.extend_from_slice(&v.unwrap_or([-1e9, -1e9]));
    }
    NodeData::new(values)
}

fn is_identity(m: &M3) -> bool {
    m.iter()
        .flatten()
        .zip(crate::color::Mat3::IDENTITY.0.iter().flatten())
        .all(|(a, b)| (a - b).abs() < 1e-9)
}

/// the same f32 maths as `map_point` in the kernel, so both sides pick the same pixels
fn map_point(inv: &[[f32; 3]; 3], x: f32, y: f32) -> [f32; 2] {
    let w = inv[2][0] * x + inv[2][1] * y + inv[2][2];
    if w.abs() < 1e-9 {
        return [-1e9, -1e9];
    }
    [
        (inv[0][0] * x + inv[0][1] * y + inv[0][2]) / w,
        (inv[1][0] * x + inv[1][1] * y + inv[1][2]) / w,
    ]
}

fn cubic_w(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ]
}

fn sample_at(img: &CpuImage, mode: u32, q: [f32; 2]) -> [f32; 4] {
    if mode == 0 {
        return img.get(q[0].floor() as i32, q[1].floor() as i32);
    }
    let s = [q[0] - 0.5, q[1] - 0.5];
    let (ix, iy) = (s[0].floor() as i32, s[1].floor() as i32);
    let (fx, fy) = (s[0] - s[0].floor(), s[1] - s[1].floor());
    let lerp = |a: [f32; 4], b: [f32; 4], t: f32| [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * t);
    if mode == 1 {
        let a = lerp(img.get(ix, iy), img.get(ix + 1, iy), fx);
        let b = lerp(img.get(ix, iy + 1), img.get(ix + 1, iy + 1), fx);
        return lerp(a, b, fy);
    }
    let (wx, wy) = (cubic_w(fx), cubic_w(fy));
    let mut acc = [0f32; 4];
    for (j, wyj) in wy.iter().enumerate() {
        let mut row = [0f32; 4];
        for (i, wxi) in wx.iter().enumerate() {
            let c = img.get(ix + i as i32 - 1, iy + j as i32 - 1);
            for k in 0..4 {
                row[k] += c[k] * wxi;
            }
        }
        for k in 0..4 {
            acc[k] += row[k] * wyj;
        }
    }
    let a = acc[3].max(0.0);
    [
        acc[0].clamp(0.0, a * 65504.0),
        acc[1].clamp(0.0, a * 65504.0),
        acc[2].clamp(0.0, a * 65504.0),
        a,
    ]
}

impl NodeKind for TransformNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn bounds(&self, cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        let s = setup(cx);
        if let Some(mesh) = &s.mesh {
            return match input_bounds(inputs, 0) {
                Bounds::Empty => Bounds::Empty,
                _ => Bounds::of(mesh.bounds(1.0 / cx.scale())),
            };
        }
        if s.inverse.is_none() {
            return Bounds::Empty;
        }
        match input_bounds(inputs, 0) {
            Bounds::Rect(r) => match map_rect(&s.forward, r) {
                Some(out) => Bounds::of(out.expand(1)),
                None => Bounds::Infinite,
            },
            other => other,
        }
    }

    fn input_region(&self, cx: &NodeCx, _input: usize, region: IRect) -> IRect {
        let s = setup(cx);
        if let Some(mesh) = &s.mesh {
            return mesh.source_at(1.0 / cx.scale()).expand(3);
        }
        match s.inverse.and_then(|inv| map_rect(&inv, region)) {
            // bicubic reads two pixels around each sample, shrinking reads a little more
            Some(r) => r.expand(3),
            // part of the region sees past the horizon of a perspective; read a generous
            // but bounded neighbourhood rather than an infinite input
            None => region.expand(PAST_HORIZON),
        }
    }

    fn key(&self, cx: &NodeCx, _region: IRect, h: &mut dyn Hasher) {
        if let Some(d) = cx.data() {
            h.write_u64(d.key);
        }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let s = setup(cx);
        let Some(input) = inputs.first().copied().flatten() else {
            return Out::Empty;
        };
        let inv = match (&s.mesh, s.inverse) {
            (Some(_), _) => None,
            (None, Some(inv)) => {
                if is_identity(&s.forward) {
                    return Out::Input(0);
                }
                Some(inv.map(|r| r.map(|v| v as f32)))
            }
            (None, None) => return Out::Empty,
        };
        let mode = s.mode;
        let scale = 1.0 / cx.scale();
        let mesh = s.mesh.as_ref();
        Out::New(CpuImage::from_fn(region, |x, y| {
            let (cxp, cyp) = (x as f32 + 0.5, y as f32 + 0.5);
            let (q, dx, dy) = match (mesh, &inv) {
                (Some(m), _) => {
                    let d = [cxp * scale, cyp * scale];
                    let src = m.src(d);
                    if !m.inside_source(src) {
                        return [0.0; 4];
                    }
                    let q = [src[0] / scale, src[1] / scale];
                    let a = m.src([d[0] + scale, d[1]]);
                    let b = m.src([d[0], d[1] + scale]);
                    (
                        q,
                        [a[0] / scale - q[0], a[1] / scale - q[1]],
                        [b[0] / scale - q[0], b[1] / scale - q[1]],
                    )
                }
                (None, Some(inv)) => {
                    let q = map_point(inv, cxp, cyp);
                    let qx = map_point(inv, cxp + 1.0, cyp);
                    let qy = map_point(inv, cxp, cyp + 1.0);
                    (
                        q,
                        [qx[0] - q[0], qx[1] - q[1]],
                        [qy[0] - q[0], qy[1] - q[1]],
                    )
                }
                (None, None) => return [0.0; 4],
            };
            let (nx, ny) = if mode == 0 {
                (1, 1)
            } else {
                (
                    ((dx[0] * dx[0] + dx[1] * dx[1]).sqrt().ceil() as i32).clamp(1, MAX_TAPS),
                    ((dy[0] * dy[0] + dy[1] * dy[1]).sqrt().ceil() as i32).clamp(1, MAX_TAPS),
                )
            };
            let mut acc = [0f32; 4];
            for j in 0..ny {
                for i in 0..nx {
                    let u = (i as f32 + 0.5) / nx as f32 - 0.5;
                    let v = (j as f32 + 0.5) / ny as f32 - 0.5;
                    let c = sample_at(
                        input,
                        mode,
                        [q[0] + dx[0] * u + dy[0] * v, q[1] + dx[1] * u + dy[1] * v],
                    );
                    for k in 0..4 {
                        acc[k] += c[k];
                    }
                }
            }
            let n = (nx * ny) as f32;
            acc.map(|v| v / n)
        }))
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let s = setup(cx);
        let Some(input) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        if let Some(mesh) = &s.mesh {
            let params = [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [s.mode as f32, 1.0, 1.0 / cx.scale(), 0.0],
            ];
            return Ok(Out::New(gx.dispatch(
                &KERNEL,
                &[Some(input)],
                region,
                Dispatch {
                    params: &params,
                    data: Some(mesh.values),
                    ..Default::default()
                },
            )?));
        }
        let Some(inv) = s.inverse else {
            return Ok(Out::Empty);
        };
        if is_identity(&s.forward) {
            return Ok(Out::Input(0));
        }
        let row = |r: [f64; 3]| [r[0] as f32, r[1] as f32, r[2] as f32, 0.0];
        let params = [
            row(inv[0]),
            row(inv[1]),
            row(inv[2]),
            [s.mode as f32, 0.0, 0.0, 0.0],
        ];
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[Some(input)],
            region,
            Dispatch {
                params: &params,
                ..Default::default()
            },
        )?))
    }
}

/// builds a transform matrix: scale about the origin, then rotate (degrees, clockwise on
/// screen), then translate. later phases build richer ones, this keeps tests short.
pub fn affine(scale: f32, rotate_deg: f32, tx: f32, ty: f32) -> [f32; 9] {
    let (s, c) = rotate_deg.to_radians().sin_cos();
    [
        scale * c,
        -scale * s,
        tx,
        scale * s,
        scale * c,
        ty,
        0.0,
        0.0,
        1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_scale_the_translation_and_perspective() {
        let m = mat([2.0, 0.0, 10.0, 0.0, 2.0, 20.0, 0.001, 0.0, 1.0]);
        let l1 = at_level(&m, 1);
        assert_eq!(l1[0][2], 5.0);
        assert_eq!(l1[1][2], 10.0);
        assert!((l1[2][0] - 0.002).abs() < 1e-9);
        // a point at level 1 maps to half of where it maps at level 0
        let (x0, y0) = apply(&m, 40.0, 40.0).unwrap();
        let (x1, y1) = apply(&l1, 20.0, 20.0).unwrap();
        assert!((x0 / 2.0 - x1).abs() < 1e-9 && (y0 / 2.0 - y1).abs() < 1e-9);
    }

    #[test]
    fn a_mesh_that_only_moves_reads_from_the_shifted_place() {
        // a 3 by 3 mesh moved 10 right and 5 down
        let src = [0.0, 0.0, 40.0, 20.0];
        let pts: Vec<[f32; 2]> = (0..9)
            .map(|k| [(k % 3) as f32 * 20.0 + 10.0, (k / 3) as f32 * 10.0 + 5.0])
            .collect();
        let data = mesh_grid(src, 3, 3, &pts);
        let m = Mesh::read(&data.values).unwrap();
        let back = m.src([30.0, 15.0]);
        assert!((back[0] - 20.0).abs() < 1e-3 && (back[1] - 10.0).abs() < 1e-3);
        // just past the edge it extrapolates and lands outside the source box
        assert!(!m.inside_source(m.src([5.0, 15.0])));
        assert!(m.bounds(1.0).contains_rect(&IRect::new(10, 5, 40, 20)));
    }

    #[test]
    fn rects_map_through_rotation() {
        let m = mat(affine(1.0, 90.0, 0.0, 0.0));
        let r = map_rect(&m, IRect::new(0, 0, 10, 4)).unwrap();
        assert_eq!(r, IRect::new(-4, 0, 4, 10));
    }
}
