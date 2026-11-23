//! puts a layer over a backdrop with a blend mode, opacity, fill and an optional mask. the
//! maths is `blend::composite`, shared with the prelude.

use crate::blend::{BlendMode, composite};
use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{InputSpec, NodeCategory, NodeCx, NodeInfo, NodeKind, Out, input_bounds};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};
use crate::rect::{Bounds, IRect};

pub struct BlendNode;

pub const MODE_OPTIONS: &[ChoiceOption] = &[
    ChoiceOption {
        id: "normal",
        label: "Normal",
    },
    ChoiceOption {
        id: "dissolve",
        label: "Dissolve",
    },
    ChoiceOption {
        id: "darken",
        label: "Darken",
    },
    ChoiceOption {
        id: "multiply",
        label: "Multiply",
    },
    ChoiceOption {
        id: "color_burn",
        label: "Color Burn",
    },
    ChoiceOption {
        id: "linear_burn",
        label: "Linear Burn",
    },
    ChoiceOption {
        id: "darker_color",
        label: "Darker Color",
    },
    ChoiceOption {
        id: "lighten",
        label: "Lighten",
    },
    ChoiceOption {
        id: "screen",
        label: "Screen",
    },
    ChoiceOption {
        id: "color_dodge",
        label: "Color Dodge",
    },
    ChoiceOption {
        id: "linear_dodge_add",
        label: "Linear Dodge (Add)",
    },
    ChoiceOption {
        id: "lighter_color",
        label: "Lighter Color",
    },
    ChoiceOption {
        id: "overlay",
        label: "Overlay",
    },
    ChoiceOption {
        id: "soft_light",
        label: "Soft Light",
    },
    ChoiceOption {
        id: "hard_light",
        label: "Hard Light",
    },
    ChoiceOption {
        id: "vivid_light",
        label: "Vivid Light",
    },
    ChoiceOption {
        id: "linear_light",
        label: "Linear Light",
    },
    ChoiceOption {
        id: "pin_light",
        label: "Pin Light",
    },
    ChoiceOption {
        id: "hard_mix",
        label: "Hard Mix",
    },
    ChoiceOption {
        id: "difference",
        label: "Difference",
    },
    ChoiceOption {
        id: "exclusion",
        label: "Exclusion",
    },
    ChoiceOption {
        id: "subtract",
        label: "Subtract",
    },
    ChoiceOption {
        id: "divide",
        label: "Divide",
    },
    ChoiceOption {
        id: "hue",
        label: "Hue",
    },
    ChoiceOption {
        id: "saturation",
        label: "Saturation",
    },
    ChoiceOption {
        id: "color",
        label: "Color",
    },
    ChoiceOption {
        id: "luminosity",
        label: "Luminosity",
    },
];

pub const MASK_CHANNELS: &[ChoiceOption] = &[
    ChoiceOption {
        id: "luminance",
        label: "Luminance",
    },
    ChoiceOption {
        id: "alpha",
        label: "Alpha",
    },
];

static INFO: NodeInfo = NodeInfo {
    id: "blend",
    title: "Blend",
    category: NodeCategory::Blend,
    description: "Puts a layer over a backdrop using a blend mode.",
    inputs: &[
        InputSpec {
            id: "backdrop",
            label: "Backdrop",
        },
        InputSpec {
            id: "layer",
            label: "Layer",
        },
        InputSpec {
            id: "mask",
            label: "Mask",
        },
    ],
    params: &[
        ParamSpec::choice("mode", "Blend mode", MODE_OPTIONS, "normal"),
        ParamSpec::percent("opacity", "Opacity", 0.0, 100.0, 100.0),
        ParamSpec::percent("fill", "Fill", 0.0, 100.0, 100.0),
        ParamSpec::choice("mask_channel", "Mask from", MASK_CHANNELS, "luminance"),
        ParamSpec::float("mask_outside", "Mask outside its area", 0.0, 1.0, 0.0),
    ],
};

