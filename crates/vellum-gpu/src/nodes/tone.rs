//! adjustments that look at a neighbourhood: shadows and highlights, and hdr toning. both
//! blur a copy of the picture to know how bright each area is, then change each pixel by
//! its surroundings, like sharpen chaining the gaussian blur's passes.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NodeCategory, NodeCx, NodeInfo, NodeKind, ONE_INPUT, Out};
use crate::image::CpuImage;
use crate::params::ParamSpec;
use crate::rect::IRect;

use super::colorfn::{luma, mix, smoothstep};
use super::gaussian_blur::{blur_cpu, blur_gpu, weights};
use super::unpremultiply;

fn reach(sigma: f32) -> i32 {
    ((weights(sigma).len() - 1) / 2) as i32
}

/// renders a two input kernel (the picture and its blur) on either backend
pub(crate) trait LocalTone: Send + Sync + 'static {
    fn info(&self) -> &'static NodeInfo;
    fn kernel(&self) -> &'static Kernel;
    /// blur sigma in level pixels and the packed parameters, none for no change
    fn setup(&self, cx: &NodeCx) -> Option<(f32, Vec<[f32; 4]>)>;
    fn pixel(&self, p: &[[f32; 4]], cx: &NodeCx, c: [f32; 4], blurred: [f32; 4]) -> [f32; 4];
}

pub(crate) struct Local<T>(pub T);

impl<T: LocalTone> NodeKind for Local<T> {
    fn info(&self) -> &'static NodeInfo {
        self.0.info()
    }

    fn input_region(&self, cx: &NodeCx, _input: usize, region: IRect) -> IRect {
        match self.0.setup(cx) {
            Some((s, _)) => region.expand(reach(s)),
            None => region,
        }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let Some(input) = inputs.first().copied().flatten() else {
            return Out::Empty;
        };
        let Some((sigma, p)) = self.0.setup(cx) else {
            return Out::Input(0);
        };
        let blurred = blur_cpu(input, region, sigma, true);
        Out::New(CpuImage::from_fn(region, |x, y| {
            let c = input.get(x, y);
            if c[3] <= 0.0 {
                return [0.0; 4];
            }
            let out = self
                .0
                .pixel(&p, cx, unpremultiply(c), unpremultiply(blurred.get(x, y)));
            let mut out = out;
            if cx.clamp() {
                for v in out.iter_mut().take(3) {
                    *v = v.clamp(0.0, 1.0);
                }
            }
            super::premultiply(out)
        }))
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
        let Some((sigma, p)) = self.0.setup(cx) else {
            return Ok(Out::Input(0));
        };
        let region = region.intersect(&input.rect);
        let blurred = blur_gpu(gx, input, region, sigma, true)?;
        Ok(Out::New(gx.dispatch(
            self.0.kernel(),
            &[Some(input), Some(&blurred)],
            region,
            Dispatch {
                params: &p,
                ..Default::default()
            },
        )?))
    }
}

/// wraps the kernel body's `fn tone(c, b) -> vec4` (straight colour and straight blur)
#[macro_export]
macro_rules! tone_kernel {
    ($name:literal, $body:literal) => {
        $crate::gpu::Kernel {
            name: $name,
            body: concat!(
                $body,
                r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    let c = unpremultiply(load0(p));
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    var out = tone(c, unpremultiply(load1(p)));
    if (clamps()) { out = vec4<f32>(clamp01(out.rgb), out.a); }
    return premultiply(out);
}
"#
            ),
        }
    };
}

// ---- shadows and highlights ----

pub struct ShadowsHighlights;

static SHADOWS: NodeInfo = NodeInfo {
    id: "effect.shadows_highlights",
    title: "Shadows/Highlights",
    category: NodeCategory::Color,
    description: "Brings out detail in dark areas and tames bright ones.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::percent("shadows", "Shadows amount", 0.0, 100.0, 35.0),
        ParamSpec::percent("shadows_tone", "Shadows tone", 1.0, 100.0, 50.0),
        ParamSpec::percent("highlights", "Highlights amount", 0.0, 100.0, 0.0),
        ParamSpec::percent("highlights_tone", "Highlights tone", 1.0, 100.0, 50.0),
        ParamSpec::pixels("radius", "Radius", 500.0, 30.0),
        ParamSpec::float("color", "Color", -100.0, 100.0, 20.0),
    ],
};

const SHADOWS_KERNEL: Kernel = tone_kernel!(
    "shadows_highlights",
    r#"
fn tone(c: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let a = param(0u);
    let l = clamp(luma(b.rgb), 0.0, 1.0);
    let ms = smoothstep(0.0, 1.0, clamp(1.0 - l / a.y, 0.0, 1.0));
    let mh = smoothstep(0.0, 1.0, clamp((l - (1.0 - a.w)) / a.w, 0.0, 1.0));
    var rgb = max(c.rgb, vec3<f32>(0.0));
    rgb = pow(rgb, vec3<f32>(1.0 / (1.0 + 2.0 * a.x * ms)));
    rgb = pow(rgb, vec3<f32>(1.0 + 2.0 * a.z * mh));
    let y = luma(rgb);
    rgb = vec3<f32>(y) + (rgb - vec3<f32>(y)) * (1.0 + param(1u).x * (ms * a.x + mh * a.z));
    return vec4<f32>(rgb, c.a);
}
"#
);

