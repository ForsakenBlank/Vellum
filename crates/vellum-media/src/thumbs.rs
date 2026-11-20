//! thumbnails and filmstrips. the service runs these as background jobs and caches the
//! results; the functions here just make the pixels.

use std::io::Cursor;
use std::path::Path;

use ::image::codecs::jpeg::JpegEncoder;
use ::image::{ImageEncoder, RgbaImage};

use crate::control::Control;
use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::Ffmpeg;
use crate::picture::{self, DecodeOptions};
use crate::probe::{MediaInfo, MediaKind};
use crate::seek;

/// the longest edge of a thumbnail. big enough for the home screen cards and the info panel
/// on a high density screen.
pub const THUMB_SIZE: u32 = 384;

/// a thumbnail for any file, or none for audio without cover art
pub fn thumbnail(
    ffmpeg: Option<&Ffmpeg>,
    path: &Path,
    info: &MediaInfo,
    size: u32,
) -> MediaResult<Option<RgbaImage>> {
    match info.kind {
        MediaKind::Image | MediaKind::Animation | MediaKind::Vector => {
            let picture = picture::decode(path, DecodeOptions::preview(size), ffmpeg)?;
            Ok(Some(picture.first().clone()))
        }
        MediaKind::Video => {
            let ffmpeg = ffmpeg.ok_or(MediaError::FfmpegMissing)?;
            video_thumbnail(ffmpeg, path, info, size).map(Some)
        }
        MediaKind::Audio => {
            let ffmpeg = ffmpeg.ok_or(MediaError::FfmpegMissing)?;
            Ok(cover_art(ffmpeg, path, size))
        }
        MediaKind::Unknown => Ok(None),
    }
}

/// the size a video frame is shown at inside a `size` square, rounded to even numbers
fn frame_size(info: &MediaInfo, size: u32) -> (u32, u32) {
    let (w, h) = (info.width.unwrap_or(16), info.height.unwrap_or(9));
    let (w, h) = picture::fit(w, h, size);
    (w.max(2) & !1, h.max(2) & !1)
}

/// a frame a tenth of the way in, which skips most fade ins and title cards. if that frame
/// is nearly black, try further in.
fn video_thumbnail(
    ffmpeg: &Ffmpeg,
    path: &Path,
    info: &MediaInfo,
    size: u32,
) -> MediaResult<RgbaImage> {
    let duration = info.duration_secs.unwrap_or(0.0);
    let size = frame_size(info, size);
    let mut fallback = None;
    for fraction in [0.1, 0.33, 0.5] {
        let at = if duration > 0.5 {
            duration * fraction
        } else {
            0.0
        };
        match seek::grab(ffmpeg, path, at, false, size) {
            Ok(frame) if mean_brightness(&frame) >= 16.0 => return Ok(frame),
            Ok(frame) => {
                fallback.get_or_insert(frame);
            }
            Err(err) if fallback.is_none() && at == 0.0 => return Err(err),
            Err(_) => {}
        }
        if duration <= 0.5 {
            break;
        }
    }
    match fallback {
        Some(frame) => Ok(frame),
        None => seek::grab(ffmpeg, path, 0.0, false, size),
    }
}

fn mean_brightness(image: &RgbaImage) -> f64 {
    let pixels = image.pixels().len().max(1) as f64;
    let total: f64 = image
        .pixels()
        .map(|p| 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]))
        .sum();
    total / pixels
}

/// the album art most music files carry as an attached picture
fn cover_art(ffmpeg: &Ffmpeg, path: &Path, size: u32) -> Option<RgbaImage> {
    let mut cmd = ffmpeg.command();
    cmd.arg("-i").arg(path).args([
        "-map",
        "0:v:0",
        "-frames:v",
        "1",
        "-f",
        "image2pipe",
        "-c:v",
        "png",
        "pipe:1",
    ]);
    let out = crate::ffmpeg::process::capture(cmd, std::time::Duration::from_secs(20)).ok()?;
    let image = ::image::load_from_memory(&out).ok()?.into_rgba8();
    Some(picture::shrink(image, size))
}

