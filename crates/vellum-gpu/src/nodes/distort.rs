//! distortions: each output pixel reads the input from somewhere else. wave, ripple,
//! twirl, polar coordinates, displace, chromatic aberration and glitch.
//!
//! positions are in level pixels, so lengths in document pixels are scaled by the zoom
//! level and the pattern stays put while zooming. samples are bilinear with edges repeated,
//! the same as the transform node's default.

use crate::gpu::Kernel;
use crate::graph::{NodeCategory, NodeCx, NodeInfo, ONE_INPUT};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};

use super::colorfn::{luma, sample};
use super::simple::{Setup, Simple};
use super::unpremultiply;
use crate::blend::{hash_u32, pixel_noise};

const TAU: f32 = std::f32::consts::TAU;

// ---- wave ----

pub struct Wave;

const WAVE_TYPES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "sine",
        label: "Sine",
    },
    ChoiceOption {
        id: "triangle",
        label: "Triangle",
    },
    ChoiceOption {
        id: "square",
        label: "Square",
    },
];

static WAVE: NodeInfo = NodeInfo {
    id: "effect.wave",
    title: "Wave",
    category: NodeCategory::Effect,
    description: "Bends the picture in waves across and down.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("kind", "Type", WAVE_TYPES, "sine"),
        ParamSpec::pixels("wavelength", "Wavelength", 2000.0, 60.0),
        ParamSpec::pixels("amplitude", "Amplitude", 500.0, 8.0),
        ParamSpec::percent("horizontal", "Horizontal", 0.0, 100.0, 100.0),
        ParamSpec::percent("vertical", "Vertical", 0.0, 100.0, 100.0),
    ],
};

const WAVE_KERNEL: Kernel = Kernel {
    name: "wave",
    body: r#"
fn wave1(kind: f32, t: f32) -> f32 {
    let f = fract(t);
    if (kind < 0.5) { return sin(t * 6.2831853); }
    if (kind < 1.5) { return 1.0 - 4.0 * abs(f - 0.5); }
    return select(-1.0, 1.0, f < 0.5);
}
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let dx = a.z * a.w * wave1(a.x, pos.y / a.y);
    let dy = a.z * param(1u).x * wave1(a.x, pos.x / a.y);
    return sample0(pos + vec2<f32>(dx, dy), true);
}
"#,
};

fn wave1(kind: f32, t: f32) -> f32 {
    let f = t - t.floor();
    if kind < 0.5 {
        (t * TAU).sin()
    } else if kind < 1.5 {
        1.0 - 4.0 * (f - 0.5).abs()
    } else if f < 0.5 {
        1.0
    } else {
        -1.0
    }
}

impl Simple for Wave {
    fn info(&self) -> &'static NodeInfo {
        &WAVE
    }
    fn kernel(&self) -> &'static Kernel {
        &WAVE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let amp = p.float("amplitude") * cx.scale();
        if amp == 0.0 {
            return None;
        }
        Some(Setup::new(vec![
            [
                p.choice_index("kind") as f32,
                (p.float("wavelength") * cx.scale()).max(1.0),
                amp,
                p.float("horizontal") / 100.0,
            ],
            [p.float("vertical") / 100.0, 0.0, 0.0, 0.0],
        ]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("amplitude") * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let dx = a[2] * a[3] * wave1(a[0], pos[1] / a[1]);
        let dy = a[2] * s.p(1)[0] * wave1(a[0], pos[0] / a[1]);
        sample(input, [pos[0] + dx, pos[1] + dy], true)
    }
}

// ---- ripple ----

pub struct Ripple;

const RIPPLE_SIZES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "small",
        label: "Small",
    },
    ChoiceOption {
        id: "medium",
        label: "Medium",
    },
    ChoiceOption {
        id: "large",
        label: "Large",
    },
];

static RIPPLE: NodeInfo = NodeInfo {
    id: "effect.ripple",
    title: "Ripple",
    category: NodeCategory::Effect,
    description: "Makes the picture look like it is seen through rippling water.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("amount", "Amount", -999.0, 999.0, 100.0),
        ParamSpec::choice("size", "Size", RIPPLE_SIZES, "medium"),
    ],
};

const RIPPLE_KERNEL: Kernel = Kernel {
    name: "ripple",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let d0 = (pos + vec2<f32>(0.5)) / a.y;
    let dx = sin(d0.y * 6.2831853 + sin(d0.x * 2.1)) * a.x;
    let dy = sin(d0.x * 6.2831853 + sin(d0.y * 1.7)) * a.x;
    return sample0(pos + vec2<f32>(dx, dy), true);
}
"#,
};

