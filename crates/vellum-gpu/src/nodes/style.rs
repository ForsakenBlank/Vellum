//! the pieces layer styles and adjustment layers are built from:
//!
//! - `mix` fades between two images through a mask, which is how an adjustment layer lands
//!   on what is below it (a plain blend would darken see through areas twice)
//! - `style.morph` grows or shrinks an image's shape, for strokes
//! - `style.effect` turns a (usually blurred) copy of a layer's shape into a shadow, glow,
//!   satin, bevel light or stroke in one colour
//! - `source.pattern` tiles a small picture forever, for pattern fills and overlays
//!
//! the image editor wires these together per layer; see `vellum-image`'s `styles.rs`.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{
    InputSpec, NO_INPUTS, NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out, input_bounds,
};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};
use crate::rect::{Bounds, IRect};

use super::blend::MASK_CHANNELS;
use super::colorfn::{mix4, sample, working_color};
use super::simple::{Setup, Simple};

fn dispatch(
    gx: &mut GpuExec,
    kernel: &Kernel,
    inputs: &[Option<&GpuImage>],
    region: IRect,
    params: &[[f32; 4]],
) -> Result<Out<GpuImage>, GpuError> {
    Ok(Out::New(gx.dispatch(
        kernel,
        inputs,
        region,
        Dispatch {
            params,
            ..Default::default()
        },
    )?))
}

// ---- mix ----

pub struct MixNode;

static MIX: NodeInfo = NodeInfo {
    id: "mix",
    title: "Mix",
    category: NodeCategory::Blend,
    description: "Fades from one image to another, through a mask if there is one.",
    inputs: &[
        InputSpec {
            id: "from",
            label: "From",
        },
        InputSpec {
            id: "to",
            label: "To",
        },
        InputSpec {
            id: "mask",
            label: "Mask",
        },
    ],
    params: &[
        ParamSpec::percent("amount", "Amount", 0.0, 100.0, 100.0),
        ParamSpec::choice("mask_channel", "Mask from", MASK_CHANNELS, "luminance"),
        ParamSpec::float("outside", "Outside the mask", 0.0, 1.0, 1.0),
        // set when a mask input is wired, so an empty mask still counts
        ParamSpec::boolean("masked", "Masked", false).hide(),
    ],
};

const MIX_KERNEL: Kernel = Kernel {
    name: "mix",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    var m = a.x;
    if (a.w > 0.5) {
        let r = F.in_rect[2];
        var v = a.z;
        if (inside(r, p)) {
            let c = textureLoad(in2, p - r.xy, 0);
            if (a.y > 0.5) { v = c.a; } else { v = clamp(0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b, 0.0, 1.0); }
        }
        m = m * v;
    }
    return mix(load0(p), load1(p), m);
}
"#,
};

impl NodeKind for MixNode {
    fn info(&self) -> &'static NodeInfo {
        &MIX
    }

    fn bounds(&self, _cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        input_bounds(inputs, 0).union(input_bounds(inputs, 1))
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let p = cx.params;
        let amount = p.float("amount") / 100.0;
        let (from, to, mask) = (inputs[0], inputs[1], inputs.get(2).copied().flatten());
        let has_mask = p.bool("masked");
        if amount <= 0.0 {
            return Out::Input(0);
        }
        if amount >= 1.0 && !has_mask {
            return Out::Input(1);
        }
        let alpha = p.choice_index("mask_channel") == 1;
        let outside = p.float("outside");
        let get = |img: Option<&CpuImage>, x, y| img.map_or([0.0; 4], |i| i.get(x, y));
        Out::New(CpuImage::from_fn(region, |x, y| {
            let mut m = amount;
            if has_mask {
                let v = match mask {
                    Some(mi) if mi.rect.contains(x, y) => {
                        let c = mi.get(x, y);
                        if alpha {
                            c[3]
                        } else {
                            (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).clamp(0.0, 1.0)
                        }
                    }
                    _ => outside,
                };
                m *= v;
            }
            mix4(get(from, x, y), get(to, x, y), m)
        }))
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let p = cx.params;
        let amount = p.float("amount") / 100.0;
        let mask = inputs.get(2).copied().flatten();
        let has_mask = p.bool("masked");
        if amount <= 0.0 {
            return Ok(Out::Input(0));
        }
        if amount >= 1.0 && !has_mask {
            return Ok(Out::Input(1));
        }
        let params = [[
            amount,
            p.choice_index("mask_channel") as f32,
            p.float("outside"),
            f32::from(u8::from(has_mask)),
        ]];
        dispatch(
            gx,
            &MIX_KERNEL,
            &[inputs[0], inputs[1], mask],
            region,
            &params,
        )
    }
}

