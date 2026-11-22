//! renders documents back into pixels on the processor: merging layers, and picking
//! colours or areas from what every layer shows.
//!
//! one engine is shared by every document. its cache is keyed by content, so a second
//! magic wand click on an unchanged picture reuses the first render.

use std::sync::OnceLock;

use vellum_gpu::engine::RenderRequest;
use vellum_gpu::{BackendChoice, Engine, EngineOptions, IRect, Precision, render_tiled};

use crate::compose::{Preview, build_raw};
use crate::document::ImageDocument;
use crate::raster::{Depth, Raster};
use crate::selection::ColorBuf;

pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        Engine::new(EngineOptions {
            use_gpu: false,
            cpu_cache_bytes: 256 << 20,
            ..Default::default()
        })
    })
}

/// what the document's layers show over `rect`, as premultiplied document values, handed
/// over a tile at a time
fn render(
    doc: &ImageDocument,
    rect: IRect,
    mut sink: impl FnMut(IRect, &[[f32; 4]]),
) -> Result<(), String> {
    if rect.is_empty() {
        return Ok(());
    }
    let (graph, node) = build_raw(doc, &Preview::default());
    let req = RenderRequest::new(node, rect)
        .working(doc.working())
        .precision(Precision::F32)
        .backend(BackendChoice::Cpu);
    render_tiled(engine(), &graph, &req, 1024, |r, img| {
        sink(r, &img.data);
        Ok(())
    })
    .map_err(|err| format!("The layers could not be combined: {err}"))
}

/// renders `node` of `graph` over `rect` into a colour raster at `depth`, straight alpha,
/// in the working space the document blends in
pub fn node_to_raster(
    graph: &vellum_gpu::Graph,
    node: vellum_gpu::NodeId,
    rect: IRect,
    depth: Depth,
    working: vellum_gpu::WorkingSpace,
) -> Result<Raster, String> {
    let mut out = Raster::rgba(depth);
    if rect.is_empty() {
        return Ok(out);
    }
    let req = RenderRequest::new(node, rect)
        .working(working)
        .precision(Precision::F32)
        .backend(BackendChoice::Cpu);
    render_tiled(engine(), graph, &req, 1024, |r, img| {
        let straight: Vec<f32> = img
            .data
            .iter()
            .flat_map(|p| {
                let a = p[3];
                if a > 0.0 {
                    [p[0] / a, p[1] / a, p[2] / a, a]
                } else {
                    [0.0; 4]
                }
            })
            .collect();
        out.write(r, &straight);
        Ok(())
    })
    .map_err(|err| format!("The pixels could not be rendered: {err}"))?;
    Ok(out)
}

/// every layer's pixels combined, as colours to select or sample from
pub fn colors(doc: &ImageDocument, rect: IRect) -> Result<ColorBuf, String> {
    let mut out = ColorBuf {
        rect,
        data: vec![[0; 4]; rect.area() as usize],
    };
    render(doc, rect, |r, px| {
        let part = ColorBuf::from_premultiplied(r, px);
        for y in 0..r.h as usize {
            let dst = (r.y - rect.y) as usize + y;
            let start = dst * rect.w as usize + (r.x - rect.x) as usize;
            out.data[start..start + r.w as usize]
                .copy_from_slice(&part.data[y * r.w as usize..(y + 1) * r.w as usize]);
        }
    })?;
    Ok(out)
}

/// the layers combined into one colour raster at the document's depth
pub fn merged(doc: &ImageDocument, rect: IRect, depth: Depth) -> Result<Raster, String> {
    let mut out = Raster::rgba(depth);
    render(doc, rect, |r, px| {
        let straight: Vec<f32> = px
            .iter()
            .flat_map(|p| {
                let a = p[3];
                if a > 0.0 {
                    [p[0] / a, p[1] / a, p[2] / a, a]
                } else {
                    [0.0; 4]
                }
            })
            .collect();
        out.write(r, &straight);
    })?;
    Ok(out)
}

/// a small picture of the document, or of one layer, or of one layer's mask, at most
/// `max` pixels on its long side: straight srgb rgba8 with its width and height
pub fn thumbnail(
    doc: &ImageDocument,
    only: Option<crate::document::LayerId>,
    mask: bool,
    max: u32,
) -> Result<(u32, u32, Vec<u8>), String> {
    let canvas = doc.canvas();
    let mut level = 0;
    while level < 16 && canvas.at_level(level).w.max(canvas.at_level(level).h) > max.max(1) {
        level += 1;
    }
    let rect = canvas.at_level(level);
    if mask {
        let raster = only
            .and_then(|id| doc.layer(id))
            .and_then(|l| l.mask.as_ref())
            .map(|m| m.raster.clone())
            .ok_or("That layer has no mask.")?;
        let fill = (raster.fill().clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut out = vec![fill; rect.area() as usize * 4];
        out.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        if let Some(px) = raster.read_level(rect, level) {
            for y in px.rect.y..px.rect.bottom() {
                for x in px.rect.x..px.rect.right() {
                    let i = ((y - px.rect.y) as u32 * px.rect.w + (x - px.rect.x) as u32) as usize;
                    let v = (px.get(i)[0].clamp(0.0, 1.0) * 255.0).round() as u8;
                    let o = (((y - rect.y) as u32 * rect.w + (x - rect.x) as u32) * 4) as usize;
                    out[o..o + 3].fill(v);
                }
            }
        }
        return Ok((rect.w, rect.h, out));
    }
    let mut view = doc.clone();
    view.quick_mask = false;
    if let Some(id) = only {
        let mut layer = doc.layer(id).ok_or("That layer is gone.")?.clone();
        layer.visible = true;
        layer.opacity = 1.0;
        layer.fill = 1.0;
        layer.blend = vellum_gpu::BlendMode::Normal;
        layer.clipped = false;
        layer.mask = None;
        view.layers = vec![layer];
    }
    let (graph, node) = crate::compose::build(&view, &Preview::default());
    let req = RenderRequest::new(node, rect)
        .level(level)
        .working(doc.working())
        .precision(Precision::F32)
        .backend(BackendChoice::Cpu);
    let img = engine()
        .render_image(&graph, &req)
        .map_err(|err| format!("The preview could not be drawn: {err}"))?;
    let linear = doc.depth == Depth::F32;
    let mut out = Vec::with_capacity(img.data.len() * 4);
    for p in &img.data {
        let a = p[3].clamp(0.0, 1.0);
        let c = |v: f32| {
            let s = if a > 0.0 { v / a } else { 0.0 };
            let s = if linear {
                vellum_gpu::Transfer::Srgb.encode(s)
            } else {
                s
            };
            (s.clamp(0.0, 1.0) * 255.0).round() as u8
        };
        out.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), (a * 255.0).round() as u8]);
    }
    Ok((rect.w, rect.h, out))
}

/// a thumbnail as png bytes, for the `.vellum` file
pub fn thumbnail_png(doc: &ImageDocument, max: u32) -> Result<Vec<u8>, String> {
    let (w, h, px) = thumbnail(doc, None, false, max)?;
    let img = image::RgbaImage::from_raw(w, h, px).ok_or("The preview has the wrong size.")?;
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .map_err(|err| format!("The preview could not be written: {err}"))?;
    Ok(bytes)
}
