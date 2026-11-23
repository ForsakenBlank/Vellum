//! fitting a picture under a size. lossy formats get a binary search for the best quality
//! that fits; if even a low quality is too big the picture is scaled down by about the
//! square root of how far over it is, and the search runs again. pngs are first optimised
//! losslessly, then reduced to a palette of 256 colours (pngquant style), then scaled.
//!
//! pngquant's own library is gpl, so palettes come from `quantette` (k-means in oklab with
//! dithering) and, for pictures with transparency, `color_quant`.

use std::path::Path;

use color_quant::NeuQuant;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{Rgb, RgbImage, RgbaImage};
use quantette::{ImageBuf, Pipeline, QuantizeMethod, dither::FloydSteinberg};
use vellum_media::picture::{self, DecodeOptions, PictureFormat};
use vellum_media::probe::MediaInfo;
use vellum_media::{Control, MediaError, MediaResult};

use super::{ImageTarget, SizeJob, already_fits, too_big};
use crate::{Env, ToolResult, naming, output_args, run_ffmpeg};

/// rounds of scaling down before giving up
const MAX_ROUNDS: usize = 10;
/// nothing is scaled below this on its long edge
const MIN_EDGE: u32 = 16;

/// the format a picture is saved in, after `auto` is worked out
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    Jpeg,
    Webp,
    Avif,
    Png,
}

impl Saved {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Avif => "avif",
            Self::Png => "png",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Webp => "WebP",
            Self::Avif => "AVIF",
            Self::Png => "PNG",
        }
    }

    /// the quality range searched, lowest first
    pub fn range(self) -> (u8, u8) {
        match self {
            Self::Jpeg => (35, 92),
            Self::Webp => (25, 92),
            Self::Avif => (15, 90),
            Self::Png => (0, 0),
        }
    }
}

/// keeps jpeg, png, webp and avif as they are; everything else becomes jpeg, or webp when
/// it has transparency a jpeg would lose
pub fn choose(target: ImageTarget, source: Option<PictureFormat>, has_alpha: bool) -> Saved {
    match target {
        ImageTarget::Jpeg => Saved::Jpeg,
        ImageTarget::Webp => Saved::Webp,
        ImageTarget::Avif => Saved::Avif,
        ImageTarget::Png => Saved::Png,
        ImageTarget::Auto => match source {
            Some(PictureFormat::Jpeg) => Saved::Jpeg,
            Some(PictureFormat::Png) => Saved::Png,
            Some(PictureFormat::Webp) => Saved::Webp,
            Some(PictureFormat::Avif) => Saved::Avif,
            _ if has_alpha => Saved::Webp,
            _ => Saved::Jpeg,
        },
    }
}

/// the best quality in `lo..=hi` whose result fits `goal`, trying the top first. `encode`
/// is assumed to give smaller files at lower qualities.
pub fn search(
    lo: u8,
    hi: u8,
    goal: u64,
    mut encode: impl FnMut(u8) -> MediaResult<Vec<u8>>,
) -> MediaResult<Option<(u8, Vec<u8>)>> {
    let top = encode(hi)?;
    if top.len() as u64 <= goal {
        return Ok(Some((hi, top)));
    }
    let bottom = encode(lo)?;
    if bottom.len() as u64 > goal {
        return Ok(None);
    }
    let (mut fits, mut over) = (lo, hi);
    let mut best = bottom;
    while over - fits > 1 {
        let mid = fits + (over - fits) / 2;
        let data = encode(mid)?;
        if data.len() as u64 <= goal {
            fits = mid;
            best = data;
        } else {
            over = mid;
        }
    }
    Ok(Some((fits, best)))
}

