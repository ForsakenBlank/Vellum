//! gaussian blur, the pattern for spatial effects: it asks for more of its input than it
//! writes (`input_region`), scales its radius with the zoom level, and runs as two passes.
//! like photoshop, the radius is the standard deviation in pixels.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out, input_bounds};
use crate::image::CpuImage;
use crate::params::{ChoiceOption, ParamSpec};
use crate::rect::{Bounds, IRect};

pub struct GaussianBlur;

pub const EDGES: &[ChoiceOption] = &[
    ChoiceOption {
        id: "repeat",
        label: "Repeat edge pixels",
    },
    ChoiceOption {
        id: "transparent",
        label: "Fade to transparent",
    },
];

static INFO: NodeInfo = NodeInfo {
    id: "effect.gaussian_blur",
    title: "Gaussian blur",
    category: NodeCategory::Effect,
    description: "Softens the image evenly in every direction.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("radius", "Radius", 1000.0, 4.0),
        ParamSpec::choice("edges", "Edges", EDGES, "repeat"),
    ],
};

/// params[0] = (half width, vertical, repeat edges)
const KERNEL: Kernel = Kernel {
    name: "gaussian_blur",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let k = i32(param(0u).x);
    var dir = vec2<i32>(1, 0);
    if (param(0u).y > 0.5) { dir = vec2<i32>(0, 1); }
    let repeat = param(0u).z > 0.5;
    var acc = vec4<f32>(0.0);
    for (var i = -k; i <= k; i = i + 1) {
        let q = p + dir * i;
        var c: vec4<f32>;
        if (repeat) { c = load0_clamped(q); } else { c = load0(q); }
        acc = acc + c * data[u32(i + k)];
    }
    return acc;
}
"#,
};

/// below this the blur is invisible and the input passes through
const MIN_SIGMA: f32 = 0.25;

/// normalised weights for a gaussian with this sigma, from -k to k
pub fn weights(sigma: f32) -> Vec<f32> {
    let k = (3.0 * sigma).ceil().max(1.0) as i32;
    let mut w: Vec<f32> = (-k..=k)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = w.iter().sum();
    w.iter_mut().for_each(|v| *v /= sum);
    w
}

fn half_width(sigma: f32) -> i32 {
    (3.0 * sigma).ceil().max(1.0) as i32
}

fn sigma(cx: &NodeCx) -> f32 {
    cx.params.float("radius") * cx.scale()
}

fn repeat(cx: &NodeCx) -> bool {
    cx.params.choice_index("edges") == 0
}

/// blurs `input` into `region` on the cpu
pub fn blur_cpu(input: &CpuImage, region: IRect, sigma: f32, repeat: bool) -> CpuImage {
    let w = weights(sigma);
    let k = half_width(sigma);
    let read = |img: &CpuImage, x: i32, y: i32| {
        if repeat {
            img.get_clamped(x, y)
        } else {
            img.get(x, y)
        }
    };
    let rows = IRect::new(region.x, region.y - k, region.w, region.h + 2 * k as u32);
    let pass1 = CpuImage::from_fn(rows, |x, y| {
        let mut acc = [0f32; 4];
        for (i, wi) in w.iter().enumerate() {
            let c = read(input, x + i as i32 - k, y);
            for c_k in 0..4 {
                acc[c_k] += c[c_k] * wi;
            }
        }
        acc
    });
    CpuImage::from_fn(region, |x, y| {
        let mut acc = [0f32; 4];
        for (i, wi) in w.iter().enumerate() {
            let c = read(&pass1, x, y + i as i32 - k);
            for c_k in 0..4 {
                acc[c_k] += c[c_k] * wi;
            }
        }
        acc
    })
}

/// blurs `input` into `region` on the gpu
pub fn blur_gpu(
    gx: &mut GpuExec,
    input: &GpuImage,
    region: IRect,
    sigma: f32,
    repeat: bool,
) -> Result<GpuImage, GpuError> {
    let w = weights(sigma);
    let k = half_width(sigma);
    let edge = f32::from(u8::from(repeat));
    let rows = IRect::new(region.x, region.y - k, region.w, region.h + 2 * k as u32);
    let pass1 = gx.dispatch(
        &KERNEL,
        &[Some(input)],
        rows,
        Dispatch {
            params: &[[k as f32, 0.0, edge, 0.0]],
            data: Some(&w),
            ..Default::default()
        },
    )?;
    gx.dispatch(
        &KERNEL,
        &[Some(&pass1)],
        region,
        Dispatch {
            params: &[[k as f32, 1.0, edge, 0.0]],
            data: Some(&w),
            ..Default::default()
        },
    )
}

impl NodeKind for GaussianBlur {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn bounds(&self, cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        let b = input_bounds(inputs, 0);
        let s = sigma(cx);
        if s < MIN_SIGMA || repeat(cx) {
            b
        } else {
            b.expand(half_width(s))
        }
    }

    fn input_region(&self, cx: &NodeCx, _input: usize, region: IRect) -> IRect {
        let s = sigma(cx);
        if s < MIN_SIGMA {
            region
        } else {
            region.expand(half_width(s))
        }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let s = sigma(cx);
        super::empty_or(inputs.first().copied().flatten(), |input| {
            if s < MIN_SIGMA {
                Out::Input(0)
            } else {
                Out::New(blur_cpu(input, region, s, repeat(cx)))
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
        let s = sigma(cx);
        let Some(input) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        if s < MIN_SIGMA {
            return Ok(Out::Input(0));
        }
        Ok(Out::New(blur_gpu(gx, input, region, s, repeat(cx))?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_are_normalised_and_symmetric() {
        let w = weights(2.0);
        assert_eq!(w.len(), 13);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!((w[0] - w[12]).abs() < 1e-9);
        assert!(w[6] > w[5]);
    }

    #[test]
    fn a_flat_image_stays_flat_with_repeated_edges() {
        let img = CpuImage::filled(IRect::new(0, 0, 32, 32), [0.5, 0.25, 0.125, 1.0]);
        let out = blur_cpu(&img, IRect::new(0, 0, 32, 32), 3.0, true);
        for px in &out.data {
            assert!((px[0] - 0.5).abs() < 1e-5 && (px[3] - 1.0).abs() < 1e-5);
        }
        // fading edges darken the border instead
        let out = blur_cpu(&img, IRect::new(0, 0, 32, 32), 3.0, false);
        assert!(out.get(0, 0)[3] < 0.5);
        assert!((out.get(16, 16)[3] - 1.0).abs() < 1e-4);
    }
}
