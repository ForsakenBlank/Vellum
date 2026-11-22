//! the blur gallery besides gaussian blur: motion, radial (spin and zoom), lens, the focus
//! blurs (tilt-shift, iris and field) and surface blur.
//!
//! blurs that gather from far away sample a fixed number of points rather than every
//! pixel in reach, so their cost does not grow with the radius. lens and focus blurs
//! sample a vogel spiral, which covers a disc evenly with any number of points.

use crate::gpu::Kernel;
use crate::graph::{NodeCategory, NodeCx, NodeInfo, ONE_INPUT};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};

use super::colorfn::{luma, sample, smoothstep};
use super::simple::{Setup, Simple};

const GOLDEN_ANGLE: f32 = 2.399_963_2;

// ---- motion blur ----

pub struct MotionBlur;

static MOTION: NodeInfo = NodeInfo {
    id: "effect.motion_blur",
    title: "Motion blur",
    category: NodeCategory::Effect,
    description: "Smears the picture along a direction, like a moving camera.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::degrees("angle", "Angle", -180.0, 180.0, 0.0),
        ParamSpec::pixels("distance", "Distance", 999.0, 10.0),
    ],
};

const MOTION_KERNEL: Kernel = Kernel {
    name: "motion_blur",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let n = i32(a.w);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    var acc = vec4<f32>(0.0);
    for (var i = 0; i <= n; i = i + 1) {
        let t = f32(i) / f32(max(n, 1)) - 0.5;
        acc = acc + sample0(pos + a.xy * (t * a.z), true);
    }
    return acc / f32(n + 1);
}
"#,
};

impl Simple for MotionBlur {
    fn info(&self) -> &'static NodeInfo {
        &MOTION
    }
    fn kernel(&self) -> &'static Kernel {
        &MOTION_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let d = cx.params.float("distance") * cx.scale();
        if d < 0.5 {
            return None;
        }
        let (s, c) = cx.params.float("angle").to_radians().sin_cos();
        let n = d.ceil().max(1.0);
        Some(Setup::new(vec![[c, -s, d, n]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("distance") * cx.scale() * 0.5).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let n = a[3] as i32;
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let mut acc = [0.0; 4];
        for i in 0..=n {
            let t = i as f32 / n.max(1) as f32 - 0.5;
            let c = sample(
                input,
                [pos[0] + a[0] * t * a[2], pos[1] + a[1] * t * a[2]],
                true,
            );
            for k in 0..4 {
                acc[k] += c[k];
            }
        }
        acc.map(|v| v / (n + 1) as f32)
    }
}

// ---- radial blur ----

pub struct RadialBlur;

const RADIAL_MODES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "spin",
        label: "Spin",
    },
    ChoiceOption {
        id: "zoom",
        label: "Zoom",
    },
];

static RADIAL: NodeInfo = NodeInfo {
    id: "effect.radial_blur",
    title: "Radial blur",
    category: NodeCategory::Effect,
    description: "Spins the picture around a point, or zooms it towards one.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("mode", "Method", RADIAL_MODES, "spin"),
        ParamSpec::float("amount", "Amount", 1.0, 100.0, 10.0),
        ParamSpec::point("center", "Centre", [0.0, 0.0]),
    ],
};

const RADIAL_TAPS: i32 = 32;

const RADIAL_KERNEL: Kernel = Kernel {
    name: "radial_blur",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let d = pos - a.xy;
    var acc = vec4<f32>(0.0);
    for (var i = 0; i < 32; i = i + 1) {
        let t = f32(i) / 31.0 - 0.5;
        var q: vec2<f32>;
        if (a.w < 0.5) {
            let ang = t * a.z;
            let cs = cos(ang); let sn = sin(ang);
            q = a.xy + vec2<f32>(d.x * cs - d.y * sn, d.x * sn + d.y * cs);
        } else {
            q = a.xy + d * (1.0 - (t + 0.5) * a.z);
        }
        acc = acc + sample0(q, true);
    }
    return acc / 32.0;
}
"#,
};

impl Simple for RadialBlur {
    fn info(&self) -> &'static NodeInfo {
        &RADIAL
    }
    fn kernel(&self) -> &'static Kernel {
        &RADIAL_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let c = p.point("center");
        let amount = p.float("amount") / 100.0;
        let zoom = p.choice_index("mode") == 1;
        let span = if zoom {
            amount * 0.6
        } else {
            amount * 60f32.to_radians()
        };
        Some(Setup::new(vec![[
            c[0] * cx.scale(),
            c[1] * cx.scale(),
            span,
            f32::from(u8::from(zoom)),
        ]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        // samples may come from anywhere on the picture
        (4096.0 * cx.scale()).ceil() as i32
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let d = [pos[0] - a[0], pos[1] - a[1]];
        let mut acc = [0.0; 4];
        for i in 0..RADIAL_TAPS {
            let t = i as f32 / (RADIAL_TAPS - 1) as f32 - 0.5;
            let q = if a[3] < 0.5 {
                let (sn, cs) = (t * a[2]).sin_cos();
                [a[0] + d[0] * cs - d[1] * sn, a[1] + d[0] * sn + d[1] * cs]
            } else {
                let k = 1.0 - (t + 0.5) * a[2];
                [a[0] + d[0] * k, a[1] + d[1] * k]
            };
            let c = sample(input, q, true);
            for k in 0..4 {
                acc[k] += c[k];
            }
        }
        acc.map(|v| v / RADIAL_TAPS as f32)
    }
}

// ---- lens blur ----

pub struct LensBlur;

static LENS: NodeInfo = NodeInfo {
    id: "effect.lens_blur",
    title: "Lens blur",
    category: NodeCategory::Effect,
    description: "Blurs like an out of focus lens, with bright spots turning into discs.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("radius", "Radius", 100.0, 8.0),
        ParamSpec::percent("brightness", "Specular brightness", 0.0, 100.0, 0.0),
        ParamSpec::float("threshold", "Specular threshold", 0.0, 255.0, 230.0),
    ],
};