pub fn fit(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    job: &SizeJob,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let goal = job.goal;
    let source_format = info.image.as_ref().map(|i| i.format);
    let keeps_format = matches!(
        (job.options.image_format, source_format),
        (
            ImageTarget::Auto,
            Some(
                PictureFormat::Jpeg
                    | PictureFormat::Png
                    | PictureFormat::Webp
                    | PictureFormat::Avif
            )
        )
    );
    if keeps_format && info.size_bytes <= goal {
        return Ok(already_fits(input, goal));
    }
    let picture = picture::decode(
        input,
        DecodeOptions {
            size: None,
            all_frames: true,
        },
        env.ffmpeg,
    )?;
    if picture.frames.len() > 1 {
        return Err(MediaError::Unsupported(format!(
            "{} is animated. Vellum shrinks animated GIFs, but not animated WebP or PNG yet. Convert it to a GIF or an MP4 first.",
            info.name
        )));
    }
    let image = picture.first().clone();
    let has_alpha = image.pixels().any(|p| p[3] < 255);
    let saved = choose(job.options.image_format, source_format, has_alpha);
    control.progress(0.1);

    let output = naming::output_for(
        input,
        env.output_folder,
        Some(&job.label),
        saved.extension(),
    );
    let (data, mut changes) = squeeze(env, &image, saved, goal, control)?;
    crate::write_atomic(output.path(), &data)?;

    let mut result = ToolResult::file(input, output.path());
    result.target_bytes = Some(goal);
    if saved.extension()
        != input
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default()
        && !(saved == Saved::Jpeg
            && input
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jpeg")))
    {
        changes.insert(0, format!("Saved as {}.", saved.label()));
    }
    let scaled = changes.iter().any(|c| c.starts_with("Scaled"));
    if saved == Saved::Png && scaled && !has_alpha {
        result.notes.push(
            "PNG is a poor fit for photos. Picking JPEG or WebP would keep it bigger and sharper at this size."
                .to_owned(),
        );
    }
    result.changes = changes;
    control.progress(1.0);
    Ok(result)
}

/// the smallest change to `image` that fits, and what was changed: the best quality
/// first, then smaller sizes. the image editor's export to a size uses this too.
pub fn squeeze(
    env: &Env,
    image: &RgbaImage,
    saved: Saved,
    goal: u64,
    control: &dyn Control,
) -> MediaResult<(Vec<u8>, Vec<String>)> {
    let (width, height) = image.dimensions();
    let mut current = image.clone();
    let mut changes = Vec::new();
    for round in 0..MAX_ROUNDS {
        control.checkpoint()?;
        control.progress(0.1 + 0.8 * round as f32 / MAX_ROUNDS as f32);
        let found = match saved {
            Saved::Png => png_fit(&current, goal, round == 0)?,
            Saved::Jpeg => {
                let rgb = flatten(&current);
                let (lo, hi) = saved.range();
                search(lo, hi, goal, |q| jpeg(&rgb, q))?.map(|(q, data)| (Some(q), data))
            }
            Saved::Webp | Saved::Avif => {
                let ffmpeg = env.ffmpeg()?;
                let temp =
                    tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
                let source = temp.path().join("source.png");
                let still = if saved == Saved::Avif {
                    // avif here has no transparency and needs even sides
                    let rgb = flatten(&current);
                    let (w, h) = (rgb.width() & !1, rgb.height() & !1);
                    image::imageops::crop_imm(&rgb, 0, 0, w.max(2), h.max(2))
                        .to_image()
                        .save(&source)
                } else {
                    current.save(&source)
                };
                still.map_err(|err| {
                    MediaError::Unsupported(format!("The picture could not be prepared: {err}"))
                })?;
                let (lo, hi) = saved.range();
                search(lo, hi, goal, |q| {
                    via_ffmpeg(ffmpeg, env, &source, saved, q, temp.path())
                })?
                .map(|(q, data)| (Some(q), data))
            }
        };
        if let Some((quality, data)) = found {
            if let Some(q) = quality.filter(|_| saved != Saved::Png) {
                changes.push(format!("Quality {q} out of 100."));
            } else if quality.is_some() {
                changes.push("Reduced to 256 colours.".to_owned());
            }
            if current.dimensions() != (width, height) {
                changes.insert(
                    0,
                    format!(
                        "Scaled down from {width}×{height} to {}×{} to fit.",
                        current.width(),
                        current.height()
                    ),
                );
            }
            return Ok((data, changes));
        }
        // how far over the lowest setting was says how much smaller to go
        let smallest = match saved {
            Saved::Png => png_quantized(&current)?.len(),
            Saved::Jpeg => jpeg(&flatten(&current), saved.range().0)?.len(),
            _ => (goal as f64 * 2.0) as usize,
        } as f64;
        let factor = ((goal as f64 / smallest).sqrt() * 0.92).clamp(0.3, 0.95);
        let (w, h) = current.dimensions();
        let next = (
            ((f64::from(w) * factor).round() as u32).max(1),
            ((f64::from(h) * factor).round() as u32).max(1),
        );
        if next.0.max(next.1) < MIN_EDGE {
            break;
        }
        current = image::imageops::resize(image, next.0, next.1, FilterType::Lanczos3);
    }
    Err(too_big(goal, "this picture"))
}

fn flatten(image: &RgbaImage) -> RgbImage {
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let p = image.get_pixel(x, y);
        let a = u16::from(p[3]);
        // over white, like a browser shows it
        let blend = |c: u8| ((u16::from(c) * a + 255 * (255 - a)) / 255) as u8;
        Rgb([blend(p[0]), blend(p[1]), blend(p[2])])
    })
}

