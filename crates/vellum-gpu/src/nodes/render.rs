//! render filters: clouds, lens flare and lighting effects. clouds paints new pixels
//! everywhere; the other two light up what is already there.

use crate::gpu::Kernel;
use crate::graph::{NodeCategory, NodeCx, NodeInfo, ONE_INPUT};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};

use super::colorfn::{fbm, luma, mix, smoothstep, working_color};
use super::simple::{Setup, Simple};
use super::{premultiply, unpremultiply};

// ---- clouds ----

pub struct Clouds;

static CLOUDS: NodeInfo = NodeInfo {
    id: "effect.clouds",
    title: "Clouds",
    category: NodeCategory::Effect,
    description: "Fills the layer with soft clouds between two colours.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::color("foreground", "Foreground", [1.0, 1.0, 1.0, 1.0]),
        ParamSpec::color("background", "Background", [0.0, 0.0, 0.0, 1.0]),
        ParamSpec::pixels("scale", "Scale", 4000.0, 128.0),
        ParamSpec::int("seed", "Seed", 0, 9999, 1),
    ],
};

const CLOUDS_KERNEL: Kernel = Kernel {
    name: "clouds",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let q = (vec2<f32>(level0(p)) + vec2<f32>(0.5)) / a.x;
    let n = fbm(q.x, q.y, 6u, u32(a.y));
    let c = mix(param(2u), param(1u), n);
    return premultiply(c);
}
"#,
};

impl Simple for Clouds {
    fn info(&self) -> &'static NodeInfo {
        &CLOUDS
    }
    fn kernel(&self) -> &'static Kernel {
        &CLOUDS_KERNEL
    }
    fn generates(&self) -> bool {
        true
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let (f, b) = (p.color("foreground"), p.color("background"));
        let (fw, bw) = (working_color(cx, f), working_color(cx, b));
        Some(Setup::new(vec![
            [p.float("scale").max(1.0), p.int("seed") as f32, 0.0, 0.0],
            [fw[0], fw[1], fw[2], f[3]],
            [bw[0], bw[1], bw[2], b[3]],
        ]))
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, _input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let sc = (1i32 << cx.level.min(30)) as f32;
        let q = [(x as f32 * sc + 0.5) / a[0], (y as f32 * sc + 0.5) / a[0]];
        let n = fbm(q[0], q[1], 6, a[1] as u32);
        let (f, b) = (s.p(1), s.p(2));
        premultiply([0, 1, 2, 3].map(|k| mix(b[k], f[k], n)))
    }
}

// ---- lens flare ----

pub struct LensFlare;

static LENS_FLARE: NodeInfo = NodeInfo {
    id: "effect.lens_flare",
    title: "Lens flare",
    category: NodeCategory::Effect,
    description: "Adds the glow, ring and ghosts of a bright light shining into the lens.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::point("position", "Flare centre", [0.0, 0.0]),
        ParamSpec::percent("brightness", "Brightness", 10.0, 300.0, 100.0),
        ParamSpec::pixels("size", "Size", 2000.0, 80.0),
        ParamSpec::point("center", "Picture centre", [0.0, 0.0]).hide(),
    ],
};

const LENS_FLARE_KERNEL: Kernel = Kernel {
    name: "lens_flare",
    body: r#"
fn flare(pos: vec2<f32>) -> vec3<f32> {
    let a = param(0u);
    let b = param(1u);
    let d = length(pos - a.xy) / a.w;
    var f = vec3<f32>(1.0, 0.95, 0.85) * exp(-d * d * 4.0) * 1.2;
    f = f + vec3<f32>(1.0, 0.8, 0.6) * exp(-d * 1.5) * 0.25;
    f = f + vec3<f32>(0.6, 0.8, 1.0) * (1.0 - smoothstep(0.0, 0.08, abs(d - 1.6))) * 0.25;
    let axis = b.xy - a.xy;
    for (var i = 1; i <= 4; i = i + 1) {
        let t = 0.4 + f32(i) * 0.35;
        let g = a.xy + axis * t;
        let r = a.w * (0.15 + 0.1 * f32(i % 3));
        let gd = length(pos - g) / r;
        let tint = vec3<f32>(0.4 + 0.15 * f32(i), 0.7, 1.0 - 0.15 * f32(i));
        f = f + tint * (1.0 - smoothstep(0.7, 1.0, gd)) * 0.12;
    }
    return f * a.z;
}
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = load0(p);
    let f = clamp01(flare(vec2<f32>(p) + vec2<f32>(0.5)));
    let rgb = c.rgb + f * (vec3<f32>(1.0) - c.rgb);
    let a = c.a + luma(f) * (1.0 - c.a);
    return vec4<f32>(min(rgb, vec3<f32>(a)), a);
}
"#,
};