impl Simple for Ripple {
    fn info(&self) -> &'static NodeInfo {
        &RIPPLE
    }
    fn kernel(&self) -> &'static Kernel {
        &RIPPLE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let amp = p.float("amount") / 100.0 * 3.0 * cx.scale();
        if amp == 0.0 {
            return None;
        }
        let len = [8.0, 16.0, 32.0][p.choice_index("size").min(2) as usize] * cx.scale();
        Some(Setup::new(vec![[amp, len.max(1.0), 0.0, 0.0]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("amount").abs() / 100.0 * 3.0 * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let d0 = [(pos[0] + 0.5) / a[1], (pos[1] + 0.5) / a[1]];
        let dx = (d0[1] * TAU + (d0[0] * 2.1).sin()).sin() * a[0];
        let dy = (d0[0] * TAU + (d0[1] * 1.7).sin()).sin() * a[0];
        sample(input, [pos[0] + dx, pos[1] + dy], true)
    }
}

// ---- twirl ----

pub struct Twirl;

static TWIRL: NodeInfo = NodeInfo {
    id: "effect.twirl",
    title: "Twirl",
    category: NodeCategory::Effect,
    description: "Turns the middle of an area more than its edge, like a whirlpool.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::degrees("angle", "Angle", -999.0, 999.0, 50.0),
        ParamSpec::pixels("radius", "Radius", 10000.0, 200.0),
        ParamSpec::point("center", "Centre", [0.0, 0.0]),
    ],
};

const TWIRL_KERNEL: Kernel = Kernel {
    name: "twirl",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let d = pos - a.xy;
    let r = length(d);
    if (r >= a.w) { return load0(p); }
    let k = 1.0 - r / a.w;
    let ang = a.z * k * k;
    let cs = cos(ang); let sn = sin(ang);
    return sample0(a.xy + vec2<f32>(d.x * cs - d.y * sn, d.x * sn + d.y * cs), true);
}
"#,
};

impl Simple for Twirl {
    fn info(&self) -> &'static NodeInfo {
        &TWIRL
    }
    fn kernel(&self) -> &'static Kernel {
        &TWIRL_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let c = p.point("center");
        let angle = p.float("angle").to_radians();
        if angle == 0.0 {
            return None;
        }
        Some(Setup::new(vec![[
            c[0] * cx.scale(),
            c[1] * cx.scale(),
            angle,
            (p.float("radius") * cx.scale()).max(1.0),
        ]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("radius") * cx.scale() * 2.0).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let d = [pos[0] - a[0], pos[1] - a[1]];
        let r = d[0].hypot(d[1]);
        if r >= a[3] {
            return input.get(x, y);
        }
        let k = 1.0 - r / a[3];
        let (sn, cs) = (a[2] * k * k).sin_cos();
        sample(
            input,
            [a[0] + d[0] * cs - d[1] * sn, a[1] + d[0] * sn + d[1] * cs],
            true,
        )
    }
}

// ---- polar coordinates ----

pub struct Polar;

const POLAR_MODES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "to_polar",
        label: "Rectangular to polar",
    },
    ChoiceOption {
        id: "to_rect",
        label: "Polar to rectangular",
    },
];

static POLAR: NodeInfo = NodeInfo {
    id: "effect.polar",
    title: "Polar coordinates",
    category: NodeCategory::Effect,
    description: "Wraps the picture into a circle, or unrolls a circle into a strip.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("mode", "Conversion", POLAR_MODES, "to_polar"),
        ParamSpec::point("center", "Centre", [0.0, 0.0]).hide(),
        ParamSpec::point("extent", "Half size", [0.0, 0.0]).hide(),
    ],
};

const POLAR_KERNEL: Kernel = Kernel {
    name: "polar",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let e = max(a.zw, vec2<f32>(1.0));
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let origin = a.xy - e;
    var q: vec2<f32>;
    if (param(1u).x < 0.5) {
        let rel = (pos - a.xy) / e;
        let ang = atan2(rel.x, -rel.y);
        q = origin + vec2<f32>((ang / 6.2831853 + 0.5) * 2.0 * e.x, length(rel) * 2.0 * e.y);
    } else {
        let u = (pos - origin) / (2.0 * e);
        let ang = (u.x - 0.5) * 6.2831853;
        q = a.xy + vec2<f32>(sin(ang) * u.y * e.x, -cos(ang) * u.y * e.y);
    }
    return sample0(q, true);
}
"#,
};

