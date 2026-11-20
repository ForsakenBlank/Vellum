//! avif, heic and jpeg xl decoded by ffmpeg into raw rgba.

use std::path::Path;
use std::time::Duration;

use ::image::RgbaImage;

use super::{DecodeOptions, Picture, PictureFormat, fit};
use crate::control::Unattended;
use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::Ffmpeg;
use crate::ffmpeg::process::{self, Running};

pub fn dimensions(ffmpeg: &Ffmpeg, path: &Path, format: PictureFormat) -> MediaResult<(u32, u32)> {
    let mut cmd = ffmpeg.probe_command();
    cmd.args([
        "-select_streams",
        "v",
        "-show_entries",
        "stream=width,height",
    ])
    .args(["-of", "csv=p=0"])
    .arg(path);
    let out = process::capture(cmd, Duration::from_secs(20))?;
    let text = String::from_utf8_lossy(&out);
    let sizes: Vec<(u32, u32)> = text
        .lines()
        .filter_map(|line| {
            let (w, h) = line.trim().split_once(',')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .collect();
    // ffmpeg before 7.1 shows a tiled heic (most iphone photos) as one stream per tile
    if format == PictureFormat::Heif && sizes.len() > 1 {
        return Err(MediaError::Unsupported(
            "This HEIC photo is stored in tiles, which needs FFmpeg 7.1 or newer. Update FFmpeg to open it.".into(),
        ));
    }
    sizes
        .first()
        .copied()
        .ok_or_else(|| MediaError::decode(path, "FFmpeg found no picture in it"))
}

pub fn decode(
    ffmpeg: &Ffmpeg,
    path: &Path,
    format: PictureFormat,
    options: DecodeOptions,
) -> MediaResult<Picture> {
    let (w, h) = dimensions(ffmpeg, path, format)?;
    let (out_w, out_h) = options.size.map_or((w, h), |size| fit(w, h, size));
    let mut cmd = ffmpeg.command();
    cmd.arg("-i")
        .arg(path)
        .args(["-map", "0:v:0", "-frames:v", "1"])
        .args(["-vf", &format!("scale={out_w}:{out_h}:flags=area")])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    let expected = (out_w as usize) * (out_h as usize) * 4;
    let mut pixels = Vec::with_capacity(expected);
    let mut running = Running::spawn(cmd)?;
    running.stream(&Unattended, |chunk| {
        pixels.extend_from_slice(chunk);
        Ok(())
    })?;
    running.finish()?;
    pixels.truncate(expected);
    let image = RgbaImage::from_raw(out_w, out_h, pixels)
        .ok_or_else(|| MediaError::decode(path, "FFmpeg returned too little picture data"))?;
    Ok(Picture::still(format, w, h, image))
}