// ---- morph ----

pub struct Morph;

const MORPH_MODES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "grow",
        label: "Grow",
    },
    ChoiceOption {
        id: "shrink",
        label: "Shrink",
    },
];

static MORPH: NodeInfo = NodeInfo {
    id: "style.morph",
    title: "Grow or shrink shape",
    category: NodeCategory::Mask,
    description: "Makes a shape fatter or thinner by a distance, with round corners.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("radius", "Distance", 250.0, 3.0),
        ParamSpec::choice("mode", "Direction", MORPH_MODES, "grow"),
    ],
};

/// at most this many rings of samples, each with up to 48 around it
const MORPH_RINGS: i32 = 16;

/// the largest (or smallest) alpha on rings around the pixel, out to the radius, so the
/// result is a disc-shaped grow or shrink with an anti aliased edge
const MORPH_KERNEL: Kernel = Kernel {
    name: "morph",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let rings = i32(a.y);
    let shrink = a.z > 0.5;
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    var v = load0(p).a;
    for (var k = 1; k <= rings; k = k + 1) {
        let r = a.x * f32(k) / f32(rings);
        let n = clamp(i32(ceil(6.2831853 * r / 1.5)), 6, 48);
        for (var i = 0; i < n; i = i + 1) {
            let th = 6.2831853 * f32(i) / f32(n);
            let s = sample0(pos + vec2<f32>(cos(th), sin(th)) * r, false).a;
            if (shrink) { v = min(v, s); } else { v = max(v, s); }
        }
    }
    return vec4<f32>(v, v, v, v);
}
"#,
};

impl Simple for Morph {
    fn info(&self) -> &'static NodeInfo {
        &MORPH
    }
    fn kernel(&self) -> &'static Kernel {
        &MORPH_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let r = cx.params.float("radius") * cx.scale();
        let rings = (r.ceil() as i32).clamp(1, MORPH_RINGS);
        Some(Setup::new(vec![[
            r,
            rings as f32,
            cx.params.choice_index("mode") as f32,
            0.0,
        ]]))
    }
    fn reach(&self, cx: &NodeCx) -> i32 {
        (cx.params.float("radius") * cx.scale()).ceil() as i32 + 2
    }
    fn spreads(&self, cx: &NodeCx) -> bool {
        cx.params.choice_index("mode") == 0
    }
    fn pixel(&self, s: &Setup, _cx: &NodeCx, input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let rings = a[1] as i32;
        let shrink = a[2] > 0.5;
        let pos = [x as f32 + 0.5, y as f32 + 0.5];
        let mut v = input.get(x, y)[3];
        for k in 1..=rings {
            let r = a[0] * k as f32 / rings as f32;
            let n = ((std::f32::consts::TAU * r / 1.5).ceil() as i32).clamp(6, 48);
            for i in 0..n {
                let (sn, cs) = (std::f32::consts::TAU * i as f32 / n as f32).sin_cos();
                let s = sample(input, [pos[0] + cs * r, pos[1] + sn * r], false)[3];
                v = if shrink { v.min(s) } else { v.max(s) };
            }
        }
        [v; 4]
    }
}

// ---- layer effects ----

pub struct LayerEffect;

