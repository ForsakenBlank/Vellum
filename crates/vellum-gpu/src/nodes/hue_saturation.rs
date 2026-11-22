//! hue and saturation. works in hsl like photoshop's master channel: hue turns around the
//! colour wheel, saturation scales, lightness moves towards black or white. colorize sets
//! one hue for the whole image and keeps the brightness.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out};
use crate::image::CpuImage;
use crate::params::ParamSpec;
use crate::rect::IRect;

use super::colorfn::{hsl_to_rgb, rgb_to_hsl};

pub struct HueSaturation;

static INFO: NodeInfo = NodeInfo {
    id: "effect.hue_saturation",
    title: "Hue/Saturation",
    category: NodeCategory::Color,
    description: "Shifts colours around the colour wheel and makes them richer or duller.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::degrees("hue", "Hue", -180.0, 180.0, 0.0),
        ParamSpec::float("saturation", "Saturation", -100.0, 100.0, 0.0),
        ParamSpec::float("lightness", "Lightness", -100.0, 100.0, 0.0),
        ParamSpec::boolean("colorize", "Colorize", false),
    ],
};

/// params[0] = (hue turn in [0, 1), saturation, lightness, colorize), all in [-1, 1]
const KERNEL: Kernel = Kernel {
    name: "hue_saturation",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = unpremultiply(load0(p));
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    let a = param(0u);
    var hsl = rgb_to_hsl(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    if (a.w > 0.5) {
        hsl.x = a.x;
        hsl.y = max(a.y, 0.0);
    } else {
        hsl.x = fract(hsl.x + a.x);
        hsl.y = clamp(hsl.y * (1.0 + a.y), 0.0, 1.0);
    }
    if (a.z < 0.0) { hsl.z = hsl.z * (1.0 + a.z); } else { hsl.z = hsl.z + a.z * (1.0 - hsl.z); }
    return premultiply(vec4<f32>(hsl_to_rgb(hsl), c.a));
}
"#,
};

fn setup(cx: &NodeCx) -> [f32; 4] {
    let p = cx.params;
    let turn = p.float("hue") / 360.0;
    let colorize = p.bool("colorize");
    [
        if colorize { turn - turn.floor() } else { turn },
        p.float("saturation") / 100.0,
        p.float("lightness") / 100.0,
        f32::from(u8::from(colorize)),
    ]
}

fn adjust(a: [f32; 4], c: [f32; 4]) -> [f32; 4] {
    let c = super::unpremultiply(c);
    if c[3] <= 0.0 {
        return [0.0; 4];
    }
    let mut hsl = rgb_to_hsl([c[0], c[1], c[2]].map(|v| v.clamp(0.0, 1.0)));
    if a[3] > 0.5 {
        hsl[0] = a[0];
        hsl[1] = a[1].max(0.0);
    } else {
        let h = hsl[0] + a[0];
        hsl[0] = h - h.floor();
        hsl[1] = (hsl[1] * (1.0 + a[1])).clamp(0.0, 1.0);
    }
    hsl[2] = if a[2] < 0.0 {
        hsl[2] * (1.0 + a[2])
    } else {
        hsl[2] + a[2] * (1.0 - hsl[2])
    };
    let rgb = hsl_to_rgb(hsl);
    super::premultiply([rgb[0], rgb[1], rgb[2], c[3]])
}

impl NodeKind for HueSaturation {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let a = setup(cx);
        super::empty_or(inputs.first().copied().flatten(), |input| {
            if a == [0.0; 4] {
                return Out::Input(0);
            }
            Out::New(super::map_pixels(input, region, |c, _, _| adjust(a, c)))
        })
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let a = setup(cx);
        let Some(input) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        if a == [0.0; 4] {
            return Ok(Out::Input(0));
        }
        let region = region.intersect(&input.rect);
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[Some(input)],
            region,
            Dispatch {
                params: &[a],
                ..Default::default()
            },
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trips() {
        for c in [
            [0.8, 0.2, 0.3],
            [0.1, 0.9, 0.5],
            [0.3, 0.3, 0.3],
            [0.0, 0.0, 1.0],
        ] {
            let back = hsl_to_rgb(rgb_to_hsl(c));
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-5, "{c:?} {back:?}");
            }
        }
    }

    #[test]
    fn a_half_turn_swaps_red_for_cyan() {
        let out = adjust([0.5, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0]);
        assert!(out[0] < 1e-5 && (out[1] - 1.0).abs() < 1e-5 && (out[2] - 1.0).abs() < 1e-5);
        let grey = adjust([0.0, -1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0]);
        assert!((grey[0] - grey[1]).abs() < 1e-6);
    }
}
