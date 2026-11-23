//! stylize and pixelate filters: emboss, find edges, glowing edges, oil paint, mosaic and
//! halftone.

use crate::gpu::Kernel;
use crate::graph::{NodeCategory, NodeCx, NodeInfo, ONE_INPUT};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};

use super::colorfn::{luma, sample};
use super::simple::{Setup, Simple};
use super::{premultiply, unpremultiply};

fn lum_at(img: &CpuImage, pos: [f32; 2]) -> f32 {
    let c = unpremultiply(sample(img, pos, true));
    luma([c[0], c[1], c[2]])
}

// ---- emboss ----

pub struct Emboss;

static EMBOSS: NodeInfo = NodeInfo {
    id: "effect.emboss",
    title: "Emboss",
    category: NodeCategory::Effect,
    description: "Makes the picture look pressed into grey metal.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::degrees("angle", "Angle", -180.0, 180.0, 135.0),
        ParamSpec::pixels("height", "Height", 10.0, 3.0),
        ParamSpec::percent("amount", "Amount", 1.0, 500.0, 100.0),
    ],
};

const EMBOSS_KERNEL: Kernel = Kernel {
    name: "emboss",
    body: r#"
fn lum0(pos: vec2<f32>) -> f32 {
    return luma(unpremultiply(sample0(pos, true)).rgb);
}
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let c = load0(p);
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let d = lum0(pos + a.xy) - lum0(pos - a.xy);
    let v = clamp(0.5 + d * a.z, 0.0, 1.0);
    return vec4<f32>(v * c.a, v * c.a, v * c.a, c.a);
}
"#,
};

impl Simple for Emboss {
    fn info(&self) -> &'static NodeInfo {
        &EMBOSS
    }
    fn kernel(&self) -> &'static Kernel {
        &EMBOSS_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let (s, c) = p.float("angle").to_radians().sin_cos();
        let h = (p.float("height") * cx.scale()).max(0.5);
        Some(Setup::new(vec![[
            c * h,
            -s * h,
            p.float("amount") / 100.0 * 2.0,
            0.0,
        ]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("height") * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let c = input.get(x, y);
        if c[3] <= 0.0 {
            return [0.0; 4];
        }
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let d = lum_at(input, [pos[0] + a[0], pos[1] + a[1]])
            - lum_at(input, [pos[0] - a[0], pos[1] - a[1]]);
        let v = (0.5 + d * a[2]).clamp(0.0, 1.0);
        [v * c[3], v * c[3], v * c[3], c[3]]
    }
}

// ---- find edges ----

pub struct FindEdges;

static FIND_EDGES: NodeInfo = NodeInfo {
    id: "effect.find_edges",
    title: "Find edges",
    category: NodeCategory::Effect,
    description: "Draws the edges in colour on white, like a pencil outline.",
    inputs: ONE_INPUT,
    params: &[ParamSpec::percent(
        "strength", "Strength", 10.0, 400.0, 100.0,
    )],
};

const FIND_EDGES_KERNEL: Kernel = Kernel {
    name: "find_edges",
    body: r#"
fn st(p: vec2<i32>, dx: i32, dy: i32) -> vec3<f32> {
    return unpremultiply(load0_clamped(p + vec2<i32>(dx, dy))).rgb;
}
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = load0(p);
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    let gx = st(p, 1, -1) + 2.0 * st(p, 1, 0) + st(p, 1, 1) - st(p, -1, -1) - 2.0 * st(p, -1, 0) - st(p, -1, 1);
    let gy = st(p, -1, 1) + 2.0 * st(p, 0, 1) + st(p, 1, 1) - st(p, -1, -1) - 2.0 * st(p, 0, -1) - st(p, 1, -1);
    let g = sqrt(gx * gx + gy * gy) * param(0u).x;
    return premultiply(vec4<f32>(clamp01(vec3<f32>(1.0) - g), c.a));
}
"#,
};

/// sobel magnitude per channel at a pixel, edges repeated
fn sobel(input: &CpuImage, x: i32, y: i32) -> [f32; 3] {
    let st = |dx: i32, dy: i32| unpremultiply(input.get_clamped(x + dx, y + dy));
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let gx = st(1, -1)[k] + 2.0 * st(1, 0)[k] + st(1, 1)[k]
            - st(-1, -1)[k]
            - 2.0 * st(-1, 0)[k]
            - st(-1, 1)[k];
        let gy = st(-1, 1)[k] + 2.0 * st(0, 1)[k] + st(1, 1)[k]
            - st(-1, -1)[k]
            - 2.0 * st(0, -1)[k]
            - st(1, -1)[k];
        *o = (gx * gx + gy * gy).sqrt();
    }
    out
}