/// the node's blend mode, for callers building graphs in code
pub fn mode_param(mode: BlendMode) -> &'static str {
    MODE_OPTIONS[mode.id() as usize].id
}

const KERNEL: Kernel = Kernel {
    name: "blend",
    body: r#"
fn mask_value(p: vec2<i32>) -> f32 {
    if (param(1u).x < 0.5) { return 1.0; }
    let r = F.in_rect[2];
    if (!inside(r, p)) { return param(0u).z; }
    let m = textureLoad(in2, p - r.xy, 0);
    if (u32(param(0u).w) == 1u) { return m.a; }
    return clamp(0.2126 * m.r + 0.7152 * m.g + 0.0722 * m.b, 0.0, 1.0);
}

fn kernel(p: vec2<i32>) -> vec4<f32> {
    let mode = u32(param(0u).x);
    let alpha = param(0u).y * mask_value(p);
    return composite(mode, load0(p), load1(p), alpha, param(1u).y > 0.5, level0(p));
}
"#,
};

struct Setup {
    mode: BlendMode,
    alpha: f32,
    outside: f32,
    alpha_channel: bool,
}

fn setup(cx: &NodeCx) -> Setup {
    let p = cx.params;
    Setup {
        mode: BlendMode::from_id(p.choice_index("mode")),
        alpha: (p.float("opacity") / 100.0) * (p.float("fill") / 100.0),
        outside: p.float("mask_outside"),
        alpha_channel: p.choice_index("mask_channel") == 1,
    }
}

fn mask_at(mask: &CpuImage, x: i32, y: i32, s: &Setup) -> f32 {
    if !mask.rect.contains(x, y) {
        return s.outside;
    }
    let m = mask.get(x, y);
    if s.alpha_channel {
        m[3]
    } else {
        (0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2]).clamp(0.0, 1.0)
    }
}

impl NodeKind for BlendNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn bounds(&self, cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        let s = setup(cx);
        let backdrop = input_bounds(inputs, 0);
        let mut layer = input_bounds(inputs, 1);
        if s.alpha <= 0.0 {
            return backdrop;
        }
        if let Some(Some(mask)) = inputs.get(2).copied()
            && s.outside <= 0.0
        {
            layer = layer.intersect(mask);
        }
        backdrop.union(layer)
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let s = setup(cx);
        let backdrop = inputs.first().copied().flatten();
        let Some(layer) = inputs.get(1).copied().flatten() else {
            return if backdrop.is_some() {
                Out::Input(0)
            } else {
                Out::Empty
            };
        };
        if s.alpha <= 0.0 {
            return if backdrop.is_some() {
                Out::Input(0)
            } else {
                Out::Empty
            };
        }
        let mask = inputs.get(2).copied().flatten();
        let clamp = cx.clamp();
        let level = cx.level;
        Out::New(CpuImage::from_fn(region, |x, y| {
            let b = backdrop.map_or([0.0; 4], |i| i.get(x, y));
            let m = mask.map_or(1.0, |m| mask_at(m, x, y, &s));
            composite(
                s.mode,
                b,
                layer.get(x, y),
                s.alpha * m,
                clamp,
                x << level,
                y << level,
            )
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
        let backdrop = inputs.first().copied().flatten();
        let layer = inputs.get(1).copied().flatten();
        if layer.is_none() || s.alpha <= 0.0 {
            return Ok(if backdrop.is_some() {
                Out::Input(0)
            } else {
                Out::Empty
            });
        }
        let mask = inputs.get(2).copied().flatten();
        let params = [
            [
                s.mode.id() as f32,
                s.alpha,
                s.outside,
                f32::from(u8::from(s.alpha_channel)),
            ],
            [
                f32::from(u8::from(mask.is_some())),
                f32::from(u8::from(cx.clamp())),
                0.0,
                0.0,
            ],
        ];
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[backdrop, layer, mask],
            region,
            Dispatch {
                params: &params,
                ..Default::default()
            },
        )?))
    }
}