const LENS_TAPS: i32 = 64;

const LENS_KERNEL: Kernel = Kernel {
    name: "lens_blur",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0; i < 64; i = i + 1) {
        let r = sqrt((f32(i) + 0.5) / 64.0) * a.x;
        let th = f32(i) * 2.3999632;
        let c = sample0(pos + vec2<f32>(cos(th), sin(th)) * r, true);
        let l = luma(unpremultiply(c).rgb);
        let w = 1.0 + a.y * 8.0 * smoothstep(a.z, 1.0, l);
        acc = acc + c * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}
"#,
};

/// a point on a vogel spiral of `n` points filling a disc of radius `r`
fn vogel(i: i32, n: i32, r: f32) -> [f32; 2] {
    let rr = ((i as f32 + 0.5) / n as f32).sqrt() * r;
    let (s, c) = (i as f32 * GOLDEN_ANGLE).sin_cos();
    [c * rr, s * rr]
}

impl Simple for LensBlur {
    fn info(&self) -> &'static NodeInfo {
        &LENS
    }
    fn kernel(&self) -> &'static Kernel {
        &LENS_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let r = p.float("radius") * cx.scale();
        if r < 0.5 {
            return None;
        }
        Some(Setup::new(vec![[
            r,
            p.float("brightness") / 100.0,
            p.float("threshold") / 255.0,
            0.0,
        ]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("radius") * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let (mut acc, mut wsum) = ([0.0; 4], 0.0);
        for i in 0..LENS_TAPS {
            let o = vogel(i, LENS_TAPS, a[0]);
            let c = sample(input, [pos[0] + o[0], pos[1] + o[1]], true);
            let u = super::unpremultiply(c);
            let w = 1.0 + a[1] * 8.0 * smoothstep(a[2], 1.0, luma([u[0], u[1], u[2]]));
            for k in 0..4 {
                acc[k] += c[k] * w;
            }
            wsum += w;
        }
        acc.map(|v| v / wsum)
    }
}

// ---- tilt-shift, iris and field blur ----

pub struct FocusBlur;

const FOCUS_MODES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "tilt_shift",
        label: "Tilt-shift",
    },
    ChoiceOption {
        id: "iris",
        label: "Iris",
    },
    ChoiceOption {
        id: "field",
        label: "Field",
    },
];

static FOCUS: NodeInfo = NodeInfo {
    id: "effect.focus_blur",
    title: "Focus blur",
    category: NodeCategory::Effect,
    description: "Keeps a band, an oval or one point sharp and blurs away from it.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("mode", "Kind", FOCUS_MODES, "tilt_shift"),
        ParamSpec::pixels("blur", "Blur", 200.0, 15.0),
        ParamSpec::point("center", "Focus point", [0.0, 0.0]),
        ParamSpec::degrees("angle", "Angle", -180.0, 180.0, 0.0),
        ParamSpec::pixels("focus", "Sharp area", 5000.0, 60.0),
        ParamSpec::pixels("transition", "Transition", 5000.0, 80.0),
        ParamSpec::float("aspect", "Oval shape", 0.1, 10.0, 1.0),
        ParamSpec::point("end", "Second pin", [100.0, 0.0]),
        ParamSpec::pixels("end_blur", "Second pin blur", 200.0, 0.0),
    ],
};

const FOCUS_TAPS: i32 = 48;