impl Simple for FindEdges {
    fn info(&self) -> &'static NodeInfo {
        &FIND_EDGES
    }
    fn kernel(&self) -> &'static Kernel {
        &FIND_EDGES_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        Some(Setup::new(vec![[
            cx.params.float("strength") / 100.0 * 0.5,
            0.0,
            0.0,
            0.0,
        ]]))
    }
    fn reach(&self, _cx: &NodeCx) -> i32 {
        1
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let c = input.get(x, y);
        if c[3] <= 0.0 {
            return [0.0; 4];
        }
        let g = sobel(input, x, y);
        let k = s.p(0)[0];
        premultiply([
            (1.0 - g[0] * k).clamp(0.0, 1.0),
            (1.0 - g[1] * k).clamp(0.0, 1.0),
            (1.0 - g[2] * k).clamp(0.0, 1.0),
            c[3],
        ])
    }
}

// ---- glowing edges ----

pub struct GlowingEdges;

static GLOWING_EDGES: NodeInfo = NodeInfo {
    id: "effect.glowing_edges",
    title: "Glowing edges",
    category: NodeCategory::Effect,
    description: "Lights up the edges like neon on black.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("width", "Edge width", 14.0, 2.0),
        ParamSpec::float("brightness", "Edge brightness", 0.0, 20.0, 6.0),
    ],
};

const GLOWING_KERNEL: Kernel = Kernel {
    name: "glowing_edges",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let c = load0(p);
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let w = a.x;
    let gx = unpremultiply(sample0(pos + vec2<f32>(w, 0.0), true)).rgb - unpremultiply(sample0(pos - vec2<f32>(w, 0.0), true)).rgb;
    let gy = unpremultiply(sample0(pos + vec2<f32>(0.0, w), true)).rgb - unpremultiply(sample0(pos - vec2<f32>(0.0, w), true)).rgb;
    let g = clamp01(sqrt(gx * gx + gy * gy) * a.y);
    return premultiply(vec4<f32>(g, c.a));
}
"#,
};

impl Simple for GlowingEdges {
    fn info(&self) -> &'static NodeInfo {
        &GLOWING_EDGES
    }
    fn kernel(&self) -> &'static Kernel {
        &GLOWING_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        Some(Setup::new(vec![[
            (p.float("width") * cx.scale()).max(0.5),
            p.float("brightness") / 2.0,
            0.0,
            0.0,
        ]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("width") * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let c = input.get(x, y);
        if c[3] <= 0.0 {
            return [0.0; 4];
        }
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let at = |dx: f32, dy: f32| unpremultiply(sample(input, [pos[0] + dx, pos[1] + dy], true));
        let w = a[0];
        let (px, nx, py, ny) = (at(w, 0.0), at(-w, 0.0), at(0.0, w), at(0.0, -w));
        let g = [0, 1, 2].map(|k| {
            let gx = px[k] - nx[k];
            let gy = py[k] - ny[k];
            ((gx * gx + gy * gy).sqrt() * a[1]).clamp(0.0, 1.0)
        });
        premultiply([g[0], g[1], g[2], c[3]])
    }
}

// ---- oil paint ----

pub struct OilPaint;

static OIL_PAINT: NodeInfo = NodeInfo {
    id: "effect.oil_paint",
    title: "Oil paint",
    category: NodeCategory::Effect,
    description: "Turns the picture into dabs of flat colour, like a painting.",
    inputs: ONE_INPUT,
    params: &[ParamSpec::pixels("radius", "Brush size", 50.0, 4.0)],
};

/// samples per side of each quadrant
const OIL_STEPS: i32 = 4;

/// a kuwahara filter: the means of the four quadrants around the pixel, weighted towards
/// the ones that vary least (papari's smooth version, which does not flicker on ties)
const OIL_KERNEL: Kernel = Kernel {
    name: "oil_paint",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var q = 0; q < 4; q = q + 1) {
        let sx = select(-1.0, 1.0, (q & 1) == 1);
        let sy = select(-1.0, 1.0, (q & 2) == 2);
        var sum = vec4<f32>(0.0);
        var sq = 0.0;
        for (var j = 0; j < 4; j = j + 1) {
            for (var i = 0; i < 4; i = i + 1) {
                let c = sample0(pos + vec2<f32>(sx * f32(i), sy * f32(j)) * a.y, true);
                sum = sum + c;
                let l = luma(c.rgb);
                sq = sq + l * l;
            }
        }
        let mean = sum / 16.0;
        let l = luma(mean.rgb);
        let v = max(sq / 16.0 - l * l, 0.0);
        let w = 1.0 / ((v * 200.0 + 0.01) * (v * 200.0 + 0.01));
        acc = acc + mean * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}
"#,
};

