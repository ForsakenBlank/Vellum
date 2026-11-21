//! gradients: linear, radial, angle, reflected and diamond, like photoshop's gradient tool.
//! stops are interpolated in the working space, which is what photoshop does in 8 and 16
//! bit documents.

use crate::blend::pixel_noise;
use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NO_INPUTS, NodeCategory, NodeCx, NodeInfo, NodeKind, Out};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, GradientStop, ParamSpec, color_to_working};
use crate::rect::{Bounds, IRect};

pub struct GradientNode;

pub const SHAPES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "linear",
        label: "Linear",
    },
    ChoiceOption {
        id: "radial",
        label: "Radial",
    },
    ChoiceOption {
        id: "angle",
        label: "Angle",
    },
    ChoiceOption {
        id: "reflected",
        label: "Reflected",
    },
    ChoiceOption {
        id: "diamond",
        label: "Diamond",
    },
];

const BLACK_TO_WHITE: &[GradientStop] = &[
    GradientStop {
        offset: 0.0,
        color: [0.0, 0.0, 0.0, 1.0],
    },
    GradientStop {
        offset: 1.0,
        color: [1.0, 1.0, 1.0, 1.0],
    },
];

static INFO: NodeInfo = NodeInfo {
    id: "source.gradient",
    title: "Gradient",
    category: NodeCategory::Source,
    description: "A smooth blend between colours along a line.",
    inputs: NO_INPUTS,
    params: &[
        ParamSpec::choice("shape", "Shape", SHAPES, "linear"),
        ParamSpec::point("start", "Start", [0.0, 0.0]),
        ParamSpec::point("end", "End", [256.0, 0.0]),
        ParamSpec::gradient("stops", "Colours", BLACK_TO_WHITE),
        ParamSpec::boolean("reverse", "Reverse", false),
        ParamSpec::boolean("dither", "Dither", true),
    ],
};

const KERNEL: Kernel = Kernel {
    name: "gradient",
    body: r#"
fn gradient_t(q: vec2<f32>) -> f32 {
    let a = param(0u).xy;
    let b = param(0u).zw;
    let shape = u32(param(1u).x);
    let ab = b - a;
    let len2 = max(dot(ab, ab), 1e-12);
    let d = q - a;
    var t = 0.0;
    switch shape {
        case 1u: { t = length(d) / sqrt(len2); }
        case 2u: {
            let pi2 = 6.283185307179586;
            let ang = atan2(-d.y, d.x) - atan2(-ab.y, ab.x);
            t = fract(ang / pi2);
        }
        case 3u: { t = abs(dot(d, ab) / len2); }
        case 4u: {
            let u = dot(d, ab) / len2;
            let v = (d.x * -ab.y + d.y * ab.x) / len2;
            t = abs(u) + abs(v);
        }
        default: { t = dot(d, ab) / len2; }
    }
    if (param(1u).y > 0.5) { t = 1.0 - t; }
    return clamp(t, 0.0, 1.0);
}

fn stop(i: u32) -> vec4<f32> {
    return vec4<f32>(data[i * 5u + 1u], data[i * 5u + 2u], data[i * 5u + 3u], data[i * 5u + 4u]);
}

fn kernel(p: vec2<i32>) -> vec4<f32> {
    let s = f32(1 << F.level);
    let q = (vec2<f32>(p) + 0.5) * s;
    let t = gradient_t(q);
    let n = F.data_len / 5u;
    var c = stop(0u);
    for (var i = 1u; i < n; i = i + 1u) {
        let o0 = data[(i - 1u) * 5u];
        let o1 = data[i * 5u];
        if (t >= o1) {
            c = stop(i);
        } else if (t > o0) {
            c = mix(stop(i - 1u), stop(i), (t - o0) / max(o1 - o0, 1e-6));
            break;
        } else {
            break;
        }
    }
    if (param(1u).z > 0.5) {
        let p0 = level0(p);
        let n = (pixel_noise(p0.x, p0.y) - 0.5) / 255.0;
        c = vec4<f32>(c.rgb + n, c.a);
    }
    return premultiply(c);
}
"#,
};

