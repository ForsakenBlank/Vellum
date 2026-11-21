//! moves, scales, rotates, skews and distorts an image with a 3x3 matrix, which covers
//! perspective too. sampling is nearest, bilinear or bicubic (catmull-rom), and shrinking
//! takes several samples per pixel so fine detail does not shimmer.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out, input_bounds};
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

fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = vec2<f32>(p) + 0.5;
    let q = map_point(c);
    let dx = map_point(c + vec2<f32>(1.0, 0.0)) - q;
    let dy = map_point(c + vec2<f32>(0.0, 1.0)) - q;
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

struct Setup {
    forward: M3,
    inverse: Option<M3>,
    mode: u32,
}

fn setup(cx: &NodeCx) -> Setup {
    let m = at_level(&mat(cx.params.matrix("matrix")), cx.level);
    Setup {
        inverse: invert(&m),
        forward: m,
        mode: cx.params.choice_index("interpolation"),
    }
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
        match s.inverse.and_then(|inv| map_rect(&inv, region)) {
            // bicubic reads two pixels around each sample, shrinking reads a little more
            Some(r) => r.expand(3),
            // part of the region sees past the horizon of a perspective; read a generous
            // but bounded neighbourhood rather than an infinite input
            None => region.expand(PAST_HORIZON),
        }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let s = setup(cx);
        let (Some(input), Some(inv)) = (inputs.first().copied().flatten(), s.inverse) else {
            return Out::Empty;
        };
        if is_identity(&s.forward) {
            return Out::Input(0);
        }
        let mode = s.mode;
        let inv = inv.map(|r| r.map(|v| v as f32));
        Out::New(CpuImage::from_fn(region, |x, y| {
            let (cxp, cyp) = (x as f32 + 0.5, y as f32 + 0.5);
            let q = map_point(&inv, cxp, cyp);
            let qx = map_point(&inv, cxp + 1.0, cyp);
            let qy = map_point(&inv, cxp, cyp + 1.0);
            let dx = [qx[0] - q[0], qx[1] - q[1]];
            let dy = [qy[0] - q[0], qy[1] - q[1]];
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
        let (Some(input), Some(inv)) = (inputs.first().copied().flatten(), s.inverse) else {
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
    fn rects_map_through_rotation() {
        let m = mat(affine(1.0, 90.0, 0.0, 0.0));
        let r = map_rect(&m, IRect::new(0, 0, 10, 4)).unwrap();
        assert_eq!(r, IRect::new(-4, 0, 4, 10));
    }
}
