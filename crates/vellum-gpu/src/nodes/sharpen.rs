//! sharpen as an unsharp mask: blur a copy, then push every pixel away from its blurred
//! value. shows how one node chains passes, reusing the gaussian blur's.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out};
use crate::image::CpuImage;
use crate::params::ParamSpec;
use crate::rect::IRect;

use super::gaussian_blur::{blur_cpu, blur_gpu};

pub struct Sharpen;

static INFO: NodeInfo = NodeInfo {
    id: "effect.sharpen",
    title: "Sharpen",
    category: NodeCategory::Effect,
    description: "Makes edges crisper by increasing contrast along them.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("amount", "Amount", 0.0, 500.0, 100.0),
        ParamSpec::pixels("radius", "Radius", 250.0, 1.0),
        ParamSpec::float("threshold", "Threshold", 0.0, 255.0, 0.0),
    ],
};

/// params[0] = (amount, threshold, clamp)
const KERNEL: Kernel = Kernel {
    name: "sharpen",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = load0(p);
    let b = load1(p);
    let a = param(0u);
    let d = c - b;
    let luma = abs(0.2126 * d.r + 0.7152 * d.g + 0.0722 * d.b);
    if (luma < a.y) { return c; }
    var out = c + d * a.x;
    out.a = clamp(out.a, 0.0, 1.0);
    if (a.z > 0.5) {
        out = vec4<f32>(clamp(out.rgb, vec3<f32>(0.0), vec3<f32>(out.a)), out.a);
    } else {
        out = vec4<f32>(max(out.rgb, vec3<f32>(0.0)), out.a);
    }
    return out;
}
"#,
};

const MIN_SIGMA: f32 = 0.25;

fn setup(cx: &NodeCx) -> (f32, f32, [f32; 4]) {
    let p = cx.params;
    let sigma = p.float("radius") * cx.scale();
    let amount = p.float("amount") / 100.0;
    (
        sigma,
        amount,
        [
            amount,
            p.float("threshold") / 255.0,
            f32::from(u8::from(cx.clamp())),
            0.0,
        ],
    )
}

fn half_width(sigma: f32) -> i32 {
    (3.0 * sigma).ceil().max(1.0) as i32
}

fn combine(a: [f32; 4], c: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let d = [c[0] - b[0], c[1] - b[1], c[2] - b[2], c[3] - b[3]];
    let luma = (0.2126 * d[0] + 0.7152 * d[1] + 0.0722 * d[2]).abs();
    if luma < a[1] {
        return c;
    }
    let mut out = [0, 1, 2, 3].map(|k| c[k] + d[k] * a[0]);
    let alpha = out[3].clamp(0.0, 1.0);
    out[3] = alpha;
    for v in out.iter_mut().take(3) {
        *v = if a[2] > 0.5 {
            v.clamp(0.0, alpha)
        } else {
            v.max(0.0)
        };
    }
    out
}

impl NodeKind for Sharpen {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn input_region(&self, cx: &NodeCx, _input: usize, region: IRect) -> IRect {
        let (sigma, _, _) = setup(cx);
        if sigma < MIN_SIGMA {
            region
        } else {
            region.expand(half_width(sigma))
        }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let (sigma, amount, a) = setup(cx);
        super::empty_or(inputs.first().copied().flatten(), |input| {
            if sigma < MIN_SIGMA || amount <= 0.0 {
                return Out::Input(0);
            }
            let blurred = blur_cpu(input, region, sigma, true);
            Out::New(CpuImage::from_fn(region, |x, y| {
                combine(a, input.get(x, y), blurred.get(x, y))
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
        let (sigma, amount, a) = setup(cx);
        let Some(input) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        if sigma < MIN_SIGMA || amount <= 0.0 {
            return Ok(Out::Input(0));
        }
        let blurred = blur_gpu(gx, input, region, sigma, true)?;
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[Some(input), Some(&blurred)],
            region,
            Dispatch {
                params: &[a],
                ..Default::default()
            },
        )?))
    }
}