/// frames spread evenly across a video, side by side in one image
#[derive(Debug, Clone)]
pub struct Filmstrip {
    pub image: RgbaImage,
    pub frame_width: u32,
    pub frame_height: u32,
    /// when each frame comes from, in seconds
    pub times: Vec<f64>,
}

pub fn filmstrip(
    ffmpeg: &Ffmpeg,
    path: &Path,
    info: &MediaInfo,
    count: u32,
    height: u32,
    control: &dyn Control,
) -> MediaResult<Filmstrip> {
    let count = count.clamp(1, 64);
    let duration = info
        .duration_secs
        .filter(|d| *d > 0.0)
        .ok_or_else(|| MediaError::decode(Path::new(&info.path), "it has no duration"))?;
    let (w, h) = (
        info.width.unwrap_or(16).max(1),
        info.height.unwrap_or(9).max(1),
    );
    let frame_height = height.max(2) & !1;
    let frame_width =
        ((f64::from(w) / f64::from(h) * f64::from(frame_height)).round() as u32).max(2) & !1;
    let times: Vec<f64> = (0..count)
        .map(|i| duration * (f64::from(i) + 0.5) / f64::from(count))
        .collect();
    // one ffmpeg per frame, a few at a time. each one mostly waits on a single keyframe
    // decode, so running four side by side takes the 4k benchmark from 3.8 s to about 1.1 s.
    let parallel = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .clamp(1, 4);
    let mut frames = Vec::with_capacity(times.len());
    for batch in times.chunks(parallel) {
        control.checkpoint()?;
        let grabbed: Vec<MediaResult<RgbaImage>> = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|&at| {
                    scope.spawn(move || {
                        seek::grab(ffmpeg, path, at, false, (frame_width, frame_height))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| Err(MediaError::Ffmpeg("a frame grab crashed".into())))
                })
                .collect()
        });
        for frame in grabbed {
            frames.push(frame?);
        }
        control.progress(frames.len() as f32 / count as f32);
    }
    let mut strip = RgbaImage::new(frame_width * count, frame_height);
    for (i, frame) in (0_u32..).zip(&frames) {
        ::image::imageops::replace(&mut strip, frame, i64::from(i * frame_width), 0);
    }
    Ok(Filmstrip {
        image: strip,
        frame_width,
        frame_height,
        times,
    })
}

/// png when the image has transparency, otherwise a jpeg at a quality that hides artefacts
/// at thumbnail size. returns the bytes and the file extension.
pub fn encode_preview(image: &RgbaImage) -> MediaResult<(Vec<u8>, &'static str)> {
    let mut out = Cursor::new(Vec::new());
    let transparent = image.pixels().any(|p| p[3] < 255);
    let bad = |err: ::image::ImageError| {
        MediaError::Unsupported(format!("could not save a preview: {err}"))
    };
    if transparent {
        image
            .write_to(&mut out, ::image::ImageFormat::Png)
            .map_err(bad)?;
        return Ok((out.into_inner(), "png"));
    }
    let rgb = ::image::DynamicImage::ImageRgba8(image.clone()).into_rgb8();
    JpegEncoder::new_with_quality(&mut out, 85)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ::image::ExtendedColorType::Rgb8,
        )
        .map_err(bad)?;
    Ok((out.into_inner(), "jpg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_keep_transparency_only_when_there_is_some() {
        let opaque = RgbaImage::from_pixel(8, 8, ::image::Rgba([10, 20, 30, 255]));
        assert_eq!(encode_preview(&opaque).unwrap().1, "jpg");
        let clear = RgbaImage::from_pixel(8, 8, ::image::Rgba([10, 20, 30, 0]));
        assert_eq!(encode_preview(&clear).unwrap().1, "png");
    }

    #[test]
    fn brightness_tells_black_from_content() {
        let black = RgbaImage::from_pixel(4, 4, ::image::Rgba([0, 0, 0, 255]));
        let grey = RgbaImage::from_pixel(4, 4, ::image::Rgba([128, 128, 128, 255]));
        assert!(mean_brightness(&black) < 1.0);
        assert!((mean_brightness(&grey) - 128.0).abs() < 1.0);
    }
}
