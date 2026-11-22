//! exporting a document as a png, jpeg, webp or avif picture, optionally fitted under a
//! size the same way phase 3's export to x mb fits pictures (`vellum_export::size::image`).
//!
//! 16 and 32 bit documents keep 16 bits per channel in png. the document's colour profile
//! is written into png, jpeg and webp files; avif files are converted to srgb instead,
//! because ffmpeg has no way to embed a profile in them.

use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use image::{Rgb, RgbImage, RgbaImage};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_export::size::TargetSize;
use vellum_export::size::budget;
use vellum_export::size::image::{Saved, jpeg, optimise, squeeze, via_ffmpeg};
use vellum_export::{Env, ToolResult};
use vellum_gpu::engine::RenderRequest;
use vellum_gpu::{BackendChoice, Precision, Transfer, WorkingSpace, render_tiled};
use vellum_media::Control;

use crate::compose::{Preview, build_raw};
use crate::document::ImageDocument;
use crate::raster::Depth;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExportFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
    Avif,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Webp => "webp",
            ExportFormat::Avif => "avif",
        }
    }

    fn saved(self) -> Saved {
        match self {
            ExportFormat::Png => Saved::Png,
            ExportFormat::Jpeg => Saved::Jpeg,
            ExportFormat::Webp => Saved::Webp,
            ExportFormat::Avif => Saved::Avif,
        }
    }
}

/// what the export dialog asks for
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// 1 to 100, for jpeg, webp and avif
    pub quality: u8,
    /// fit under this size, lowering the quality first and then the size
    pub target: Option<TargetSize>,
    /// straight srgb shown behind see through pixels in formats without transparency
    pub background: [f32; 3],
    /// keep the colour profile in the file
    pub embed_profile: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Png,
            quality: 90,
            target: None,
            background: [1.0, 1.0, 1.0],
            embed_profile: true,
        }
    }
}

/// the flattened picture, straight srgb or document values
enum Pixels {
    Eight(RgbaImage),
    Sixteen(u32, u32, Vec<u16>),
}

fn failed(err: impl std::fmt::Display) -> String {
    err.to_string()
}

