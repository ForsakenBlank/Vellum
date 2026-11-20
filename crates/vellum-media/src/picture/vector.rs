//! svg rendered by resvg at whatever size is asked for.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use ::image::RgbaImage;
use resvg::{tiny_skia, usvg};

use super::{DecodeOptions, Picture, PictureFormat};
use crate::error::{MediaError, MediaResult};

/// the largest edge an svg is rendered at, to keep a typo like 100000 from using gigabytes
pub const MAX_SVG_EDGE: u32 = 16_384;

/// system fonts for text in svgs, loaded once since it scans every font on the machine
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

fn parse(path: &Path) -> MediaResult<usvg::Tree> {
    let data = std::fs::read(path).map_err(|err| MediaError::io(path, err))?;
    let options = usvg::Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        fontdb: fonts(),
        ..Default::default()
    };
    usvg::Tree::from_data(&data, &options).map_err(|err| MediaError::decode(path, err))
}

pub fn decode(path: &Path, options: DecodeOptions) -> MediaResult<Picture> {
    let tree = parse(path)?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    let longest = w.max(h);
    let target = options
        .size
        .map_or(longest, |s| s as f32)
        .min(MAX_SVG_EDGE as f32);
    let scale = if longest > 0.0 { target / longest } else { 1.0 };
    let out_w = (w * scale).round().max(1.0) as u32;
    let out_h = (h * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(out_w, out_h)
        .ok_or_else(|| MediaError::decode(path, "the drawing has no size"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia keeps colours premultiplied by alpha, images everywhere else do not
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|px| {
            let c = px.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    let image = RgbaImage::from_raw(out_w, out_h, rgba)
        .ok_or_else(|| MediaError::decode(path, "the rendered image had the wrong size"))?;
    let full = (w.round().max(1.0) as u32, h.round().max(1.0) as u32);
    Ok(Picture::still(PictureFormat::Svg, full.0, full.1, image))
}

pub fn dimensions(path: &Path) -> MediaResult<(u32, u32)> {
    let size = parse(path)?.size();
    Ok((
        size.width().round().max(1.0) as u32,
        size.height().round().max(1.0) as u32,
    ))
}
