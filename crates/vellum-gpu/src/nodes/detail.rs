//! sharpen, noise and grain filters: smart sharpen, high pass and reduce noise compare each
//! pixel with a blurred copy (the same two pass shape as shadows and highlights); add noise
//! and film grain draw a stable pattern from level 0 positions so it does not swim when
//! zooming.

use crate::gpu::Kernel;
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};
use crate::point_kernel;
use crate::tone_kernel;

use super::colorfn::{fbm, luma, mix, smoothstep, value_noise};
use super::simple::{Setup, Simple, point};
use super::tone::{Local, LocalTone};
use crate::blend::pixel_noise as blend_noise;

// ---- smart sharpen ----

pub struct SmartSharpen;

static SMART_SHARPEN: NodeInfo = NodeInfo {
    id: "effect.smart_sharpen",
    title: "Smart sharpen",
    category: NodeCategory::Effect,
    description: "Sharpens edges while leaving noise and the deepest shadows and brightest highlights alone.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("amount", "Amount", 0.0, 500.0, 150.0),
        ParamSpec::pixels("radius", "Radius", 64.0, 1.0),
        ParamSpec::percent("reduce_noise", "Reduce noise", 0.0, 100.0, 10.0),
        ParamSpec::percent("shadows_fade", "Shadows fade", 0.0, 100.0, 0.0),
        ParamSpec::percent("highlights_fade", "Highlights fade", 0.0, 100.0, 0.0),
    ],
};

const SMART_SHARPEN_KERNEL: Kernel = tone_kernel!(
    "smart_sharpen",
    r#"
fn tone(c: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let a = param(0u);
    let d = c.rgb - b.rgb;
    let gate = smoothstep(a.y * 0.5, a.y, abs(luma(d)) + 1e-6);
    let y = clamp(luma(c.rgb), 0.0, 1.0);
    let fade = 1.0 - a.z * (1.0 - smoothstep(0.0, 0.3, y)) - a.w * smoothstep(0.7, 1.0, y);
    return vec4<f32>(c.rgb + d * a.x * gate * fade, c.a);
}
"#
);

impl LocalTone for SmartSharpen {
    fn info(&self) -> &'static NodeInfo {
        &SMART_SHARPEN
    }
    fn kernel(&self) -> &'static Kernel {
        &SMART_SHARPEN_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<(f32, Vec<[f32; 4]>)> {
        let p = cx.params;
        let amount = p.float("amount") / 100.0;
        if amount == 0.0 {
            return None;
        }
        Some((
            (p.float("radius") * cx.scale()).max(0.3),
            vec![[
                amount,
                p.float("reduce_noise") / 100.0 * 0.05,
                p.float("shadows_fade") / 100.0,
                p.float("highlights_fade") / 100.0,
            ]],
        ))
    }
    fn pixel(&self, p: &[[f32; 4]], _cx: &NodeCx, c: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        let a = p[0];
        let d = [c[0] - b[0], c[1] - b[1], c[2] - b[2]];
        let gate = smoothstep(a[1] * 0.5, a[1], luma(d).abs() + 1e-6);
        let y = luma([c[0], c[1], c[2]]).clamp(0.0, 1.0);
        let fade = 1.0 - a[2] * (1.0 - smoothstep(0.0, 0.3, y)) - a[3] * smoothstep(0.7, 1.0, y);
        let k = a[0] * gate * fade;
        [c[0] + d[0] * k, c[1] + d[1] * k, c[2] + d[2] * k, c[3]]
    }
}

pub fn smart_sharpen() -> impl NodeKind {
    Local(SmartSharpen)
}

// ---- high pass ----

pub struct HighPass;

static HIGH_PASS: NodeInfo = NodeInfo {
    id: "effect.high_pass",
    title: "High pass",
    category: NodeCategory::Effect,
    description: "Keeps only the fine detail on a middle grey, for sharpening with overlay.",
    inputs: ONE_INPUT,
    params: &[ParamSpec::pixels("radius", "Radius", 1000.0, 10.0)],
};

const HIGH_PASS_KERNEL: Kernel = tone_kernel!(
    "high_pass",
    r#"
fn tone(c: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(vec3<f32>(0.5) + c.rgb - b.rgb, c.a);
}
"#
);

