//! pixels from outside the graph. the node reads raw pixels from its [`ImageSource`] and
//! converts them to premultiplied working space values with the ingest kernel.

use std::hash::Hasher;

use crate::color::ColorTransform;
use crate::gpu::{Dispatch, GpuError, GpuExec, GpuImage, Kernel};
use crate::graph::{NO_INPUTS, NodeCategory, NodeCx, NodeInfo, NodeKind, Out};
use crate::image::{AlphaMode, CpuImage, SourcePixels};
use crate::rect::{Bounds, IRect};

pub struct ImageNode;

static INFO: NodeInfo = NodeInfo {
    id: "source.image",
    title: "Image",
    category: NodeCategory::Source,
    description: "Pixels from a layer, a photo or a video frame.",
    inputs: NO_INPUTS,
    params: &[],
};

/// params[0].x is the alpha mode, the colour steps start at params[1]
pub const INGEST: Kernel = Kernel {
    name: "ingest",
    body: r#"
fn kernel(p: vec2<i32>) -> vec4<f32> {
    var c = load0(p);
    let mode = u32(param(0u).x);
    if (mode == 1u) { c = unpremultiply(c); }
    if (mode == 2u) { c.a = 1.0; }
    let rgb = color_steps(1u, c.rgb);
    return premultiply(vec4<f32>(rgb, c.a));
}
"#,
};

fn alpha_id(a: AlphaMode) -> f32 {
    match a {
        AlphaMode::Straight => 0.0,
        AlphaMode::Premultiplied => 1.0,
        AlphaMode::Opaque => 2.0,
    }
}

/// converts raw pixels to working space on the cpu, the twin of [`INGEST`]
pub fn ingest_cpu(px: &SourcePixels, transform: &ColorTransform) -> CpuImage {
    let mut out = CpuImage::transparent(px.rect);
    let w = px.rect.w as usize;
    super::par_rows(&mut out, |y, row| {
        let base = (y - px.rect.y) as usize * w;
        for (i, dst) in row.iter_mut().enumerate() {
            let mut c = px.get(base + i);
            if px.alpha == AlphaMode::Premultiplied {
                c = super::unpremultiply(c);
            }
            let rgb = transform.apply([c[0], c[1], c[2]]);
            *dst = super::premultiply([rgb[0], rgb[1], rgb[2], c[3]]);
        }
    });
    out
}

/// converts raw pixels to working space on the gpu
pub fn ingest_gpu(
    gx: &mut GpuExec,
    px: &SourcePixels,
    transform: &ColorTransform,
) -> Result<GpuImage, GpuError> {
    let raw = gx.ctx.upload_source(px)?;
    let mut params = vec![[alpha_id(px.alpha), 0.0, 0.0, 0.0]];
    transform.pack(&mut params);
    gx.dispatch(
        &INGEST,
        &[Some(&raw)],
        px.rect,
        Dispatch {
            params: &params,
            lut: transform.lut().map(|l| &**l),
            ..Default::default()
        },
    )
}

impl NodeKind for ImageNode {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }

    fn bounds(&self, cx: &NodeCx, _inputs: &[Option<Bounds>]) -> Bounds {
        match cx.source() {
            Some(s) => Bounds::of(s.bounds()).at_level(cx.level),
            None => Bounds::Empty,
        }
    }

    fn key(&self, cx: &NodeCx, region: IRect, h: &mut dyn Hasher) {
        if let Some(s) = cx.source() {
            s.key(region, cx.level, h);
            h.write_u64(s.to_working(cx.working).key());
        }
    }

    fn cpu(&self, cx: &NodeCx, _inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage> {
        let Some(source) = cx.source() else {
            return Out::Empty;
        };
        match source.read(region, cx.level) {
            Some(px) if !px.is_empty() => Out::New(ingest_cpu(&px, &source.to_working(cx.working))),
            _ => Out::Empty,
        }
    }

    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        _inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError> {
        let Some(source) = cx.source() else {
            return Ok(Out::Empty);
        };
        match source.read(region, cx.level) {
            Some(px) if !px.is_empty() => Ok(Out::New(ingest_gpu(
                gx,
                &px,
                &source.to_working(cx.working),
            )?)),
            _ => Ok(Out::Empty),
        }
    }
}