impl Simple for OilPaint {
    fn info(&self) -> &'static NodeInfo {
        &OIL_PAINT
    }
    fn kernel(&self) -> &'static Kernel {
        &OIL_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let r = cx.params.float("radius") * cx.scale();
        if r < 0.5 {
            return None;
        }
        Some(Setup::new(vec![[r, r / (OIL_STEPS - 1) as f32, 0.0, 0.0]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("radius") * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let (mut acc, mut wsum) = ([0.0f32; 4], 0.0f32);
        for q in 0..4 {
            let sx = if q & 1 == 1 { 1.0 } else { -1.0 };
            let sy = if q & 2 == 2 { 1.0 } else { -1.0 };
            let (mut sum, mut sq) = ([0.0f32; 4], 0.0f32);
            for j in 0..OIL_STEPS {
                for i in 0..OIL_STEPS {
                    let c = sample(
                        input,
                        [pos[0] + sx * i as f32 * a[1], pos[1] + sy * j as f32 * a[1]],
                        true,
                    );
                    for k in 0..4 {
                        sum[k] += c[k];
                    }
                    let l = luma([c[0], c[1], c[2]]);
                    sq += l * l;
                }
            }
            let mean = sum.map(|v| v / 16.0);
            let l = luma([mean[0], mean[1], mean[2]]);
            let v = (sq / 16.0 - l * l).max(0.0);
            let w = 1.0 / ((v * 200.0 + 0.01) * (v * 200.0 + 0.01));
            for k in 0..4 {
                acc[k] += mean[k] * w;
            }
            wsum += w;
        }
        acc.map(|v| v / wsum)
    }
}

// ---- mosaic ----

pub struct Mosaic;

static MOSAIC: NodeInfo = NodeInfo {
    id: "effect.mosaic",
    title: "Pixelate",
    category: NodeCategory::Effect,
    description: "Turns the picture into big square blocks of colour.",
    inputs: ONE_INPUT,
    params: &[ParamSpec::pixels("cell", "Cell size", 200.0, 10.0)],
};

/// each block's colour is the mean of 4x4 samples spread over it, so the cost does not
/// grow with the block size. blocks line up with document pixels at every zoom level.
const MOSAIC_KERNEL: Kernel = Kernel {
    name: "mosaic",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let cell = floor(pos / a.x);
    let origin = cell * a.x;
    var acc = vec4<f32>(0.0);
    for (var j = 0; j < 4; j = j + 1) {
        for (var i = 0; i < 4; i = i + 1) {
            acc = acc + sample0(origin + (vec2<f32>(f32(i), f32(j)) + vec2<f32>(0.5)) * a.x * 0.25, true);
        }
    }
    return acc / 16.0;
}
"#,
};

impl Simple for Mosaic {
    fn info(&self) -> &'static NodeInfo {
        &MOSAIC
    }
    fn kernel(&self) -> &'static Kernel {
        &MOSAIC_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let cell = cx.params.float("cell") * cx.scale();
        if cell < 1.0 {
            return None;
        }
        Some(Setup::new(vec![[cell, 0.0, 0.0, 0.0]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("cell") * cx.scale()).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let cell = s.p(0)[0];
        let ox = ((x as f32 + 0.5) / cell).floor() * cell;
        let oy = ((y as f32 + 0.5) / cell).floor() * cell;
        let mut acc = [0.0; 4];
        for j in 0..4 {
            for i in 0..4 {
                let c = sample(
                    input,
                    [
                        ox + (i as f32 + 0.5) * cell * 0.25,
                        oy + (j as f32 + 0.5) * cell * 0.25,
                    ],
                    true,
                );
                for k in 0..4 {
                    acc[k] += c[k];
                }
            }
        }
        acc.map(|v| v / 16.0)
    }
}

// ---- halftone ----

pub struct Halftone;

const HALFTONE_MODES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "color",
        label: "Colour",
    },
    ChoiceOption {
        id: "mono",
        label: "Black and white",
    },
];

static HALFTONE: NodeInfo = NodeInfo {
    id: "effect.halftone",
    title: "Color halftone",
    category: NodeCategory::Effect,
    description: "Prints the picture as dots of ink, like a newspaper.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("cell", "Dot spacing", 100.0, 8.0),
        ParamSpec::degrees("angle", "Screen angle", -90.0, 90.0, 45.0),
        ParamSpec::choice("mode", "Ink", HALFTONE_MODES, "color"),
    ],
};

