//! thumbnails for every kind of file, and filmstrips across a video.
//!
//! video thumbnails use keyframe seeking: ffmpeg jumps to the keyframe before the chosen
//! time and decodes only that one frame, so a 4k hevc file costs about as much as a jpeg.
//! nearly black frames (fades, intros) are skipped by looking a little further in.

use std::io::{Cursor, Read};
use std::path::Path;

use image::{DynamicImage, GenericImage, ImageFormat, RgbImage};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::{FfmpegTools, Reporter, run_reading};
use crate::images::{DecodeOptions, decode_image};
use crate::info::{MediaInfo, MediaKind, StreamKind};
use crate::sniff::Format;

/// where in a video to look for a thumbnail, as fractions of its length
const THUMBNAIL_SPOTS: [f64; 3] = [0.1, 0.3, 0.5];
/// average brightness (0 to 255) below which a frame counts as black
const BLACK_LEVEL: f64 = 12.0;

/// makes a thumbnail that fits in a `max_side` square. none when the file has no picture,
/// like an audio file without cover art.
pub fn make_thumbnail(
    path: &Path,
    format: Format,
    info: &MediaInfo,
    max_side: u32,
    tools: Option<&FfmpegTools>,
) -> MediaResult<Option<DynamicImage>> {
    if format.is_image() {
        let decoded = decode_image(path, format, &DecodeOptions::preview(max_side), tools)?;
        let mut frames = decoded.frames;
        return Ok(frames.drain(..).next().map(|f| f.image));
    }
    let tools = tools.ok_or(MediaError::FfmpegMissing)?;
    if let Some(video) = &info.video {
        let duration = info.duration_secs.unwrap_or(0.0);
        let mut last = None;
        for spot in THUMBNAIL_SPOTS {
            let image = grab(tools, path, video.stream_index, duration * spot, max_side)?;
            if brightness(&image) >= BLACK_LEVEL {
                return Ok(Some(image));
            }
            last = Some(image);
        }
        return Ok(last);
    }
    // cover art in music files is a video stream marked as an attached picture
    if let Some(cover) = info.streams.iter().find(|s| s.kind == StreamKind::Picture) {
        return grab(tools, path, cover.index, 0.0, max_side).map(Some);
    }
    Ok(None)
}

/// one frame from the keyframe at or before `seek`, shrunk to fit `max_side`. it comes
/// out as png so ffmpeg works out the size, including rotation and non square pixels.
fn grab(
    tools: &FfmpegTools,
    path: &Path,
    stream: u32,
    seek: f64,
    max_side: u32,
) -> MediaResult<DynamicImage> {
    let side = max_side.max(16);
    // small pictures (cover art) are never blown up
    let mut cmd = tools.ffmpeg();
    cmd.args(["-noaccurate_seek", "-skip_frame", "nokey"])
        .args(["-ss", &format!("{:.3}", seek.max(0.0))])
        .arg("-i")
        .arg(path)
        .args(["-map", &format!("0:{stream}")])
        .args(["-frames:v", "1", "-an", "-sn", "-fps_mode", "passthrough"])
        .args([
            "-vf",
            &format!(
                "scale=trunc(iw*sar/2)*2:ih,setsar=1,\
                 scale=w='min({side},iw)':h='min({side},ih)':\
                 force_original_aspect_ratio=decrease:flags=bicubic"
            ),
        ])
        .args(["-c:v", "png", "-f", "image2pipe", "pipe:1"]);
    let bytes = run_reading(&mut cmd, "ffmpeg", |stdout| {
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .map_err(|err| MediaError::io(path, err))?;
        Ok(bytes)
    })?;
    if bytes.is_empty() {
        return Err(MediaError::decode(format!(
            "there is no picture at {seek:.1} seconds"
        )));
    }
    image::load_from_memory_with_format(&bytes, ImageFormat::Png)
        .map_err(|err| MediaError::decode(format!("ffmpeg gave an unreadable frame: {err}")))
}

