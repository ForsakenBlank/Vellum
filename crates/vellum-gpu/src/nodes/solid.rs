//! a solid colour everywhere

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NO_INPUTS, NodeCategory, NodeCx, NodeInfo, NodeKind, Out};
use crate::image::CpuImage;
use crate::params::{ParamSpec, color_to_working};
use crate::rect::{Bounds, IRect};

pub struct SolidNode;

static INFO: NodeInfo = NodeInfo {
    id: "source.solid",
    title: "Solid colour",
    category: NodeCategory::Source,
    description: "Fills everything with one colour.",
    inputs: NO_INPUTS,
    params: &[ParamSpec::color("color", "Colour", [1.0, 1.0, 1.0, 1.0])],
};

const KERNEL: Kernel = Kernel {
    name: "solid",
    body: "fn kernel(p: vec2<i32>) -> vec4<f32> { return param(0u); }",
};

fn color(cx: &NodeCx) -> [f32; 4] {
    super::premultiply(color_to_working(cx.params.color("color"), cx.working))
}

impl NodeKind for SolidNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn bounds(&self, cx: &NodeCx, _inputs: &[Option<Bounds>]) -> Bounds {
        if cx.params.color("color")[3] <= 0.0 {
            Bounds::Empty
        } else {
            Bounds::Infinite
        }
    }

    fn cpu(&self, cx: &NodeCx, _inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        Out::New(CpuImage::filled(region, color(cx)))
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        _inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let params = [color(cx)];
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[],
            region,
            Dispatch {
                params: &params,
                ..Default::default()
            },
        )?))
    }
}
