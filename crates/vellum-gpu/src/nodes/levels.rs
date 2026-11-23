//! levels, the simplest effect and the one to copy: a static `NodeInfo`, a kernel body,
//! a cpu twin of the same maths, and the two `NodeKind` methods that run them.
//!
//! like photoshop, values are on a 0 to 255 scale whatever the bit depth, and the maths
//! runs on straight colour in the working space encoding.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};
use crate::rect::IRect;

pub struct Levels;

pub const CHANNELS: &[ChoiceOption] = &[
    ChoiceOption {
        id: "rgb",
        label: "RGB",
    },
    ChoiceOption {
        id: "red",
        label: "Red",
    },
    ChoiceOption {
        id: "green",
        label: "Green",
    },
    ChoiceOption {
        id: "blue",
        label: "Blue",
    },
];

static INFO: NodeInfo = NodeInfo {
    id: "effect.levels",
    title: "Levels",
    category: NodeCategory::Color,
    description: "Sets the black point, white point and midtones.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("channel", "Channel", CHANNELS, "rgb"),
        ParamSpec::float("input_black", "Input black", 0.0, 255.0, 0.0),
        ParamSpec::float("input_white", "Input white", 0.0, 255.0, 255.0),
        ParamSpec::float("gamma", "Midtones", 0.01, 9.99, 1.0),
        ParamSpec::float("output_black", "Output black", 0.0, 255.0, 0.0),
        ParamSpec::float("output_white", "Output white", 0.0, 255.0, 255.0),
    ],
};

/// params[0] = (input black, input white, 1 / gamma, clamp), params[1] = (output black,
/// output white), params[2] = which channels change
const KERNEL: Kernel = Kernel {
    name: "levels",
    body: r#"
fn level1(x: f32) -> f32 {
    let a = param(0u);
    var v = (x - a.x) / max(a.y - a.x, 1e-6);
    v = max(v, 0.0);
    if (a.w > 0.5) { v = min(v, 1.0); }
    v = pow(v, a.z);
    return param(1u).x + v * (param(1u).y - param(1u).x);
}

fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = unpremultiply(load0(p));
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    let m = param(2u).xyz;
    let out = vec3<f32>(level1(c.r), level1(c.g), level1(c.b));
    return premultiply(vec4<f32>(mix(c.rgb, out, m), c.a));
}
"#,
};

struct Setup {
    params: [[f32; 4]; 3],
}

impl Setup {
    fn of(cx: &NodeCx) -> Self {
        let p = cx.params;
        let mask = match p.choice_index("channel") {
            1 => [1.0, 0.0, 0.0, 0.0],
            2 => [0.0, 1.0, 0.0, 0.0],
            3 => [0.0, 0.0, 1.0, 0.0],
            _ => [1.0, 1.0, 1.0, 0.0],
        };
        Self {
            params: [
                [
                    p.float("input_black") / 255.0,
                    p.float("input_white") / 255.0,
                    1.0 / p.float("gamma"),
                    f32::from(u8::from(cx.clamp())),
                ],
                [
                    p.float("output_black") / 255.0,
                    p.float("output_white") / 255.0,
                    0.0,
                    0.0,
                ],
                mask,
            ],
        }
    }

    fn is_identity(&self) -> bool {
        let [a, b, _] = self.params;
        a[0] == 0.0 && a[1] == 1.0 && a[2] == 1.0 && b[0] == 0.0 && b[1] == 1.0
    }

    fn level(&self, x: f32) -> f32 {
        let [a, b, _] = self.params;
        let mut v = ((x - a[0]) / (a[1] - a[0]).max(1e-6)).max(0.0);
        if a[3] > 0.5 {
            v = v.min(1.0);
        }
        b[0] + v.powf(a[2]) * (b[1] - b[0])
    }
}

impl NodeKind for Levels {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let s = Setup::of(cx);
        super::empty_or(inputs.first().copied().flatten(), |input| {
            if s.is_identity() {
                return Out::Input(0);
            }
            let m = s.params[2];
            Out::New(super::map_pixels(input, region, |c, _, _| {
                let c = super::unpremultiply(c);
                if c[3] <= 0.0 {
                    return [0.0; 4];
                }
                let mut out = c;
                for k in 0..3 {
                    out[k] = c[k] + (s.level(c[k]) - c[k]) * m[k];
                }
                super::premultiply(out)
            }))
        })
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let s = Setup::of(cx);
        let Some(input) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        if s.is_identity() {
            return Ok(Out::Input(0));
        }
        let region = region.intersect(&input.rect);
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[Some(input)],
            region,
            Dispatch {
                params: &s.params,
                ..Default::default()
            },
        )?))
    }
}