pub const EFFECT_KINDS: &[ChoiceOption] = &[
    ChoiceOption {
        id: "shadow",
        label: "Drop shadow",
    },
    ChoiceOption {
        id: "inner_shadow",
        label: "Inner shadow",
    },
    ChoiceOption {
        id: "outer_glow",
        label: "Outer glow",
    },
    ChoiceOption {
        id: "inner_glow_edge",
        label: "Inner glow from the edge",
    },
    ChoiceOption {
        id: "inner_glow_center",
        label: "Inner glow from the centre",
    },
    ChoiceOption {
        id: "satin",
        label: "Satin",
    },
    ChoiceOption {
        id: "bevel_highlight",
        label: "Bevel highlight",
    },
    ChoiceOption {
        id: "bevel_shadow",
        label: "Bevel shadow",
    },
    ChoiceOption {
        id: "stroke",
        label: "Stroke",
    },
];

static EFFECT: NodeInfo = NodeInfo {
    id: "style.effect",
    title: "Layer effect",
    category: NodeCategory::Effect,
    description: "Turns a layer's shape into a shadow, glow, satin, bevel light or stroke.",
    inputs: &[
        InputSpec {
            id: "shape",
            label: "Shape",
        },
        InputSpec {
            id: "layer",
            label: "Layer",
        },
        InputSpec {
            id: "inner",
            label: "Inner edge",
        },
    ],
    params: &[
        ParamSpec::choice("kind", "Effect", EFFECT_KINDS, "shadow"),
        ParamSpec::color("color", "Colour", [0.0, 0.0, 0.0, 1.0]),
        ParamSpec::point("offset", "Offset", [0.0, 0.0]),
        ParamSpec::percent("spread", "Spread", 0.0, 100.0, 0.0),
        ParamSpec::boolean("invert", "Invert", false),
        ParamSpec::degrees("angle", "Light angle", -180.0, 180.0, 120.0),
        ParamSpec::degrees("altitude", "Light altitude", 0.0, 90.0, 30.0),
        ParamSpec::percent("depth", "Depth", 1.0, 1000.0, 100.0),
        ParamSpec::boolean("down", "Press in", false),
    ],
};

const EFFECT_KERNEL: Kernel = Kernel {
    name: "layer_effect",
    body: r#"
fn spread(v: f32) -> f32 {
    return clamp(v / max(1.0 - param(1u).z, 0.01), 0.0, 1.0);
}
fn a0(q: vec2<f32>) -> f32 {
    return sample0(q, false).a;
}
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let k = u32(param(0u).w);
    let col = param(0u).xyz;
    let b = param(1u);
    let c = param(2u);
    let pos = vec2<f32>(p) + vec2<f32>(0.5);
    let off = b.xy;
    let layer = load1(p).a;
    var a = 0.0;
    switch k {
        case 0u: { a = spread(a0(pos - off)); }
        case 1u: { a = spread(1.0 - a0(pos - off)) * layer; }
        case 2u: { a = spread(a0(pos)); }
        case 3u: { a = spread(1.0 - a0(pos)) * layer; }
        case 4u: { a = spread(a0(pos)) * layer; }
        case 5u: {
            var v = abs(a0(pos + off) - a0(pos - off));
            if (b.w > 0.5) { v = 1.0 - v; }
            a = v * layer;
        }
        case 6u, 7u: {
            let dx = (a0(pos + vec2<f32>(1.0, 0.0)) - a0(pos - vec2<f32>(1.0, 0.0))) * c.z;
            let dy = (a0(pos + vec2<f32>(0.0, 1.0)) - a0(pos - vec2<f32>(0.0, 1.0))) * c.z;
            let n = normalize(vec3<f32>(-dx, -dy, 1.0));
            let l = vec3<f32>(c.x, c.y, c.w);
            var shade = dot(n, l) - l.z;
            if (k == 7u) { shade = -shade; }
            a = clamp(shade * 2.0, 0.0, 1.0) * layer;
        }
        default: { a = clamp(load0(p).a - load2(p).a, 0.0, 1.0); }
    }
    return vec4<f32>(col * a, a);
}
"#,
};

struct EffectSetup {
    kind: u32,
    params: [[f32; 4]; 3],
    offset: [f32; 2],
}