impl Simple for Polar {
    fn info(&self) -> &'static NodeInfo {
        &POLAR
    }
    fn kernel(&self) -> &'static Kernel {
        &POLAR_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let (c, e) = (p.point("center"), p.point("extent"));
        let s = cx.scale();
        Some(Setup::new(vec![
            [c[0] * s, c[1] * s, e[0] * s, e[1] * s],
            [p.choice_index("mode") as f32, 0.0, 0.0, 0.0],
        ]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        let e = cx.params.point("extent");
        ((e[0] + e[1]) * 2.0 * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let e = [a[2].max(1.0), a[3].max(1.0)];
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let origin = [a[0] - e[0], a[1] - e[1]];
        let q = if s.p(1)[0] < 0.5 {
            let rel = [(pos[0] - a[0]) / e[0], (pos[1] - a[1]) / e[1]];
            let ang = rel[0].atan2(-rel[1]);
            [
                origin[0] + (ang / TAU + 0.5) * 2.0 * e[0],
                origin[1] + rel[0].hypot(rel[1]) * 2.0 * e[1],
            ]
        } else {
            let u = [
                (pos[0] - origin[0]) / (2.0 * e[0]),
                (pos[1] - origin[1]) / (2.0 * e[1]),
            ];
            let (sn, cs) = ((u[0] - 0.5) * TAU).sin_cos();
            [a[0] + sn * u[1] * e[0], a[1] - cs * u[1] * e[1]]
        };
        sample(input, q, true)
    }
}

// ---- displace ----

pub struct Displace;

static DISPLACE: NodeInfo = NodeInfo {
    id: "effect.displace",
    title: "Displace",
    category: NodeCategory::Effect,
    description: "Shifts each pixel by how bright the picture is there, like a wrinkled surface.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("horizontal", "Horizontal scale", 999.0, 10.0),
        ParamSpec::pixels("vertical", "Vertical scale", 999.0, 10.0),
    ],
};

const DISPLACE_KERNEL: Kernel = Kernel {
    name: "displace",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let m = luma(unpremultiply(load0(p)).rgb) - 0.5;
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    return sample0(pos + vec2<f32>(m * a.x, m * a.y) * 2.0, true);
}
"#,
};

impl Simple for Displace {
    fn info(&self) -> &'static NodeInfo {
        &DISPLACE
    }
    fn kernel(&self) -> &'static Kernel {
        &DISPLACE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let (h, v) = (
            p.float("horizontal") * cx.scale(),
            p.float("vertical") * cx.scale(),
        );
        if h == 0.0 && v == 0.0 {
            return None;
        }
        Some(Setup::new(vec![[h, v, 0.0, 0.0]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        let p = cx.params;
        (p.float("horizontal").max(p.float("vertical")) * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let c = unpremultiply(input.get(x, y));
        let m = luma([c[0], c[1], c[2]]) - 0.5;
        sample(
            input,
            [
                x as f32 + 0.5 + m * a[0] * 2.0,
                y as f32 + 0.5 + m * a[1] * 2.0,
            ],
            true,
        )
    }
}

// ---- chromatic aberration ----

pub struct ChromaticAberration;

static CHROMATIC: NodeInfo = NodeInfo {
    id: "effect.chromatic_aberration",
    title: "Chromatic aberration",
    category: NodeCategory::Effect,
    description: "Splits red and blue apart towards the edges, like a cheap lens.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("amount", "Amount", 100.0, 4.0),
        ParamSpec::point("center", "Centre", [0.0, 0.0]).hide(),
        ParamSpec::point("extent", "Half size", [0.0, 0.0]).hide(),
    ],
};

const CHROMATIC_KERNEL: Kernel = Kernel {
    name: "chromatic_aberration",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let d = (pos - a.xy) / max(length(a.zw), 1.0);
    let off = d * param(1u).x;
    let r = sample0(pos + off, true);
    let g = load0(p);
    let b = sample0(pos - off, true);
    let ag = g.a;
    return vec4<f32>(r.r / max(r.a, 1e-6) * ag, g.g, b.b / max(b.a, 1e-6) * ag, ag);
}
"#,
};