fn flare(s: &Setup, pos: [f32; 2]) -> [f32; 3] {
    let (a, b) = (s.p(0), s.p(1));
    let d = (pos[0] - a[0]).hypot(pos[1] - a[1]) / a[3];
    let mut f = [1.0, 0.95, 0.85].map(|v| v * (-d * d * 4.0).exp() * 1.2);
    let glow = (-d * 1.5).exp() * 0.25;
    let ring = (1.0 - smoothstep(0.0, 0.08, (d - 1.6).abs())) * 0.25;
    for (k, v) in f.iter_mut().enumerate() {
        *v += [1.0, 0.8, 0.6][k] * glow + [0.6, 0.8, 1.0][k] * ring;
    }
    let axis = [b[0] - a[0], b[1] - a[1]];
    for i in 1..=4 {
        let t = 0.4 + i as f32 * 0.35;
        let g = [a[0] + axis[0] * t, a[1] + axis[1] * t];
        let r = a[3] * (0.15 + 0.1 * (i % 3) as f32);
        let gd = (pos[0] - g[0]).hypot(pos[1] - g[1]) / r;
        let tint = [0.4 + 0.15 * i as f32, 0.7, 1.0 - 0.15 * i as f32];
        let w = (1.0 - smoothstep(0.7, 1.0, gd)) * 0.12;
        for k in 0..3 {
            f[k] += tint[k] * w;
        }
    }
    f.map(|v| v * a[2])
}

impl Simple for LensFlare {
    fn info(&self) -> &'static NodeInfo {
        &LENS_FLARE
    }
    fn kernel(&self) -> &'static Kernel {
        &LENS_FLARE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let s = cx.scale();
        let (f, c) = (p.point("position"), p.point("center"));
        Some(Setup::new(vec![
            [
                f[0] * s,
                f[1] * s,
                p.float("brightness") / 100.0,
                (p.float("size") * s).max(1.0),
            ],
            [c[0] * s, c[1] * s, 0.0, 0.0],
        ]))
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let c = input.get(x, y);
        let f = flare(s, [x as f32 + 0.5, y as f32 + 0.5]).map(|v| v.clamp(0.0, 1.0));
        let a = c[3] + luma(f) * (1.0 - c[3]);
        let rgb = [0, 1, 2].map(|k| (c[k] + f[k] * (1.0 - c[k])).min(a));
        [rgb[0], rgb[1], rgb[2], a]
    }
}

// ---- lighting effects ----

pub struct Lighting;

const LIGHT_KINDS: &[ChoiceOption] = &[
    ChoiceOption {
        id: "spot",
        label: "Spot",
    },
    ChoiceOption {
        id: "point",
        label: "Point",
    },
    ChoiceOption {
        id: "infinite",
        label: "Infinite",
    },
];

static LIGHTING: NodeInfo = NodeInfo {
    id: "effect.lighting",
    title: "Lighting effects",
    category: NodeCategory::Effect,
    description: "Lights the picture with a lamp, using its brightness as a bumpy surface.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("kind", "Light", LIGHT_KINDS, "spot"),
        ParamSpec::point("position", "Light position", [0.0, 0.0]),
        ParamSpec::degrees("angle", "Direction", -180.0, 180.0, -45.0),
        ParamSpec::percent("intensity", "Intensity", 0.0, 200.0, 100.0),
        ParamSpec::color("color", "Light colour", [1.0, 1.0, 1.0, 1.0]),
        ParamSpec::percent("ambience", "Ambience", 0.0, 100.0, 20.0),
        ParamSpec::pixels("reach", "Reach", 10000.0, 300.0),
        ParamSpec::float("height", "Texture height", 0.0, 100.0, 10.0),
        ParamSpec::percent("cone", "Spot width", 5.0, 100.0, 50.0),
    ],
};