impl LocalTone for ShadowsHighlights {
    fn info(&self) -> &'static NodeInfo {
        &SHADOWS
    }
    fn kernel(&self) -> &'static Kernel {
        &SHADOWS_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<(f32, Vec<[f32; 4]>)> {
        let p = cx.params;
        let (s, h) = (p.float("shadows") / 100.0, p.float("highlights") / 100.0);
        if s == 0.0 && h == 0.0 {
            return None;
        }
        let sigma = (p.float("radius") * cx.scale() * 0.5).max(0.3);
        Some((
            sigma,
            vec![
                [
                    s,
                    p.float("shadows_tone") / 100.0,
                    h,
                    p.float("highlights_tone") / 100.0,
                ],
                [p.float("color") / 100.0, 0.0, 0.0, 0.0],
            ],
        ))
    }
    fn pixel(&self, p: &[[f32; 4]], _cx: &NodeCx, c: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        let a = p[0];
        let l = luma([b[0], b[1], b[2]]).clamp(0.0, 1.0);
        let ms = smoothstep(0.0, 1.0, (1.0 - l / a[1]).clamp(0.0, 1.0));
        let mh = smoothstep(0.0, 1.0, ((l - (1.0 - a[3])) / a[3]).clamp(0.0, 1.0));
        let mut rgb = [c[0], c[1], c[2]].map(|v| v.max(0.0));
        rgb = rgb.map(|v| v.powf(1.0 / (1.0 + 2.0 * a[0] * ms)));
        rgb = rgb.map(|v| v.powf(1.0 + 2.0 * a[2] * mh));
        let y = luma(rgb);
        let k = 1.0 + p[1][0] * (ms * a[0] + mh * a[2]);
        rgb = rgb.map(|v| y + (v - y) * k);
        [rgb[0], rgb[1], rgb[2], c[3]]
    }
}

pub fn shadows_highlights() -> impl NodeKind {
    Local(ShadowsHighlights)
}

// ---- hdr toning ----

pub struct HdrToning;

static HDR: NodeInfo = NodeInfo {
    id: "effect.hdr_toning",
    title: "HDR Toning",
    category: NodeCategory::Color,
    description: "Squeezes a wide range of tones into view while keeping local detail.",
    inputs: ONE_INPUT,
    params: &[
        ParamSpec::pixels("radius", "Radius", 500.0, 40.0),
        ParamSpec::float("strength", "Strength", 0.0, 1.0, 0.5),
        ParamSpec::float("exposure", "Exposure", -5.0, 5.0, 0.0),
        ParamSpec::float("gamma", "Gamma", 0.1, 3.0, 1.0),
        ParamSpec::percent("detail", "Detail", -100.0, 300.0, 30.0),
        ParamSpec::percent("saturation", "Saturation", -100.0, 100.0, 20.0),
    ],
};

/// compresses the blurred base towards the middle, keeps the detail above it, then
/// carries colour along by ratio
const HDR_KERNEL: Kernel = tone_kernel!(
    "hdr_toning",
    r#"
fn tone(c: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let a = param(0u);
    let k = param(1u);
    let y = max(luma(c.rgb), 1e-4);
    let base = max(luma(b.rgb), 1e-4);
    let squeezed = mix(base, pow(base, 1.0 - a.x * 0.6) * pow(0.5, a.x * 0.6), a.x);
    let detail = pow(y / base, 1.0 + a.w);
    var ny = squeezed * detail * a.y;
    ny = pow(max(ny, 0.0), a.z);
    var rgb = c.rgb * (ny / y);
    let l = luma(rgb);
    rgb = vec3<f32>(l) + (rgb - vec3<f32>(l)) * (1.0 + k.x);
    return vec4<f32>(rgb, c.a);
}
"#
);

impl LocalTone for HdrToning {
    fn info(&self) -> &'static NodeInfo {
        &HDR
    }
    fn kernel(&self) -> &'static Kernel {
        &HDR_KERNEL
    }
    fn setup(&self, cx: &NodeCx) -> Option<(f32, Vec<[f32; 4]>)> {
        let p = cx.params;
        let sigma = (p.float("radius") * cx.scale() * 0.5).max(0.3);
        Some((
            sigma,
            vec![
                [
                    p.float("strength"),
                    2f32.powf(p.float("exposure")),
                    1.0 / p.float("gamma"),
                    p.float("detail") / 100.0,
                ],
                [p.float("saturation") / 100.0, 0.0, 0.0, 0.0],
            ],
        ))
    }
    fn pixel(&self, p: &[[f32; 4]], _cx: &NodeCx, c: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        let (a, k) = (p[0], p[1]);
        let y = luma([c[0], c[1], c[2]]).max(1e-4);
        let base = luma([b[0], b[1], b[2]]).max(1e-4);
        let squeezed = mix(
            base,
            base.powf(1.0 - a[0] * 0.6) * 0.5f32.powf(a[0] * 0.6),
            a[0],
        );
        let detail = (y / base).powf(1.0 + a[3]);
        let ny = (squeezed * detail * a[1]).max(0.0).powf(a[2]);
        let mut rgb = [c[0], c[1], c[2]].map(|v| v * (ny / y));
        let l = luma(rgb);
        rgb = rgb.map(|v| l + (v - l) * (1.0 + k[0]));
        [rgb[0], rgb[1], rgb[2], c[3]]
    }
}

pub fn hdr_toning() -> impl NodeKind {
    Local(HdrToning)
}