pub fn jpeg(image: &RgbImage, quality: u8) -> MediaResult<Vec<u8>> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .encode_image(image)
        .map_err(|err| MediaError::Unsupported(format!("The JPEG could not be written: {err}")))?;
    Ok(out)
}

/// a webp or avif of the png at `source`, encoded by ffmpeg at a quality from 0 to 100
pub fn via_ffmpeg(
    ffmpeg: &vellum_media::ffmpeg::Ffmpeg,
    env: &Env,
    source: &Path,
    saved: Saved,
    quality: u8,
    dir: &Path,
) -> MediaResult<Vec<u8>> {
    let out = dir.join(format!("q{quality}.{}", saved.extension()));
    let mut args: Vec<String> = vec!["-i".into(), source.display().to_string()];
    match saved {
        Saved::Webp => args.extend([
            "-c:v".into(),
            "libwebp".into(),
            "-quality".into(),
            quality.to_string(),
            "-compression_level".into(),
            "4".into(),
            "-f".into(),
            "webp".into(),
        ]),
        _ => {
            let encoder = crate::avif_encoder(env)?.to_owned();
            let crf = 63 - u32::from(quality) * 6 / 10;
            args.extend([
                "-c:v".into(),
                encoder.clone(),
                "-crf".into(),
                crf.to_string(),
            ]);
            if encoder == "libaom-av1" {
                args.extend([
                    "-still-picture".into(),
                    "1".into(),
                    "-cpu-used".into(),
                    "6".into(),
                ]);
            } else {
                args.extend(["-preset".into(), "8".into()]);
            }
            args.extend([
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-f".into(),
                "avif".into(),
            ]);
        }
    }
    args.extend(output_args(&out));
    run_ffmpeg(ffmpeg, &args, None, &vellum_media::Unattended)?;
    std::fs::read(&out).map_err(|err| MediaError::io(&out, err))
}

/// lossless first (only on the first round, at full size), then a 256 colour palette
fn png_fit(
    image: &RgbaImage,
    goal: u64,
    try_lossless: bool,
) -> MediaResult<Option<(Option<u8>, Vec<u8>)>> {
    if try_lossless {
        let lossless = optimise(&plain_png(image)?)?;
        if lossless.len() as u64 <= goal {
            return Ok(Some((None, lossless)));
        }
    }
    let quantized = png_quantized(image)?;
    Ok((quantized.len() as u64 <= goal).then_some((Some(0), quantized)))
}

/// a plain 8 bit png, rgb when nothing is see through
pub fn plain_png(image: &RgbaImage) -> MediaResult<Vec<u8>> {
    let opaque = image.pixels().all(|p| p[3] == 255);
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    let result = if opaque {
        image::DynamicImage::ImageRgba8(image.clone())
            .to_rgb8()
            .write_with_encoder(encoder)
    } else {
        image.write_with_encoder(encoder)
    };
    result
        .map_err(|err| MediaError::Unsupported(format!("The PNG could not be written: {err}")))?;
    Ok(out)
}

/// the same png, losslessly smaller
pub fn optimise(png: &[u8]) -> MediaResult<Vec<u8>> {
    let options = oxipng::Options::from_preset(2);
    oxipng::optimize_from_memory(png, &options)
        .map_err(|err| MediaError::Unsupported(format!("The PNG could not be optimised: {err}")))
}