struct Prepared {
    params: [[f32; 4]; 2],
    stops: Vec<f32>,
}

fn prepare(cx: &NodeCx) -> Prepared {
    let p = cx.params;
    let a = p.point("start");
    let b = p.point("end");
    let mut stops: Vec<f32> = Vec::new();
    let list = p.gradient("stops");
    for s in &list {
        let c = color_to_working(s.color, cx.working);
        stops.extend_from_slice(&[s.offset.clamp(0.0, 1.0), c[0], c[1], c[2], c[3]]);
    }
    if stops.is_empty() {
        stops.extend_from_slice(&[0.0, 0.0, 0.0, 0.0, 0.0]);
    }
    Prepared {
        params: [
            [a[0], a[1], b[0], b[1]],
            [
                p.choice_index("shape") as f32,
                f32::from(u8::from(p.bool("reverse"))),
                f32::from(u8::from(p.bool("dither"))),
                0.0,
            ],
        ],
        stops,
    }
}

fn gradient_t(prep: &Prepared, q: [f32; 2]) -> f32 {
    let [ax, ay, bx, by] = prep.params[0];
    let shape = prep.params[1][0] as u32;
    let (abx, aby) = (bx - ax, by - ay);
    let len2 = (abx * abx + aby * aby).max(1e-12);
    let (dx, dy) = (q[0] - ax, q[1] - ay);
    let mut t = match shape {
        1 => (dx * dx + dy * dy).sqrt() / len2.sqrt(),
        2 => {
            let ang = (-dy).atan2(dx) - (-aby).atan2(abx);
            let v = ang / std::f32::consts::TAU;
            v - v.floor()
        }
        3 => ((dx * abx + dy * aby) / len2).abs(),
        4 => {
            let u = (dx * abx + dy * aby) / len2;
            let v = (dx * -aby + dy * abx) / len2;
            u.abs() + v.abs()
        }
        _ => (dx * abx + dy * aby) / len2,
    };
    if prep.params[1][1] > 0.5 {
        t = 1.0 - t;
    }
    t.clamp(0.0, 1.0)
}

fn color_at(prep: &Prepared, t: f32) -> [f32; 4] {
    let s = &prep.stops;
    let n = s.len() / 5;
    let stop = |i: usize| [s[i * 5 + 1], s[i * 5 + 2], s[i * 5 + 3], s[i * 5 + 4]];
    let mut c = stop(0);
    for i in 1..n {
        let (o0, o1) = (s[(i - 1) * 5], s[i * 5]);
        if t >= o1 {
            c = stop(i);
        } else if t > o0 {
            let f = (t - o0) / (o1 - o0).max(1e-6);
            let (c0, c1) = (stop(i - 1), stop(i));
            c = [0, 1, 2, 3].map(|k| c0[k] + (c1[k] - c0[k]) * f);
            break;
        } else {
            break;
        }
    }
    c
}

impl NodeKind for GradientNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn bounds(&self, _cx: &NodeCx, _inputs: &[Option<Bounds>]) -> Bounds {
        Bounds::Infinite
    }

    fn cpu(&self, cx: &NodeCx, _inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let prep = prepare(cx);
        let s = (1u64 << cx.level) as f32;
        let level = cx.level;
        let dither = prep.params[1][2] > 0.5;
        Out::New(CpuImage::from_fn(region, |x, y| {
            let q = [(x as f32 + 0.5) * s, (y as f32 + 0.5) * s];
            let mut c = color_at(&prep, gradient_t(&prep, q));
            if dither {
                let n = (pixel_noise(x << level, y << level) - 0.5) / 255.0;
                c = [c[0] + n, c[1] + n, c[2] + n, c[3]];
            }
            super::premultiply(c)
        }))
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        _inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let prep = prepare(cx);
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[],
            region,
            Dispatch {
                params: &prep.params,
                data: Some(&prep.stops),
                ..Default::default()
            },
        )?))
    }
}