impl Simple for ChromaticAberration {
    fn info(&self) -> &'static NodeInfo {
        &CHROMATIC
    }
    fn kernel(&self) -> &'static Kernel {
        &CHROMATIC_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let amount = p.float("amount") * cx.scale();
        if amount == 0.0 {
            return None;
        }
        let (c, e) = (p.point("center"), p.point("extent"));
        let s = cx.scale();
        Some(Setup::new(vec![
            [c[0] * s, c[1] * s, e[0] * s, e[1] * s],
            [amount, 0.0, 0.0, 0.0],
        ]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("amount") * cx.scale() * 1.5).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let n = a[2].hypot(a[3]).max(1.0);
        let off = [
            (pos[0] - a[0]) / n * s.p(1)[0],
            (pos[1] - a[1]) / n * s.p(1)[0],
        ];
        let r = sample(input, [pos[0] + off[0], pos[1] + off[1]], true);
        let g = input.get(x, y);
        let b = sample(input, [pos[0] - off[0], pos[1] - off[1]], true);
        let ag = g[3];
        [
            r[0] / r[3].max(1e-6) * ag,
            g[1],
            b[2] / b[3].max(1e-6) * ag,
            ag,
        ]
    }
}

// ---- glitch ----

pub struct Glitch;

static GLITCH: NodeInfo = NodeInfo {
    id: "effect.glitch",
    title: "Glitch",
    category: NodeCategory::Effect,
    description: "Tears bands of the picture sideways and splits its colours, like a broken signal.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("amount", "Amount", 0.0, 100.0, 40.0),
        ParamSpec::pixels("band", "Band height", 200.0, 12.0),
        ParamSpec::pixels("shift", "Largest shift", 500.0, 30.0),
        ParamSpec::pixels("split", "Colour split", 50.0, 4.0),
        ParamSpec::int("seed", "Seed", 0, 9999, 7),
    ],
};

const GLITCH_KERNEL: Kernel = Kernel {
    name: "glitch",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let b = param(1u);
    let band = i32(floor(f32(level0(p).y) / a.y));
    let h = hash_u32(bitcast<u32>(band) ^ u32(b.y));
    let roll = f32(h & 65535u) / 65536.0;
    var shift = 0.0;
    if (roll < a.x) { shift = (f32((h >> 16u) & 65535u) / 65536.0 - 0.5) * 2.0 * a.z; }
    let pos = vec2<f32>(p) + vec2<f32>(0.5 + shift, 0.5);
    let r = sample0(pos + vec2<f32>(a.w, 0.0), true);
    let g = sample0(pos, true);
    let bl = sample0(pos - vec2<f32>(a.w, 0.0), true);
    let ag = g.a;
    var out = vec4<f32>(r.r / max(r.a, 1e-6) * ag, g.g, bl.b / max(bl.a, 1e-6) * ag, ag);
    if (pixel_noise(level0(p).x / 3, band) < b.x) { out = vec4<f32>(out.rgb * 0.0 + vec3<f32>(ag), ag); }
    return out;
}
"#,
};

impl Simple for Glitch {
    fn info(&self) -> &'static NodeInfo {
        &GLITCH
    }
    fn kernel(&self) -> &'static Kernel {
        &GLITCH_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let amount = p.float("amount") / 100.0;
        if amount == 0.0 {
            return None;
        }
        Some(Setup::new(vec![
            [
                amount,
                p.float("band").max(1.0),
                p.float("shift") * cx.scale(),
                p.float("split") * cx.scale(),
            ],
            [amount * 0.004, p.int("seed") as f32, 0.0, 0.0],
        ]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        let p = cx.params;
        ((p.float("shift") + p.float("split")) * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let (a, b) = (s.p(0), s.p(1));
        let sc = 1i32 << cx.level.min(30);
        let (x0, y0) = (x * sc, y * sc);
        let band = (y0 as f32 / a[1]).floor() as i32;
        let h = hash_u32((band as u32) ^ (b[1] as u32));
        let roll = (h & 65535) as f32 / 65536.0;
        let shift = if roll < a[0] {
            (((h >> 16) & 65535) as f32 / 65536.0 - 0.5) * 2.0 * a[2]
        } else {
            0.0
        };
        let pos = [x as f32 + 0.5 + shift, y as f32 + 0.5];
        let r = sample(input, [pos[0] + a[3], pos[1]], true);
        let g = sample(input, pos, true);
        let bl = sample(input, [pos[0] - a[3], pos[1]], true);
        let ag = g[3];
        if pixel_noise(x0 / 3, band) < b[0] {
            return [ag, ag, ag, ag];
        }
        [
            r[0] / r[3].max(1e-6) * ag,
            g[1],
            bl[2] / bl[3].max(1e-6) * ag,
            ag,
        ]
    }
}