fn effect_setup(cx: &NodeCx) -> EffectSetup {
    let p = cx.params;
    let s = cx.scale();
    let kind = p.choice_index("kind");
    let col = working_color(cx, p.color("color"));
    let off = p.point("offset");
    let (sn, cs) = p.float("angle").to_radians().sin_cos();
    let (asn, acs) = p.float("altitude").to_radians().sin_cos();
    let down = p.bool("down");
    // a pixel's alpha step to a slope in document pixels, scaled by depth
    let depth = p.float("depth") / 100.0 * 0.5 * s * 8.0;
    let flip = if down { -1.0 } else { 1.0 };
    EffectSetup {
        kind,
        offset: [off[0] * s, off[1] * s],
        params: [
            [col[0], col[1], col[2], kind as f32],
            [
                off[0] * s,
                off[1] * s,
                p.float("spread") / 100.0,
                f32::from(u8::from(p.bool("invert"))),
            ],
            [cs * acs * flip, -sn * acs * flip, depth, asn],
        ],
    }
}

fn effect_pixel(
    e: &EffectSetup,
    shape: Option<&CpuImage>,
    layer: Option<&CpuImage>,
    inner: Option<&CpuImage>,
    x: i32,
    y: i32,
) -> [f32; 4] {
    let [col, b, c] = e.params;
    let spread = |v: f32| (v / (1.0 - b[2]).max(0.01)).clamp(0.0, 1.0);
    let a0 = |q: [f32; 2]| shape.map_or(0.0, |i| sample(i, q, false)[3]);
    let pos = [x as f32 + 0.5, y as f32 + 0.5];
    let off = [b[0], b[1]];
    let layer_a = layer.map_or(0.0, |i| i.get(x, y)[3]);
    let a = match e.kind {
        0 => spread(a0([pos[0] - off[0], pos[1] - off[1]])),
        1 => spread(1.0 - a0([pos[0] - off[0], pos[1] - off[1]])) * layer_a,
        2 => spread(a0(pos)),
        3 => spread(1.0 - a0(pos)) * layer_a,
        4 => spread(a0(pos)) * layer_a,
        5 => {
            let mut v = (a0([pos[0] + off[0], pos[1] + off[1]])
                - a0([pos[0] - off[0], pos[1] - off[1]]))
            .abs();
            if b[3] > 0.5 {
                v = 1.0 - v;
            }
            v * layer_a
        }
        6 | 7 => {
            let dx = (a0([pos[0] + 1.0, pos[1]]) - a0([pos[0] - 1.0, pos[1]])) * c[2];
            let dy = (a0([pos[0], pos[1] + 1.0]) - a0([pos[0], pos[1] - 1.0])) * c[2];
            let len = (dx * dx + dy * dy + 1.0).sqrt();
            let n = [-dx / len, -dy / len, 1.0 / len];
            let mut shade = n[0] * c[0] + n[1] * c[1] + n[2] * c[3] - c[3];
            if e.kind == 7 {
                shade = -shade;
            }
            (shade * 2.0).clamp(0.0, 1.0) * layer_a
        }
        _ => {
            let outer = shape.map_or(0.0, |i| i.get(x, y)[3]);
            let inn = inner.map_or(0.0, |i| i.get(x, y)[3]);
            (outer - inn).clamp(0.0, 1.0)
        }
    };
    [col[0] * a, col[1] * a, col[2] * a, a]
}

impl NodeKind for LayerEffect {
    fn info(&self) -> &'static NodeInfo {
        &EFFECT
    }

    fn bounds(&self, cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        let e = effect_setup(cx);
        match e.kind {
            0 | 2 => {
                let r = e.offset[0].abs().max(e.offset[1].abs()).ceil() as i32 + 1;
                input_bounds(inputs, 0).expand(r)
            }
            8 => input_bounds(inputs, 0),
            _ => input_bounds(inputs, 1),
        }
    }

    fn input_region(&self, cx: &NodeCx, input: usize, region: IRect) -> IRect {
        if input == 0 {
            let e = effect_setup(cx);
            let r = e.offset[0].abs().max(e.offset[1].abs()).ceil() as i32 + 2;
            region.expand(r)
        } else {
            region
        }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let e = effect_setup(cx);
        let (shape, layer, inner) = (
            inputs[0],
            inputs.get(1).copied().flatten(),
            inputs.get(2).copied().flatten(),
        );
        if shape.is_none() && !matches!(e.kind, 1 | 3) {
            return Out::Empty;
        }
        Out::New(CpuImage::from_fn(region, |x, y| {
            effect_pixel(&e, shape, layer, inner, x, y)
        }))
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let e = effect_setup(cx);
        if inputs[0].is_none() && !matches!(e.kind, 1 | 3) {
            return Ok(Out::Empty);
        }
        let ins = [
            inputs[0],
            inputs.get(1).copied().flatten(),
            inputs.get(2).copied().flatten(),
        ];
        dispatch(gx, &EFFECT_KERNEL, &ins, region, &e.params)
    }
}