/// one ink's coverage at a position: the dot of the screen cell it falls in, sized by the
/// ink the picture needs at the cell's centre
const HALFTONE_KERNEL: Kernel = Kernel {
    name: "halftone",
    body: r#"
fn dot1(pos: vec2<f32>, angle: f32, ch: i32) -> f32 {
    let cell = param(0u).x;
    let cs = cos(angle); let sn = sin(angle);
    let r = vec2<f32>(pos.x * cs + pos.y * sn, -pos.x * sn + pos.y * cs);
    let base = floor(r / cell - vec2<f32>(0.5));
    var best = 0.0;
    for (var j = 0; j < 2; j = j + 1) {
        for (var i = 0; i < 2; i = i + 1) {
            let centre_r = (base + vec2<f32>(f32(i), f32(j)) + vec2<f32>(0.5)) * cell;
            let centre = vec2<f32>(centre_r.x * cs - centre_r.y * sn, centre_r.x * sn + centre_r.y * cs);
            let c = unpremultiply(sample0(centre, true));
            var ink: f32;
            if (ch < 0) { ink = 1.0 - luma(c.rgb); } else { ink = 1.0 - c[ch]; }
            let radius = sqrt(clamp(ink, 0.0, 1.0)) * cell * 0.7071;
            let d = length(r - centre_r);
            best = max(best, clamp(radius - d + 0.5, 0.0, 1.0));
        }
    }
    return best;
}
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let c = load0(p);
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    var rgb: vec3<f32>;
    if (a.z > 0.5) {
        rgb = vec3<f32>(1.0 - dot1(pos, a.y, -1));
    } else {
        rgb = vec3<f32>(1.0 - dot1(pos, a.y + 0.2617994, 0), 1.0 - dot1(pos, a.y + 1.3089969, 1), 1.0 - dot1(pos, a.y, 2));
    }
    return premultiply(vec4<f32>(rgb, c.a));
}
"#,
};

fn dot1(input: &CpuImage, cell: f32, pos: [f32; 2], angle: f32, ch: i32) -> f32 {
    let (sn, cs) = angle.sin_cos();
    let r = [pos[0] * cs + pos[1] * sn, -pos[0] * sn + pos[1] * cs];
    let base = [(r[0] / cell - 0.5).floor(), (r[1] / cell - 0.5).floor()];
    let mut best = 0.0f32;
    for j in 0..2 {
        for i in 0..2 {
            let cr = [
                (base[0] + i as f32 + 0.5) * cell,
                (base[1] + j as f32 + 0.5) * cell,
            ];
            let centre = [cr[0] * cs - cr[1] * sn, cr[0] * sn + cr[1] * cs];
            let c = unpremultiply(sample(input, centre, true));
            let ink = if ch < 0 {
                1.0 - luma([c[0], c[1], c[2]])
            } else {
                1.0 - c[ch as usize]
            };
            let radius = ink.clamp(0.0, 1.0).sqrt() * cell * std::f32::consts::FRAC_1_SQRT_2;
            let d = (r[0] - cr[0]).hypot(r[1] - cr[1]);
            best = best.max((radius - d + 0.5).clamp(0.0, 1.0));
        }
    }
    best
}

impl Simple for Halftone {
    fn info(&self) -> &'static NodeInfo {
        &HALFTONE
    }
    fn kernel(&self) -> &'static Kernel {
        &HALFTONE_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let p = cx.params;
        let cell = p.float("cell") * cx.scale();
        if cell < 1.0 {
            return None;
        }
        Some(Setup::new(vec![[
            cell,
            p.float("angle").to_radians(),
            p.choice_index("mode") as f32,
            0.0,
        ]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("cell") * cx.scale() * 1.5).ceil() as i32 + 2
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let c = input.get(x, y);
        if c[3] <= 0.0 {
            return [0.0; 4];
        }
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let rgb = if a[2] > 0.5 {
            [1.0 - dot1(input, a[0], pos, a[1], -1); 3]
        } else {
            [
                1.0 - dot1(input, a[0], pos, a[1] + 15f32.to_radians(), 0),
                1.0 - dot1(input, a[0], pos, a[1] + 75f32.to_radians(), 1),
                1.0 - dot1(input, a[0], pos, a[1], 2),
            ]
        };
        premultiply([rgb[0], rgb[1], rgb[2], c[3]])
    }
}
