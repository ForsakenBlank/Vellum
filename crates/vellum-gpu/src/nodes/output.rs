//! the end of a graph: flattens onto a background if asked and converts from the working
//! space to the space of the file or display the pixels are going to. an attached colour
//! transform (an icc display profile) wins over the named space.

use crate::color::ColorTransform;
use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out, input_bounds};
use crate::image::CpuImage;
use crate::params::{ParamSpec, color_to_working};
use crate::rect::{Bounds, IRect};

use super::{SPACE_OPTIONS, space_by_id};

pub struct OutputNode;

static INFO: NodeInfo = NodeInfo {
    id: "output",
    title: "Output",
    category: NodeCategory::Output,
    description: "Converts the result to the colour space of a file or display.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::choice("space", "Colour space", SPACE_OPTIONS, "srgb"),
        ParamSpec::color("background", "Background", [0.0, 0.0, 0.0, 0.0]),
    ],
};

const KERNEL: Kernel = Kernel {
    name: "output",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    var c = load0(p);
    let bg = param(31u);
    if (bg.a > 0.0) { c = c + bg * (1.0 - c.a); }
    let s = unpremultiply(c);
    if (s.a <= 0.0) { return vec4<f32>(0.0); }
    return premultiply(vec4<f32>(color_steps(0u, s.rgb), s.a));
}
"#,
};

fn transform(cx: &NodeCx) -> ColorTransform {
    match cx.color_transform() {
        Some(t) => (**t).clone(),
        None => ColorTransform::between(
            cx.working.space(),
            space_by_id(&cx.params.choice("space"), cx.working),
        ),
    }
}

fn background(cx: &NodeCx) -> [f32; 4] {
    super::premultiply(color_to_working(cx.params.color("background"), cx.working))
}

impl NodeKind for OutputNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn bounds(&self, cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        if background(cx)[3] > 0.0 {
            Bounds::Infinite
        } else {
            input_bounds(inputs, 0)
        }
    }

    fn key(&self, cx: &NodeCx, _region: IRect, h: &mut dyn std::hash::Hasher) {
        h.write_u64(transform(cx).key());
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let t = transform(cx);
        let bg = background(cx);
        let input = inputs.first().copied().flatten();
        if bg[3] <= 0.0 && t.is_identity() {
            return if input.is_some() {
                Out::Input(0)
            } else {
                Out::Empty
            };
        }
        Out::New(CpuImage::from_fn(region, |x, y| {
            let mut c = input.map_or([0.0; 4], |i| i.get(x, y));
            if bg[3] > 0.0 {
                c = [0, 1, 2, 3].map(|k| c[k] + bg[k] * (1.0 - c[3]));
            }
            let s = super::unpremultiply(c);
            if s[3] <= 0.0 {
                return [0.0; 4];
            }
            let rgb = t.apply([s[0], s[1], s[2]]);
            super::premultiply([rgb[0], rgb[1], rgb[2], s[3]])
        }))
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let t = transform(cx);
        let bg = background(cx);
        let input = inputs.first().copied().flatten();
        if bg[3] <= 0.0 && t.is_identity() {
            return Ok(if input.is_some() {
                Out::Input(0)
            } else {
                Out::Empty
            });
        }
        let mut params = Vec::new();
        t.pack(&mut params);
        params.resize(32, [0.0; 4]);
        params[31] = bg;
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[input],
            region,
            Dispatch {
                params: &params,
                lut: t.lut().map(|l| &**l),
                ..Default::default()
            },
        )?))
    }
}