impl LocalTone for HighPass {
    fn info(&self) -> &'static NodeInfo {
        &HIGH_PASS
    }
    fn kernel(&self) -> &'static Kernel {
        &HIGH_PASS_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<(f32, Vec<[f32; 4]>)> {
        Some((
            (cx.params.float("radius") * cx.scale()).max(0.3),
            Vec::new(),
        ))
    }
    fn pixel(&self, _p: &[[f32; 4]], _cx: &NodeCx, c: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        [
            0.5 + c[0] - b[0],
            0.5 + c[1] - b[1],
            0.5 + c[2] - b[2],
            c[3],
        ]
    }
}

pub fn high_pass() -> impl NodeKind {
    Local(HighPass)
}

// ---- reduce noise ----

pub struct ReduceNoise;

static REDUCE_NOISE: NodeInfo = NodeInfo {
    id: "effect.reduce_noise",
    title: "Reduce noise",
    category: NodeCategory::Effect,
    description: "Smooths grain and colour speckles while keeping edges.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::float("strength", "Strength", 0.0, 10.0, 6.0),
        ParamSpec::percent("preserve", "Preserve details", 0.0, 100.0, 60.0),
        ParamSpec::percent("color", "Reduce color noise", 0.0, 100.0, 45.0),
    ],
};

const REDUCE_NOISE_KERNEL: Kernel = tone_kernel!(
    "reduce_noise",
    r#"
fn tone(c: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let a = param(0u);
    let y = luma(c.rgb);
    let yb = luma(b.rgb);
    let keep = smoothstep(0.0, mix(0.4, 0.02, a.y), abs(y - yb));
    let ny = mix(y, yb, a.x * (1.0 - keep));
    let chroma = mix(c.rgb - vec3<f32>(y), b.rgb - vec3<f32>(yb), a.z);
    return vec4<f32>(vec3<f32>(ny) + chroma, c.a);
}
"#
);

impl LocalTone for ReduceNoise {
    fn info(&self) -> &'static NodeInfo {
        &REDUCE_NOISE
    }
    fn kernel(&self) -> &'static Kernel {
        &REDUCE_NOISE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<(f32, Vec<[f32; 4]>)> {
        let p = cx.params;
        let s = p.float("strength") / 10.0;
        let col = p.float("color") / 100.0;
        if s == 0.0 && col == 0.0 {
            return None;
        }
        Some((
            (1.5 * cx.scale()).max(0.3),
            vec![[s, p.float("preserve") / 100.0, col, 0.0]],
        ))
    }
    fn pixel(&self, p: &[[f32; 4]], _cx: &NodeCx, c: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        let a = p[0];
        let y = luma([c[0], c[1], c[2]]);
        let yb = luma([b[0], b[1], b[2]]);
        let keep = smoothstep(0.0, mix(0.4, 0.02, a[1]), (y - yb).abs());
        let ny = mix(y, yb, a[0] * (1.0 - keep));
        let ch = |k: usize| ny + mix(c[k] - y, b[k] - yb, a[2]);
        [ch(0), ch(1), ch(2), c[3]]
    }
}

pub fn reduce_noise() -> impl NodeKind {
    Local(ReduceNoise)
}

// ---- add noise ----

pub struct AddNoise;

const DISTRIBUTIONS: &[ChoiceOption] = &[
    ChoiceOption {
        id: "uniform",
        label: "Uniform",
    },
    ChoiceOption {
        id: "gaussian",
        label: "Gaussian",
    },
];

static ADD_NOISE: NodeInfo = NodeInfo {
    id: "effect.add_noise",
    title: "Add noise",
    category: NodeCategory::Effect,
    description: "Sprinkles random speckles over the picture.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("amount", "Amount", 0.0, 400.0, 12.5),
        ParamSpec::choice("distribution", "Distribution", DISTRIBUTIONS, "uniform"),
        ParamSpec::boolean("monochromatic", "Monochromatic", false),
        ParamSpec::int("seed", "Seed", 0, 9999, 1),
    ],
};

const ADD_NOISE_KERNEL: Kernel = point_kernel!(
    "add_noise",
    r#"
fn noise1(q: vec2<i32>, k: i32, gauss: bool) -> f32 {
    let s = i32(param(0u).w) * 7919 + k * 104729;
    if (gauss) {
        return (pixel_noise(q.x + s, q.y) + pixel_noise(q.x + s + 31, q.y + 17) + pixel_noise(q.x + s + 67, q.y + 43)) / 3.0 - 0.5;
    }
    return pixel_noise(q.x + s, q.y) - 0.5;
}
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let q = level0(p);
    let gauss = a.y > 0.5;
    var n = vec3<f32>(noise1(q, 0, gauss), noise1(q, 1, gauss), noise1(q, 2, gauss));
    if (a.z > 0.5) { n = vec3<f32>(n.x); }
    return vec4<f32>(c.rgb + n * a.x, c.a);
}
"#
);