/// average brightness, looking at a sparse grid of pixels
fn brightness(image: &DynamicImage) -> f64 {
    let rgb = image.to_rgb8();
    let step_x = (rgb.width() / 16).max(1);
    let step_y = (rgb.height() / 16).max(1);
    let (mut sum, mut count) = (0.0, 0.0);
    for y in (0..rgb.height()).step_by(step_y as usize) {
        for x in (0..rgb.width()).step_by(step_x as usize) {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            sum += 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
            count += 1.0;
        }
    }
    if count == 0.0 { 0.0 } else { sum / count }
}

/// encodes a thumbnail: jpeg when opaque, png when it has see-through parts.
/// returns the bytes and the file extension.
pub fn encode_thumbnail(image: &DynamicImage) -> MediaResult<(Vec<u8>, &'static str)> {
    let has_alpha = image.color().has_alpha() && image.to_rgba8().pixels().any(|p| p.0[3] < 255);
    let mut out = Cursor::new(Vec::new());
    let ext = if has_alpha {
        image
            .to_rgba8()
            .write_to(&mut out, ImageFormat::Png)
            .map_err(|err| MediaError::decode(err.to_string()))?;
        "png"
    } else {
        let rgb = DynamicImage::ImageRgb8(image.to_rgb8());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82);
        rgb.write_with_encoder(encoder)
            .map_err(|err| MediaError::decode(err.to_string()))?;
        "jpg"
    };
    Ok((out.into_inner(), ext))
}

/// a strip of evenly spaced frames from a video, side by side in one image
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Filmstrip {
    /// the image holding every tile, left to right
    pub file: String,
    pub tile_width: u32,
    pub tile_height: u32,
    /// when each tile was taken, in seconds
    pub times: Vec<f64>,
}

/// the strip image and tile layout. `file` is left empty for the caller to fill.
pub fn make_filmstrip(
    tools: &FfmpegTools,
    path: &Path,
    info: &MediaInfo,
    count: u32,
    tile_height: u32,
    reporter: &dyn Reporter,
) -> MediaResult<(RgbImage, Filmstrip)> {
    let video = info
        .video
        .as_ref()
        .filter(|_| info.kind == MediaKind::Video || info.kind == MediaKind::Animation)
        .ok_or_else(|| MediaError::Unsupported("Only videos have a filmstrip.".into()))?;
    let duration = info
        .duration_secs
        .ok_or_else(|| MediaError::Unsupported("This video does not say how long it is.".into()))?;
    let count = count.clamp(1, 200);
    let tile_height = tile_height.clamp(16, 1080);
    let aspect = f64::from(video.display_width) / f64::from(video.display_height.max(1));
    let tile_width = ((f64::from(tile_height) * aspect).round() as u32).max(1);
    let mut strip = RgbImage::new(tile_width * count, tile_height);
    let times: Vec<f64> = (0..count)
        .map(|i| duration * (f64::from(i) + 0.5) / f64::from(count))
        .collect();
    // decoding one keyframe barely uses more than one core, so several run side by side
    let parallel = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 4));
    let side = tile_width.max(tile_height);
    let mut done = 0u32;
    for batch in times.chunks(parallel) {
        reporter.checkpoint()?;
        let frames: Vec<MediaResult<DynamicImage>> = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|time| scope.spawn(move || grab(tools, path, video.stream_index, *time, side)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(MediaError::decode("a frame grab crashed")))
                })
                .collect()
        });
        for frame in frames {
            // the grabbed frame fits the box, so stretch it the last pixel or two to the tile
            let frame = frame?.resize_exact(
                tile_width,
                tile_height,
                image::imageops::FilterType::Triangle,
            );
            strip
                .copy_from(&frame.to_rgb8(), done * tile_width, 0)
                .map_err(|err| MediaError::decode(err.to_string()))?;
            done += 1;
        }
        reporter.progress(done as f32 / count as f32);
    }
    Ok((
        strip,
        Filmstrip {
            file: String::new(),
            tile_width,
            tile_height,
            times,
        },
    ))
}

#[cfg(test)]
mod tests {
    use image::{Rgb, Rgba, RgbaImage};

    use super::*;
    use crate::ffmpeg::{NoReporter, probe_media, testing};
    use crate::images::probe_image;