const FOCUS_KERNEL: Kernel = Kernel {
    name: "focus_blur",
    body: r#"
fn focus_radius(pos: vec2<f32>) -> f32 {
    let a = param(0u); let b = param(1u); let c = param(2u);
    let d = pos - a.xy;
    if (c.w > 1.5) {
        let ab = c.xy - a.xy;
        let t = clamp(dot(d, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
        return mix(a.z, c.z, t);
    }
    var dist: f32;
    if (c.w > 0.5) {
        let u = d.x * b.x + d.y * b.y;
        let v = (-d.x * b.y + d.y * b.x) * b.w;
        dist = length(vec2<f32>(u, v));
    } else {
        dist = abs(-d.x * b.y + d.y * b.x);
    }
    return a.z * smoothstep(a.w, a.w + b.z, dist);
}

fn kernel(p: vec2<i32>) -> vec4<f32> {
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let r = focus_radius(pos);
    if (r < 0.3) { return load0(p); }
    var acc = vec4<f32>(0.0);
    for (var i = 0; i < 48; i = i + 1) {
        let rr = sqrt((f32(i) + 0.5) / 48.0) * r;
        let th = f32(i) * 2.3999632;
        acc = acc + sample0(pos + vec2<f32>(cos(th), sin(th)) * rr, true);
    }
    return acc / 48.0;
}
"#,
};

fn focus_radius(s: &Setup, pos: [f32; 2]) -> f32 {
    let (a, b, c) = (s.p(0), s.p(1), s.p(2));
    let d = [pos[0] - a[0], pos[1] - a[1]];
    if c[3] > 1.5 {
        let ab = [c[0] - a[0], c[1] - a[1]];
        let t = ((d[0] * ab[0] + d[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6))
            .clamp(0.0, 1.0);
        return a[2] + (c[2] - a[2]) * t;
    }
    let dist = if c[3] > 0.5 {
        let u = d[0] * b[0] + d[1] * b[1];
        let v = (-d[0] * b[1] + d[1] * b[0]) * b[3];
        u.hypot(v)
    } else {
        (-d[0] * b[1] + d[1] * b[0]).abs()
    };
    a[2] * smoothstep(a[3], a[3] + b[2], dist)
}

impl Simple for FocusBlur {
    fn info(&self) -> &'static NodeInfo {
        &FOCUS
    }
    fn kernel(&self) -> &'static Kernel {
        &FOCUS_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let sc = cx.scale();
        let mode = p.choice_index("mode");
        let blur = p.float("blur") * sc;
        let end_blur = p.float("end_blur") * sc;
        if blur < 0.3 && (mode != 2 || end_blur < 0.3) {
            return None;
        }
        let c = p.point("center");
        let e = p.point("end");
        let (s, co) = p.float("angle").to_radians().sin_cos();
        Some(Setup::new(vec![
            [c[0] * sc, c[1] * sc, blur, p.float("focus") * sc * 0.5],
            [co, s, p.float("transition") * sc, 1.0 / p.float("aspect")],
            [e[0] * sc, e[1] * sc, end_blur, mode as f32],
        ]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("blur").max(cx.params.float("end_blur")) * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let r = focus_radius(s, pos);
        if r < 0.3 {
            return input.get(x, y);
        }
        let mut acc = [0.0; 4];
        for i in 0..FOCUS_TAPS {
            let o = vogel(i, FOCUS_TAPS, r);
            let c = sample(input, [pos[0] + o[0], pos[1] + o[1]], true);
            for k in 0..4 {
                acc[k] += c[k];
            }
        }
        acc.map(|v| v / FOCUS_TAPS as f32)
    }
}

// ---- surface blur ----

pub struct SurfaceBlur;

static SURFACE: NodeInfo = NodeInfo {
    id: "effect.surface_blur",
    title: "Surface blur",
    category: NodeCategory::Effect,
    description: "Smooths flat areas and keeps edges sharp.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("radius", "Radius", 100.0, 5.0),
        ParamSpec::float("threshold", "Threshold", 2.0, 255.0, 15.0),
    ],
};

/// at most this many samples each way from the centre
const SURFACE_STEPS: i32 = 6;

const SURFACE_KERNEL: Kernel = Kernel {
    name: "surface_blur",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let n = i32(a.x);
    let step = a.y;
    let c = load0(p);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let lc = luma(c.rgb);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var j = -n; j <= n; j = j + 1) {
        for (var i = -n; i <= n; i = i + 1) {
            let q = sample0(pos + vec2<f32>(f32(i), f32(j)) * step, true);
            let w = max(1.0 - abs(luma(q.rgb) - lc) / a.z, 0.0);
            acc = acc + q * w;
            wsum = wsum + w;
        }
    }
    return acc / max(wsum, 1e-6);
}
"#,
};

impl Simple for SurfaceBlur {
    fn info(&self) -> &'static NodeInfo {
        &SURFACE
    }
    fn kernel(&self) -> &'static Kernel {
        &SURFACE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let r = cx.params.float("radius") * cx.scale();
        if r < 0.5 {
            return None;
        }
        let n = (r.ceil() as i32).min(SURFACE_STEPS);
        let step = r / n as f32;
        Some(Setup::new(vec![[
            n as f32,
            step,
            cx.params.float("threshold") / 255.0 * 2.5,
            0.0,
        ]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("radius") * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let n = a[0] as i32;
        let c = input.get(x, y);
        let lc = luma([c[0], c[1], c[2]]);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let (mut acc, mut wsum) = ([0.0; 4], 0.0);
        for j in -n..=n {
            for i in -n..=n {
                let q = sample(
                    input,
                    [pos[0] + i as f32 * a[1], pos[1] + j as f32 * a[1]],
                    true,
                );
                let w = (1.0 - (luma([q[0], q[1], q[2]]) - lc).abs() / a[2]).max(0.0);
                for k in 0..4 {
                    acc[k] += q[k] * w;
                }
                wsum += w;
            }
        }
        acc.map(|v| v / wsum.max(1e-6))
    }
}