/// a 256 colour palette png, dithered, then optimised
pub fn png_quantized(image: &RgbaImage) -> MediaResult<Vec<u8>> {
    let (w, h) = image.dimensions();
    let opaque = image.pixels().all(|p| p[3] == 255);
    let (palette, alphas, indices): (Vec<u8>, Option<Vec<u8>>, Vec<u8>) = if opaque {
        let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
        let buf = ImageBuf::try_from(rgb).map_err(|err| {
            MediaError::Unsupported(format!("The picture is too big to reduce: {err}"))
        })?;
        let size = 256_u16
            .try_into()
            .map_err(|_| MediaError::Unsupported("bad palette size".to_owned()))?;
        let indexed = Pipeline::new()
            .palette_size(size)
            .quantize_method(QuantizeMethod::kmeans())
            .ditherer(FloydSteinberg::new())
            .parallel(true)
            .input_image(buf.as_ref())
            .output_srgb8_indexed_image();
        let (colors, indices) = indexed.into_parts();
        let palette = colors
            .iter()
            .flat_map(|c| [c.red, c.green, c.blue])
            .collect();
        (palette, None, indices)
    } else {
        // fully clear pixels get a slot of their own, the rest share 255 learned colours
        let visible: Vec<u8> = image
            .pixels()
            .filter(|p| p[3] > 0)
            .flat_map(|p| p.0)
            .collect();
        let sample = if visible.is_empty() {
            vec![0, 0, 0, 255]
        } else {
            visible
        };
        let quant = NeuQuant::new(5, 255, &sample);
        let map = quant.color_map_rgba();
        let clear = (map.len() / 4) as u8;
        let mut palette: Vec<u8> = map
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2]])
            .collect();
        palette.extend([0, 0, 0]);
        let mut alphas: Vec<u8> = map.as_chunks::<4>().0.iter().map(|c| c[3]).collect();
        alphas.push(0);
        let indices = image
            .pixels()
            .map(|p| {
                if p[3] == 0 {
                    clear
                } else {
                    quant.index_of(&p.0) as u8
                }
            })
            .collect();
        (palette, Some(alphas), indices)
    };
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_palette(palette);
        if let Some(alphas) = alphas {
            encoder.set_trns(alphas);
        }
        let mut writer = encoder.write_header().map_err(|err| {
            MediaError::Unsupported(format!("The PNG could not be written: {err}"))
        })?;
        writer.write_image_data(&indices).map_err(|err| {
            MediaError::Unsupported(format!("The PNG could not be written: {err}"))
        })?;
    }
    optimise(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_keeps_web_formats_and_picks_for_the_rest() {
        assert_eq!(
            choose(ImageTarget::Auto, Some(PictureFormat::Png), true),
            Saved::Png
        );
        assert_eq!(
            choose(ImageTarget::Auto, Some(PictureFormat::Heif), false),
            Saved::Jpeg
        );
        assert_eq!(
            choose(ImageTarget::Auto, Some(PictureFormat::Tiff), true),
            Saved::Webp
        );
        assert_eq!(
            choose(ImageTarget::Avif, Some(PictureFormat::Png), true),
            Saved::Avif
        );
    }

    #[test]
    fn search_finds_the_best_quality_that_fits() {
        // a pretend encoder whose size is ten bytes per quality step
        let encode = |q: u8| Ok(vec![0; usize::from(q) * 10]);
        let (q, data) = search(10, 90, 555, encode).unwrap().unwrap();
        assert_eq!(q, 55);
        assert_eq!(data.len(), 550);
        assert_eq!(search(10, 90, 5000, encode).unwrap().unwrap().0, 90);
        assert!(search(10, 90, 50, encode).unwrap().is_none());
    }

    #[test]
    fn palette_pngs_keep_transparency() {
        let mut image = RgbaImage::new(40, 30);
        for (x, y, p) in image.enumerate_pixels_mut() {
            *p = image::Rgba([
                (x * 6) as u8,
                (y * 8) as u8,
                90,
                if x < 5 { 0 } else { 255 },
            ]);
        }
        let data = png_quantized(&image).unwrap();
        let back = image::load_from_memory(&data).unwrap().to_rgba8();
        assert_eq!(back.dimensions(), (40, 30));
        assert!(back.get_pixel(1, 1)[3] < 20);
        assert_eq!(back.get_pixel(30, 10)[3], 255);
    }
}