const LIGHTING_KERNEL: Kernel = Kernel {
    name: "lighting",
    body: r#"
fn hl(p: vec2<i32>) -> f32 {
    return luma(unpremultiply(load0_clamped(p)).rgb);
}
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u); let b = param(1u); let c4 = param(2u); let d = param(3u);
    let src = unpremultiply(load0(p));
    if (src.a <= 0.0) { return vec4<f32>(0.0); }
    let nx = (hl(p - vec2<i32>(1, 0)) - hl(p + vec2<i32>(1, 0))) * d.x;
    let ny = (hl(p - vec2<i32>(0, 1)) - hl(p + vec2<i32>(0, 1))) * d.x;
    let n = normalize(vec3<f32>(nx, ny, 1.0));
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    var l: vec3<f32>;
    var fall = 1.0;
    if (a.w > 1.5) {
        l = normalize(vec3<f32>(b.x, b.y, 1.0));
    } else {
        let toward = vec3<f32>(a.xy - pos, a.z * 0.5);
        l = normalize(toward);
        fall = 1.0 - smoothstep(0.0, a.z * 2.0, length(a.xy - pos));
        if (a.w < 0.5) {
            let along = dot(normalize(pos - a.xy + vec2<f32>(1e-4)), b.xy);
            fall = fall * smoothstep(b.z, b.z + 0.2, along);
        }
    }
    let diffuse = max(dot(n, l), 0.0) * fall * b.w;
    let rgb = src.rgb * (vec3<f32>(d.y) + c4.rgb * diffuse);
    return premultiply(vec4<f32>(rgb, src.a));
}
"#,
};

impl Simple for Lighting {
    fn info(&self) -> &'static NodeInfo {
        &LIGHTING
    }
    fn kernel(&self) -> &'static Kernel {
        &LIGHTING_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let s = cx.scale();
        let pos = p.point("position");
        let (sn, cs) = p.float("angle").to_radians().sin_cos();
        let col = working_color(cx, p.color("color"));
        let cone = p.float("cone") / 100.0;
        Some(Setup::new(vec![
            [
                pos[0] * s,
                pos[1] * s,
                (p.float("reach") * s).max(1.0),
                p.choice_index("kind") as f32,
            ],
            [cs, sn, 1.0 - cone * 1.2, p.float("intensity") / 100.0 * 1.3],
            [col[0], col[1], col[2], 0.0],
            [
                p.float("height") * 0.5,
                p.float("ambience") / 100.0,
                0.0,
                0.0,
            ],
        ]))
    }
    fn reach(&self, _cx: &NodeCx) -> i32 {
        1
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let (a, b, c4, d) = (s.p(0), s.p(1), s.p(2), s.p(3));
        let src = unpremultiply(input.get(x, y));
        if src[3] <= 0.0 {
            return [0.0; 4];
        }
        let hl = |x: i32, y: i32| {
            let c = unpremultiply(input.get_clamped(x, y));
            luma([c[0], c[1], c[2]])
        };
        let nx = (hl(x - 1, y) - hl(x + 1, y)) * d[0];
        let ny = (hl(x, y - 1) - hl(x, y + 1)) * d[0];
        let n = normalize([nx, ny, 1.0]);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let (l, fall) = if a[3] > 1.5 {
            (normalize([b[0], b[1], 1.0]), 1.0)
        } else {
            let toward = [a[0] - pos[0], a[1] - pos[1], a[2] * 0.5];
            let mut fall = 1.0 - smoothstep(0.0, a[2] * 2.0, toward[0].hypot(toward[1]));
            if a[3] < 0.5 {
                let dir = [pos[0] - a[0] + 1e-4, pos[1] - a[1] + 1e-4];
                let len = dir[0].hypot(dir[1]).max(1e-12);
                let along = (dir[0] * b[0] + dir[1] * b[1]) / len;
                fall *= smoothstep(b[2], b[2] + 0.2, along);
            }
            (normalize(toward), fall)
        };
        let diffuse = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0) * fall * b[3];
        let rgb = [0, 1, 2].map(|k| src[k] * (d[1] + c4[k] * diffuse));
        premultiply([rgb[0], rgb[1], rgb[2], src[3]])
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    v.map(|c| c / l)
}