fn noise1(q: [i32; 2], seed: i32, k: i32, gauss: bool) -> f32 {
    let s = seed
        .wrapping_mul(7919)
        .wrapping_add(k.wrapping_mul(104_729));
    let n = |dx: i32, dy: i32| blend_noise(q[0].wrapping_add(s).wrapping_add(dx), q[1] + dy);
    if gauss {
        (n(0, 0) + n(31, 17) + n(67, 43)) / 3.0 - 0.5
    } else {
        n(0, 0) - 0.5
    }
}

impl Simple for AddNoise {
    fn info(&self) -> &'static NodeInfo {
        &ADD_NOISE
    }
    fn kernel(&self) -> &'static Kernel {
        &ADD_NOISE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let amount = p.float("amount") / 100.0;
        if amount == 0.0 {
            return None;
        }
        let gauss = p.choice_index("distribution") == 1;
        Some(Setup::new(vec![[
            amount * if gauss { 2.0 } else { 1.0 },
            f32::from(u8::from(gauss)),
            f32::from(u8::from(p.bool("monochromatic"))),
            p.int("seed") as f32,
        ]]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let sc = 1i32 << cx.level.min(30);
        let q = [x * sc, y * sc];
        let gauss = a[1] > 0.5;
        let seed = a[3] as i32;
        point(cx, input.get(x, y), |c| {
            let mut n = [0, 1, 2].map(|k| noise1(q, seed, k, gauss));
            if a[2] > 0.5 {
                n = [n[0]; 3];
            }
            [
                c[0] + n[0] * a[0],
                c[1] + n[1] * a[0],
                c[2] + n[2] * a[0],
                c[3],
            ]
        })
    }
}

// ---- film grain ----

pub struct FilmGrain;

static FILM_GRAIN: NodeInfo = NodeInfo {
    id: "effect.film_grain",
    title: "Film grain",
    category: NodeCategory::Effect,
    description: "Adds soft grain like film, strongest in the midtones.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("amount", "Amount", 0.0, 100.0, 30.0),
        ParamSpec::pixels("size", "Grain size", 20.0, 1.5),
        ParamSpec::percent("roughness", "Roughness", 0.0, 100.0, 50.0),
        ParamSpec::percent("highlights", "Highlight area", 0.0, 100.0, 30.0),
    ],
};

const FILM_GRAIN_KERNEL: Kernel = point_kernel!(
    "film_grain",
    r#"
fn adjust(c: vec4<f32>, p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let q = vec2<f32>(level0(p)) / a.y;
    let n = mix(value_noise(q.x, q.y, 5u), fbm(q.x, q.y, 3u, 5u), a.z) - 0.5;
    let y = clamp(luma(c.rgb), 0.0, 1.0);
    let w = 4.0 * y * (1.0 - y) + a.w * smoothstep(0.6, 1.0, y);
    return vec4<f32>(c.rgb + vec3<f32>(n * a.x * w), c.a);
}
"#
);

impl Simple for FilmGrain {
    fn info(&self) -> &'static NodeInfo {
        &FILM_GRAIN
    }
    fn kernel(&self) -> &'static Kernel {
        &FILM_GRAIN_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let amount = p.float("amount") / 100.0;
        if amount == 0.0 {
            return None;
        }
        Some(Setup::new(vec![[
            amount * 0.4,
            p.float("size").max(0.5),
            p.float("roughness") / 100.0,
            p.float("highlights") / 100.0,
        ]]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let sc = (1i32 << cx.level.min(30)) as f32;
        let q = [x as f32 * sc / a[1], y as f32 * sc / a[1]];
        point(cx, input.get(x, y), |c| {
            let n = mix(value_noise(q[0], q[1], 5), fbm(q[0], q[1], 3, 5), a[2]) - 0.5;
            let yy = luma([c[0], c[1], c[2]]).clamp(0.0, 1.0);
            let w = 4.0 * yy * (1.0 - yy) + a[3] * smoothstep(0.6, 1.0, yy);
            let add = n * a[0] * w;
            [c[0] + add, c[1] + add, c[2] + add, c[3]]
        })
    }
}