// ---- pattern ----

pub struct PatternNode;

static PATTERN: NodeInfo = NodeInfo {
    id: "source.pattern",
    title: "Pattern",
    category: NodeCategory::Source,
    description: "Repeats a small picture across the whole canvas.",
    inputs: NO_INPUTS,
    params: &[
        ParamSpec::percent("scale", "Scale", 1.0, 1000.0, 100.0),
        ParamSpec::point("offset", "Offset", [0.0, 0.0]),
    ],
};

const PATTERN_KERNEL: Kernel = Kernel {
    name: "pattern",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let a = param(0u);
    let w = i32(data[0]);
    let h = i32(data[1]);
    let q = (vec2<f32>(p) + vec2<f32>(0.5)) * a.z - a.xy;
    let u = vec2<i32>(floor(q / a.w));
    let x = ((u.x % w) + w) % w;
    let y = ((u.y % h) + h) % h;
    let i = 2u + u32((y * w + x) * 4);
    let c = vec4<f32>(data[i], data[i + 1u], data[i + 2u], data[i + 3u]);
    let rgb = encode3(F.encoding, F.gamma, decode3(1u, 1.0, c.rgb));
    return premultiply(vec4<f32>(rgb, c.a));
}
"#,
};

/// pixels for a pattern node: width, height, then straight srgb rgba rows
pub fn pattern_data(w: u32, h: u32, rgba: &[[f32; 4]]) -> crate::graph::NodeData {
    let mut v = Vec::with_capacity(2 + rgba.len() * 4);
    v.push(w as f32);
    v.push(h as f32);
    v.extend(rgba.iter().flatten());
    crate::graph::NodeData::new(v)
}

impl Simple for PatternNode {
    fn info(&self) -> &'static NodeInfo {
        &PATTERN
    }
    fn kernel(&self) -> &'static Kernel {
        &PATTERN_KERNEL
    }
    fn generates(&self) -> bool {
        true
    }
    fn setup(&self, cx: &NodeCx) -> Option<Setup> {
        let data = cx.data()?;
        let (w, h) = (data.values.first()?, data.values.get(1)?);
        if *w < 1.0 || *h < 1.0 || data.values.len() < 2 + (*w * *h) as usize * 4 {
            return None;
        }
        let p = cx.params;
        let o = p.point("offset");
        Some(
            Setup::new(vec![[
                o[0],
                o[1],
                1.0 / cx.scale(),
                (p.float("scale") / 100.0).max(0.01),
            ]])
            .with_data(data.values.clone()),
        )
    }
    fn pixel(&self, s: &Setup, cx: &NodeCx, _input: &CpuImage, x: i32, y: i32) -> [f32; 4] {
        let a = s.p(0);
        let (w, h) = (s.data[0] as i32, s.data[1] as i32);
        let q = [
            (x as f32 + 0.5) * a[2] - a[0],
            (y as f32 + 0.5) * a[2] - a[1],
        ];
        let u = [(q[0] / a[3]).floor() as i32, (q[1] / a[3]).floor() as i32];
        let (px, py) = (u[0].rem_euclid(w), u[1].rem_euclid(h));
        let i = 2 + ((py * w + px) * 4) as usize;
        let c = [s.data[i], s.data[i + 1], s.data[i + 2], s.data[i + 3]];
        let rgb = working_color(cx, c);
        super::premultiply([rgb[0], rgb[1], rgb[2], c[3]])
    }
}