/// every visible layer combined into straight pixels, a tile at a time
fn flatten(
    doc: &ImageDocument,
    sixteen: bool,
    to_srgb: bool,
    control: &dyn Control,
) -> Result<Pixels, String> {
    let canvas = doc.canvas();
    let (graph, node) = build_raw(doc, &Preview::default());
    let req = RenderRequest::new(node, canvas)
        .working(doc.working())
        .precision(Precision::F32)
        .backend(BackendChoice::Cpu);
    let (w, h) = (doc.width as usize, doc.height as usize);
    let linear = doc.depth == Depth::F32;
    let convert = to_srgb.then(|| doc.color.to_working(WorkingSpace::SRGB));
    let mut out8 = if sixteen {
        Vec::new()
    } else {
        vec![0u8; w * h * 4]
    };
    let mut out16 = if sixteen {
        vec![0u16; w * h * 4]
    } else {
        Vec::new()
    };
    let total = canvas.area() as f32;
    let mut done = 0f32;
    render_tiled(crate::flatten::engine(), &graph, &req, 1024, |r, img| {
        control
            .checkpoint()
            .map_err(|_| vellum_gpu::RenderError::Cancelled)?;
        for (i, p) in img.data.iter().enumerate() {
            let (x, y) = (
                r.x as usize + i % r.w as usize,
                r.y as usize + i / r.w as usize,
            );
            let a = p[3].clamp(0.0, 1.0);
            let mut c = if p[3] > 0.0 {
                [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
            } else {
                [0.0; 3]
            };
            if linear {
                c = c.map(|v| Transfer::Srgb.encode(v));
            }
            if let Some(t) = &convert {
                c = t.apply(c);
            }
            let o = (y * w + x) * 4;
            let px = [c[0], c[1], c[2], a];
            if sixteen {
                for (k, v) in px.iter().enumerate() {
                    out16[o + k] = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
                }
            } else {
                for (k, v) in px.iter().enumerate() {
                    out8[o + k] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
        done += r.area() as f32;
        control.progress(0.4 * done / total);
        Ok(())
    })
    .map_err(|e| format!("The picture could not be drawn for export: {e}"))?;
    if sixteen {
        return Ok(Pixels::Sixteen(doc.width, doc.height, out16));
    }
    RgbaImage::from_raw(doc.width, doc.height, out8)
        .map(Pixels::Eight)
        .ok_or_else(|| "The picture has the wrong size.".into())
}

/// a png at 8 or 16 bits, rgb when nothing is see through
fn png(w: u32, h: u32, values: &[u16], sixteen: bool) -> Result<Vec<u8>, String> {
    let opaque = values
        .chunks_exact(4)
        .all(|p| p[3] == if sixteen { 65535 } else { 255 });
    let mut data = Vec::with_capacity(values.len() * if sixteen { 2 } else { 1 });
    for p in values.chunks_exact(4) {
        for v in &p[..if opaque { 3 } else { 4 }] {
            if sixteen {
                data.extend_from_slice(&v.to_be_bytes());
            } else {
                data.push(*v as u8);
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(if opaque {
            png::ColorType::Rgb
        } else {
            png::ColorType::Rgba
        });
        enc.set_depth(if sixteen {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
        let mut writer = enc.write_header().map_err(failed)?;
        writer.write_image_data(&data).map_err(failed)?;
    }
    Ok(out)
}

/// see through pixels over a colour, for formats without transparency
fn over(img: &RgbaImage, bg: [f32; 3]) -> RgbImage {
    let bg = bg.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u16);
    RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        let a = u16::from(p[3]);
        let mix = |c: u8, b: u16| ((u16::from(c) * a + b * (255 - a) + 127) / 255) as u8;
        Rgb([mix(p[0], bg[0]), mix(p[1], bg[1]), mix(p[2], bg[2])])
    })
}

fn rgba_of(rgb: &RgbImage) -> RgbaImage {
    RgbaImage::from_fn(rgb.width(), rgb.height(), |x, y| {
        let p = rgb.get_pixel(x, y);
        image::Rgba([p[0], p[1], p[2], 255])
    })
}

/// writes `doc` to `path`. `margin` is the export to size safety margin in percent.
pub fn export(
    doc: &ImageDocument,
    path: &Path,
    options: &ExportOptions,
    env: &Env,
    margin: f64,
    control: &dyn Control,
) -> Result<ToolResult, String> {
    let format = options.format;
    let mut changes = Vec::new();
    let profile = doc.color.icc.as_ref().map(|b| b.to_vec());
    // avif cannot carry a profile here, and 32 bit documents are linear, so both go out
    // as plain srgb
    let to_srgb = profile.is_some() && format == ExportFormat::Avif;
    let embed = options.embed_profile && !to_srgb && doc.depth != Depth::F32;
    if to_srgb {
        changes
            .push("Converted to sRGB, since AVIF files here cannot carry a colour profile.".into());
    }
    let sixteen = format == ExportFormat::Png && options.target.is_none() && doc.depth != Depth::U8;
    if format == ExportFormat::Png && options.target.is_some() && doc.depth != Depth::U8 {
        changes.push("Saved at 8 bits per channel so it can fit the size.".into());
    }
    let pixels = flatten(doc, sixteen, to_srgb, control)?;
    control.checkpoint().map_err(failed)?;
    let quality = options.quality.clamp(1, 100);
    let mut target_bytes = None;
    let data = match pixels {
        Pixels::Sixteen(w, h, values) => {
            changes.push("16 bits per channel.".into());
            png(w, h, &values, true)?
        }
        Pixels::Eight(img) => {
            let flat = || rgba_of(&over(&img, options.background));
            let ready = match format {
                ExportFormat::Jpeg | ExportFormat::Avif => flat(),
                _ => img,
            };
            match options.target {
                Some(target) => {
                    let goal = budget::target_bytes(target.bytes(), margin);
                    target_bytes = Some(goal);
                    let span = vellum_media::control::Span {
                        inner: control,
                        start: 0.4,
                        end: 0.95,
                    };
                    let (data, mut how) =
                        squeeze(env, &ready, format.saved(), goal, &span).map_err(failed)?;
                    changes.append(&mut how);
                    data
                }
                None => encode(env, &ready, format, quality, &mut changes)?,
            }
        }
    };
    control.checkpoint().map_err(failed)?;
    let data = match (&profile, embed) {
        (Some(icc), true) => embed_profile(format, data, icc)?,
        _ => data,
    };
    vellum_export::write_atomic(path, &data).map_err(failed)?;
    control.progress(1.0);
    Ok(ToolResult {
        path: path.display().to_string(),
        source: doc.title.clone(),
        bytes: Some(data.len() as u64),
        target_bytes,
        changes,
        ..ToolResult::default()
    })
}

fn encode(
    env: &Env,
    img: &RgbaImage,
    format: ExportFormat,
    quality: u8,
    changes: &mut Vec<String>,
) -> Result<Vec<u8>, String> {
    match format {
        ExportFormat::Png => {
            let values: Vec<u16> = img.as_raw().iter().map(|v| u16::from(*v)).collect();
            let plain = png(img.width(), img.height(), &values, false)?;
            optimise(&plain).map_err(failed)
        }
        ExportFormat::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            changes.push(format!("Quality {quality} out of 100."));
            jpeg(&rgb, quality).map_err(failed)
        }
        ExportFormat::Webp | ExportFormat::Avif => {
            let ffmpeg = env.ffmpeg().map_err(failed)?;
            let temp = tempfile::tempdir().map_err(failed)?;
            let source = temp.path().join("source.png");
            let still = if format == ExportFormat::Avif {
                // the avif encoder wants even sides
                let (w, h) = ((img.width() & !1).max(2), (img.height() & !1).max(2));
                if (w, h) != img.dimensions() {
                    changes.push(format!("Trimmed to {w}×{h}, since AVIF needs even sides."));
                }
                image::imageops::crop_imm(img, 0, 0, w, h).to_image()
            } else {
                img.clone()
            };
            still.save(&source).map_err(failed)?;
            changes.push(format!("Quality {quality} out of 100."));
            via_ffmpeg(ffmpeg, env, &source, format.saved(), quality, temp.path()).map_err(failed)
        }
    }
}

fn crc(kind: &[u8], data: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(kind);
    c.update(data);
    c.sum()
}

/// adds a colour profile to an encoded picture
pub fn embed_profile(format: ExportFormat, data: Vec<u8>, icc: &[u8]) -> Result<Vec<u8>, String> {
    let broken = || "The exported file could not be given its colour profile.".to_owned();
    match format {
        ExportFormat::Png => {
            // after the signature and the header chunk, which is always first
            const AFTER_IHDR: usize = 8 + 4 + 4 + 13 + 4;
            if data.len() < AFTER_IHDR || &data[12..16] != b"IHDR" {
                return Err(broken());
            }
            let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
            z.write_all(icc).map_err(failed)?;
            let mut body = b"ICC Profile\0\0".to_vec();
            body.extend_from_slice(&z.finish().map_err(failed)?);
            let mut out = data[..AFTER_IHDR].to_vec();
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(b"iCCP");
            out.extend_from_slice(&body);
            out.extend_from_slice(&crc(b"iCCP", &body).to_be_bytes());
            out.extend_from_slice(&data[AFTER_IHDR..]);
            Ok(out)
        }
        ExportFormat::Jpeg => {
            if data.len() < 4 || data[..2] != [0xFF, 0xD8] {
                return Err(broken());
            }
            // after the jfif segment when there is one
            let mut at = 2;
            if data[2..4] == [0xFF, 0xE0] && data.len() > 6 {
                at = 4 + usize::from(u16::from_be_bytes([data[4], data[5]]));
            }
            let chunks: Vec<&[u8]> = icc.chunks(65519).collect();
            let mut segs = Vec::new();
            for (i, chunk) in chunks.iter().enumerate() {
                segs.extend_from_slice(&[0xFF, 0xE2]);
                segs.extend_from_slice(&((2 + 14 + chunk.len()) as u16).to_be_bytes());
                segs.extend_from_slice(b"ICC_PROFILE\0");
                segs.push((i + 1) as u8);
                segs.push(chunks.len() as u8);
                segs.extend_from_slice(chunk);
            }
            let mut out = data[..at].to_vec();
            out.extend_from_slice(&segs);
            out.extend_from_slice(&data[at..]);
            Ok(out)
        }
        ExportFormat::Webp => embed_webp(&data, icc).ok_or_else(broken),
        ExportFormat::Avif => Ok(data),
    }
}

/// a webp in the extended format with an ICCP chunk. a simple webp gains a VP8X chunk
/// saying how big the canvas is and whether it has alpha.
fn embed_webp(data: &[u8], icc: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 20 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return None;
    }
    let mut chunks: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    let mut at = 12;
    while at + 8 <= data.len() {
        let kind: [u8; 4] = data[at..at + 4].try_into().ok()?;
        let len = u32::from_le_bytes(data[at + 4..at + 8].try_into().ok()?) as usize;
        let body = data.get(at + 8..at + 8 + len)?.to_vec();
        chunks.push((kind, body));
        at += 8 + len + (len & 1);
    }
    chunks.retain(|(k, _)| k != b"ICCP");
    if chunks.first().is_some_and(|(k, _)| k == b"VP8X") {
        chunks[0].1[0] |= 0x20;
    } else {
        let (w, h, alpha) = chunks.iter().find_map(|(k, b)| match k {
            b"VP8 " if b.len() >= 10 && b[3..6] == [0x9d, 0x01, 0x2a] => Some((
                u32::from(u16::from_le_bytes([b[6], b[7]]) & 0x3fff),
                u32::from(u16::from_le_bytes([b[8], b[9]]) & 0x3fff),
                false,
            )),
            b"VP8L" if b.len() >= 5 && b[0] == 0x2f => {
                let bits = u32::from_le_bytes([b[1], b[2], b[3], b[4]]);
                Some((
                    (bits & 0x3fff) + 1,
                    ((bits >> 14) & 0x3fff) + 1,
                    bits & (1 << 28) != 0,
                ))
            }
            _ => None,
        })?;
        let alpha = alpha || chunks.iter().any(|(k, _)| k == b"ALPH");
        let mut x = vec![0x20 | if alpha { 0x10 } else { 0 }, 0, 0, 0];
        x.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
        x.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
        chunks.insert(0, (*b"VP8X", x));
    }
    chunks.insert(1, (*b"ICCP", icc.to_vec()));
    let mut body = b"WEBP".to_vec();
    for (k, b) in &chunks {
        body.extend_from_slice(k);
        body.extend_from_slice(&(b.len() as u32).to_le_bytes());
        body.extend_from_slice(b);
        if b.len() & 1 == 1 {
            body.push(0);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_gpu::IRect;
    use vellum_media::Unattended;
    use vellum_media::ffmpeg::hw::HardwareSupport;

    fn env(hw: &HardwareSupport) -> Env<'_> {
        Env {
            ffmpeg: None,
            hardware: hw,
            allow_hardware: false,
            output_folder: None,
        }
    }

    fn sample(depth: Depth) -> ImageDocument {
        let mut doc = ImageDocument::new("t", 40, 30, depth, Some([0.2, 0.5, 0.9, 1.0]));
        let r = doc.layers[0].raster_mut().unwrap();
        r.update(IRect::new(0, 0, 40, 30), |x, y, px| {
            px[0] = x as f32 / 40.0;
            px[1] = y as f32 / 30.0;
        });
        doc
    }

    #[test]
    fn sixteen_bit_documents_keep_sixteen_bits_in_png() {
        let hw = HardwareSupport::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep.png");
        let mut doc = sample(Depth::U16);
        doc.color = crate::document::DocColor::from_icc(
            moxcms::ColorProfile::new_display_p3().encode().unwrap(),
        );
        let result = export(
            &doc,
            &path,
            &ExportOptions::default(),
            &env(&hw),
            5.0,
            &Unattended,
        )
        .unwrap();
        assert_eq!(result.bytes, std::fs::metadata(&path).ok().map(|m| m.len()));
        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        assert_eq!(info.bit_depth, png::BitDepth::Sixteen);
        assert!(info.icc_profile.is_some(), "the profile is embedded");
        let back = image::open(&path).unwrap().to_rgba16();
        assert_eq!(
            back.get_pixel(20, 15)[0],
            (20.0f32 / 40.0 * 65535.0).round() as u16
        );
    }

    #[test]
    fn jpegs_flatten_and_carry_the_profile() {
        let hw = HardwareSupport::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        let mut doc = ImageDocument::new("t", 64, 64, Depth::U8, None);
        doc.color = crate::document::DocColor::from_icc(
            moxcms::ColorProfile::new_display_p3().encode().unwrap(),
        );
        let options = ExportOptions {
            format: ExportFormat::Jpeg,
            background: [1.0, 0.0, 0.0],
            ..ExportOptions::default()
        };
        export(&doc, &path, &options, &env(&hw), 5.0, &Unattended).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.windows(12).any(|w| w == b"ICC_PROFILE\0"));
        let back = image::open(&path).unwrap().to_rgb8();
        // the see through document comes out over the red background
        assert!(back.get_pixel(30, 30)[0] > 240 && back.get_pixel(30, 30)[1] < 20);
    }

    #[test]
    fn export_to_a_size_fits() {
        let hw = HardwareSupport::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.jpg");
        let mut doc = ImageDocument::new("t", 600, 400, Depth::U8, Some([1.0; 4]));
        let r = doc.layers[0].raster_mut().unwrap();
        // noise makes a big jpeg
        r.update(IRect::new(0, 0, 600, 400), |x, y, px| {
            let n = ((x * 7919 + y * 104729) % 251) as f32 / 251.0;
            px[0] = n;
            px[1] = 1.0 - n;
        });
        let options = ExportOptions {
            format: ExportFormat::Jpeg,
            target: Some(TargetSize {
                amount: 0.03,
                unit: vellum_core::settings::SizeUnit::Mb,
            }),
            ..ExportOptions::default()
        };
        let result = export(&doc, &path, &options, &env(&hw), 5.0, &Unattended).unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size <= 30_000, "{size} bytes");
        assert!(!result.changes.is_empty());
    }

    #[test]
    fn webp_gains_an_extended_header_with_the_profile() {
        // a tiny lossless webp: 1x1, made by hand
        let vp8l = [
            0x2f, 0x00, 0x00, 0x00, 0x10, 0x07, 0x10, 0x11, 0x11, 0x88, 0x88, 0xfe, 0x07, 0x00,
        ];
        let mut data = b"RIFF".to_vec();
        data.extend_from_slice(&((4 + 8 + vp8l.len()) as u32).to_le_bytes());
        data.extend_from_slice(b"WEBP");
        data.extend_from_slice(b"VP8L");
        data.extend_from_slice(&(vp8l.len() as u32).to_le_bytes());
        data.extend_from_slice(&vp8l);
        let out = embed_webp(&data, b"profile!").unwrap();
        assert_eq!(&out[12..16], b"VP8X");
        assert_eq!(out[20] & 0x20, 0x20);
        assert_eq!(&out[30..34], b"ICCP");
        let riff = u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, out.len());
    }
}
