//! colour conversions inside a graph: between named spaces, or through an attached
//! transform such as an icc profile lut (and, in phase 10, a .cube file).

use crate::color::ColorTransform;
use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out};
use crate::image::CpuImage;
use crate::params::ParamSpec;
use crate::rect::IRect;

use super::{SPACE_OPTIONS, space_by_id};

/// runs a packed colour transform on straight colour. shared with the output node.
pub const COLOR_KERNEL: Kernel = Kernel {
    name: "color_steps",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = unpremultiply(load0(p));
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    return premultiply(vec4<f32>(color_steps(0u, c.rgb), c.a));
}
"#,
};

pub fn apply_cpu(input: &CpuImage, region: IRect, t: &ColorTransform) -> CpuImage {
    super::map_pixels(input, region, |c, _, _| {
        let s = super::unpremultiply(c);
        if s[3] <= 0.0 {
            return [0.0; 4];
        }
        let rgb = t.apply([s[0], s[1], s[2]]);
        super::premultiply([rgb[0], rgb[1], rgb[2], s[3]])
    })
}

pub fn apply_gpu(
    gx: &mut GpuExec,
    input: &GpuImage,
    region: IRect,
    t: &ColorTransform,
) -> Result<GpuImage, GpuError> {
    let mut params = Vec::new();
    t.pack(&mut params);
    gx.dispatch(
        &COLOR_KERNEL,
        &[Some(input)],
        region.intersect(&input.rect),
        Dispatch {
            params: &params,
            lut: t.lut().map(|l| &**l),
            ..Default::default()
        },
    )
}

pub struct ColorConvertNode;

static CONVERT_INFO: NodeInfo = NodeInfo {
    id: "color.convert",
    title: "Convert colour space",
    category: NodeCategory::Color,
    description: "Reinterprets colours from one colour space as another.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("from", "From", SPACE_OPTIONS, "working"),
        ParamSpec::choice("to", "To", SPACE_OPTIONS, "srgb"),
    ],
};

fn convert_transform(cx: &NodeCx) -> ColorTransform {
    let from = space_by_id(&cx.params.choice("from"), cx.working);
    let to = space_by_id(&cx.params.choice("to"), cx.working);
    ColorTransform::between(from, to)
}

impl NodeKind for ColorConvertNode {
    fn info(&self) -> &'static NodeInfo {
        &CONVERT_INFO
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let t = convert_transform(cx);
        super::empty_or(inputs.first().copied().flatten(), |input| {
            if t.is_identity() {
                Out::Input(0)
            } else {
                Out::New(apply_cpu(input, region, &t))
            }
        })
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let t = convert_transform(cx);
        let Some(input) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        if t.is_identity() {
            return Ok(Out::Input(0));
        }
        Ok(Out::New(apply_gpu(gx, input, region, &t)?))
    }
}

/// applies the colour transform attached to the node
pub struct LutNode;

static LUT_INFO: NodeInfo = NodeInfo {
    id: "color.lut",
    title: "Colour lookup",
    category: NodeCategory::Color,
    description: "Applies a colour profile or lookup table.",
    inputs: ONE_INPUT,
    params: &[],
};

impl NodeKind for LutNode {
    fn info(&self) -> &'static NodeInfo {
        &LUT_INFO
    }

    fn key(&self, cx: &NodeCx, _region: IRect, h: &mut dyn std::hash::Hasher) {
        h.write_u64(cx.color_transform().map_or(0, |t| t.key()));
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        super::empty_or(inputs.first().copied().flatten(), |input| {
            match cx.color_transform() {
                Some(t) if !t.is_identity() => Out::New(apply_cpu(input, region, t)),
                _ => Out::Input(0),
            }
        })
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let Some(input) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        match cx.color_transform() {
            Some(t) if !t.is_identity() => Ok(Out::New(apply_gpu(gx, input, region, t)?)),
            _ => Ok(Out::Input(0)),
        }
    }
}
