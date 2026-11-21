//! the quick tools from phase 3: export to a size, video to gif, the gif tools and the
//! converter. each tool is a plain function over files that reports through a
//! [`Control`], so it runs the same in a job, in the cli and in tests. [`service`] turns
//! a quick tool document into jobs for the app.
//!
//! - [`size`]: fits videos, images, gifs and audio under a size limit
//! - [`anim`]: video to gif (and webp, apng, avif, mp4 loops), captions, lossy gifs, the
//!   gif frame editor
//! - [`convert`]: any format to any format, remuxing when nothing needs re-encoding
//! - [`parts`]: joining files into one and cutting one into parts
//! - [`naming`]: where results go, never over an existing file
//!
//! see `ARCHITECTURE.md` section 23 for the design.

pub mod anim;
pub mod convert;
pub mod naming;
pub mod parts;
pub mod service;
pub mod size;

use std::path::Path;
use std::process::Command;

use serde::Serialize;
use ts_rs::TS;
use vellum_media::ffmpeg::Ffmpeg;
use vellum_media::ffmpeg::hw::HardwareSupport;
use vellum_media::ffmpeg::process::{ProgressParser, Running};
use vellum_media::{Control, MediaError, MediaResult};

/// what every tool needs from the running app
#[derive(Clone, Copy)]
pub struct Env<'a> {
    pub ffmpeg: Option<&'a Ffmpeg>,
    pub hardware: &'a HardwareSupport,
    /// from settings, whether hardware encoders may be used
    pub allow_hardware: bool,
    /// where results go, none puts them next to the source
    pub output_folder: Option<&'a Path>,
}

impl Env<'_> {
    pub fn ffmpeg(&self) -> MediaResult<&Ffmpeg> {
        self.ffmpeg.ok_or(MediaError::FfmpegMissing)
    }
}

/// the av1 encoder for avif pictures. libaom first: svt-av1 is faster but refuses pictures
/// under 64 pixels, which icons and small stickers often are.
pub fn avif_encoder(env: &Env) -> MediaResult<&'static str> {
    let has = |name: &str| env.hardware.encoders.iter().any(|e| e.name == name);
    if has("libaom-av1") {
        Ok("libaom-av1")
    } else if has("libsvtav1") {
        Ok("libsvtav1")
    } else {
        Err(MediaError::Unsupported(
            "This copy of FFmpeg cannot make AVIF files. Pick another format or install a fuller FFmpeg build.".to_owned(),
        ))
    }
}

/// what a finished tool run hands back. `path` is what "open" and "show in folder" use.
#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolResult {
    pub path: String,
    /// the source this came from
    pub source: String,
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub source_bytes: Option<u64>,
    /// the size the result had to stay under, for export to size
    #[ts(type = "number | null")]
    pub target_bytes: Option<u64>,
    /// what was changed to get there, in plain words
    pub changes: Vec<String>,
    /// suggestions, eg "as an mp4 this would be 4 times smaller"
    pub notes: Vec<String>,
    /// the source already fit, so nothing was written and `path` is the source
    pub already_fits: bool,
    /// true when `path` is a folder of files, like extracted frames
    pub folder: bool,
}

impl ToolResult {
    pub fn file(source: &Path, path: &Path) -> Self {
        Self {
            path: path.display().to_string(),
            source: source.display().to_string(),
            bytes: std::fs::metadata(path).ok().map(|m| m.len()),
            source_bytes: std::fs::metadata(source).ok().map(|m| m.len()),
            ..Self::default()
        }
    }
}

/// runs ffmpeg with `args`, which must end with `-progress pipe:1 ... output`. progress is
/// read across `seconds` of output.
pub fn run_ffmpeg(
    ffmpeg: &Ffmpeg,
    args: &[String],
    seconds: Option<f64>,
    control: &dyn Control,
) -> MediaResult<()> {
    let mut cmd: Command = ffmpeg.command();
    cmd.args(args);
    let parser = ProgressParser::new(seconds);
    let mut running = Running::spawn(cmd)?;
    running.lines(control, |line| {
        if let Some(fraction) = parser.and_then(|p| p.parse(line)) {
            control.progress(fraction);
        }
    })?;
    running.finish()
}

/// the arguments every output ends with: progress on stdout, overwrite the partial file
pub fn output_args(path: &Path) -> Vec<String> {
    vec![
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        "-y".into(),
        path.display().to_string(),
    ]
}

/// writes `bytes` to `path` by way of a temporary name, so a crash never leaves half a file
pub fn write_atomic(path: &Path, bytes: &[u8]) -> MediaResult<()> {
    let partial = vellum_media::ffmpeg::encode::partial_path(path);
    std::fs::write(&partial, bytes).map_err(|err| MediaError::io(&partial, err))?;
    std::fs::rename(&partial, path).map_err(|err| {
        let _ = std::fs::remove_file(&partial);
        MediaError::io(path, err)
    })
}

/// moves a finished partial file onto its real name
pub fn finish_partial(partial: &Path, path: &Path) -> MediaResult<()> {
    std::fs::rename(partial, path).map_err(|err| {
        let _ = std::fs::remove_file(partial);
        MediaError::io(path, err)
    })
}

/// a number for an ffmpeg argument without trailing noise: 1.5, 30, 0.333333
pub fn num(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(30.0), "30");
        assert_eq!(num(1.0 / 3.0), "0.333333");
        assert_eq!(num(0.0), "0");
    }
}
