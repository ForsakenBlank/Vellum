//! hides parts of an image using another image as a mask, like a layer mask or a clipping
//! base. white (or opaque) shows, black (or transparent) hides.

use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{InputSpec, NodeCategory, NodeCx, NodeInfo, NodeKind, Out, input_bounds};
use crate::image::CpuImage;
use crate::params::ParamSpec;
use crate::rect::{Bounds, IRect};

use super::blend::MASK_CHANNELS;

pub struct MaskNode;

static INFO: NodeInfo = NodeInfo {
    id: "mask",
    title: "Mask",
    category: NodeCategory::Mask,
    description: "Shows an image only where the mask is light.",
    inputs: &[
        InputSpec {
            id: "image",
            label: "Image",
        },
        InputSpec {
            id: "mask",
            label: "Mask",
        },
    ],
    params: &[
        ParamSpec::choice("channel", "Mask from", MASK_CHANNELS, "luminance"),
        ParamSpec::boolean("invert", "Invert", false),
        ParamSpec::percent("density", "Density", 0.0, 100.0, 100.0),
        ParamSpec::float("outside", "Outside the mask", 0.0, 1.0, 0.0),
    ],
};

const KERNEL: Kernel = Kernel {
    name: "mask",
    body: r#"
fn shape(m_in: f32) -> f32 {
    var m = m_in;
    if (param(0u).y > 0.5) { m = 1.0 - m; }
    return 1.0 - param(0u).z * (1.0 - m);
}

fn kernel(p: vec2<i32>) -> vec4<f32> {
    let r = F.in_rect[1];
    var m: f32;
    if (!inside(r, p)) {
        m = param(0u).w;
    } else {
        let c = textureLoad(in1, p - r.xy, 0);
        if (u32(param(0u).x) == 1u) { m = c.a; } else { m = clamp(0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b, 0.0, 1.0); }
    }
    return load0(p) * shape(m);
}
"#,
};

struct Setup {
    alpha_channel: bool,
    invert: bool,
    density: f32,
    outside: f32,
}

impl Setup {
    fn of(cx: &NodeCx) -> Self {
        let p = cx.params;
        Self {
            alpha_channel: p.choice_index("channel") == 1,
            invert: p.bool("invert"),
            density: p.float("density") / 100.0,
            outside: p.float("outside"),
        }
    }

    fn shape(&self, m: f32) -> f32 {
        let m = if self.invert { 1.0 - m } else { m };
        1.0 - self.density * (1.0 - m)
    }

    fn read(&self, mask: &CpuImage, x: i32, y: i32) -> f32 {
        let m = if mask.rect.contains(x, y) {
            let c = mask.get(x, y);
            if self.alpha_channel {
                c[3]
            } else {
                (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).clamp(0.0, 1.0)
            }
        } else {
            self.outside
        };
        self.shape(m)
    }
}

impl NodeKind for MaskNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn bounds(&self, cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        let s = Setup::of(cx);
        let image = input_bounds(inputs, 0);
        match inputs.get(1).copied().flatten() {
            // nothing outside the mask survives, so the result ends where the mask does
            Some(mask) if s.shape(s.outside) <= 0.0 => image.intersect(mask),
            _ => image,
        }
    }

    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let s = Setup::of(cx);
        let Some(image) = inputs.first().copied().flatten() else {
            return Out::Empty;
        };
        let mask = inputs.get(1).copied().flatten();
        match mask {
            None if s.shape(s.outside) >= 1.0 => Out::Input(0),
            None => {
                let m = s.shape(s.outside);
                Out::New(super::map_pixels(image, region, |c, _, _| c.map(|v| v * m)))
            }
            Some(mask) => Out::New(super::map_pixels(image, region, |c, x, y| {
                let m = s.read(mask, x, y);
                c.map(|v| v * m)
            })),
        }
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let s = Setup::of(cx);
        let Some(image) = inputs.first().copied().flatten() else {
            return Ok(Out::Empty);
        };
        let mask = inputs.get(1).copied().flatten();
        if mask.is_none() && s.shape(s.outside) >= 1.0 {
            return Ok(Out::Input(0));
        }
        let params = [[
            f32::from(u8::from(s.alpha_channel)),
            f32::from(u8::from(s.invert)),
            s.density,
            s.outside,
        ]];
        let region = region.intersect(&image.rect);
        Ok(Out::New(gx.dispatch(
            &KERNEL,
            &[Some(image), mask],
            region,
            Dispatch {
                params: &params,
                ..Default::default()
            },
        )?))
    }
}