    #[test]
    fn opaque_thumbnails_are_jpeg_and_see_through_ones_png() {
        let opaque = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, Rgb([1, 2, 3])));
        assert_eq!(encode_thumbnail(&opaque).unwrap().1, "jpg");
        let mut clear = RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]));
        clear.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        assert_eq!(
            encode_thumbnail(&DynamicImage::ImageRgba8(clear))
                .unwrap()
                .1,
            "png"
        );
        // an alpha channel that is fully opaque does not need png
        let solid = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255])));
        assert_eq!(encode_thumbnail(&solid).unwrap().1, "jpg");
    }

    #[test]
    fn image_thumbnails_fit_the_box() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        RgbImage::from_pixel(800, 200, Rgb([9, 9, 9]))
            .save(&path)
            .unwrap();
        let info = probe_image(&path, Format::Png, None).unwrap();
        let thumb = make_thumbnail(&path, Format::Png, &info, 100, None)
            .unwrap()
            .unwrap();
        assert_eq!((thumb.width(), thumb.height()), (100, 25));
    }

    #[test]
    fn video_thumbnails_skip_black_openings_and_follow_rotation() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let flat = dir.path().join("flat.mp4");
        // two black seconds, then colour bars
        testing::make(
            &tools,
            "-f lavfi -i color=c=black:s=320x180:r=25:d=2 -f lavfi -i testsrc2=s=320x180:r=25:d=4 \
             -filter_complex [0][1]concat=n=2:v=1 -c:v libx264 -g 25",
            &flat,
        );
        // then marked as filmed sideways, so it shows portrait
        let path = dir.path().join("fade.mp4");
        testing::make(
            &tools,
            &format!("-display_rotation 90 -i {} -c copy", flat.display()),
            &path,
        );
        let info = probe_media(&tools, &path).unwrap();
        let thumb = make_thumbnail(&path, Format::Media, &info, 160, Some(&tools))
            .unwrap()
            .unwrap();
        assert!(brightness(&thumb) > BLACK_LEVEL);
        // 320 x 180 turned a quarter turn fits 160 tall
        assert_eq!((thumb.width(), thumb.height()), (90, 160));
    }

    #[test]
    fn audio_without_cover_art_has_no_thumbnail_and_with_it_does() {
        let Some(tools) = testing::tools() else {
            return;
        };
        if !testing::has_encoder(&tools, "libmp3lame") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.mp3");
        testing::make(&tools, "-f lavfi -i sine=d=1", &plain);
        let info = probe_media(&tools, &plain).unwrap();
        assert!(
            make_thumbnail(&plain, Format::Media, &info, 64, Some(&tools))
                .unwrap()
                .is_none()
        );

        let cover = dir.path().join("cover.png");
        RgbImage::from_pixel(50, 50, Rgb([200, 0, 0]))
            .save(&cover)
            .unwrap();
        let with_art = dir.path().join("art.mp3");
        testing::make(
            &tools,
            &format!(
                "-f lavfi -i sine=d=1 -i {} -map 0 -map 1 -c:a libmp3lame -c:v png \
                 -disposition:v attached_pic",
                cover.display()
            ),
            &with_art,
        );
        let info = probe_media(&tools, &with_art).unwrap();
        assert_eq!(info.kind, MediaKind::Audio);
        let thumb = make_thumbnail(&with_art, Format::Media, &info, 64, Some(&tools))
            .unwrap()
            .unwrap();
        assert_eq!((thumb.width(), thumb.height()), (50, 50));
    }

    #[test]
    fn filmstrips_tile_frames_across_the_whole_video() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mp4");
        testing::make(
            &tools,
            "-f lavfi -i testsrc2=s=320x240:r=25:d=4 -c:v libx264 -g 10",
            &path,
        );
        let info = probe_media(&tools, &path).unwrap();
        let (strip, layout) = make_filmstrip(&tools, &path, &info, 5, 60, &NoReporter).unwrap();
        assert_eq!((layout.tile_width, layout.tile_height), (80, 60));
        assert_eq!((strip.width(), strip.height()), (400, 60));
        assert_eq!(layout.times.len(), 5);
        assert!((layout.times[0] - 0.4).abs() < 1e-9);
        assert!((layout.times[4] - 3.6).abs() < 1e-9);
    }
}
