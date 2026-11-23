//! frame accurate access to video frames, for scrubbing and for picking exact frames.
//!
//! a [`FrameIndex`] lists when every frame is shown, read once from the packet headers
//! (no decoding, so it is quick even for long files) and cached. with it, "frame 1234" is an
//! exact timestamp even in variable frame rate phone videos, where frame number times frame
//! duration drifts. ffmpeg then seeks to the keyframe before it and decodes forward to that
//! exact frame.
//!
//! this spawns one ffmpeg per frame, which is right for picking frames and thumbnails but
//! too slow for smooth playback. phase 8 adds a long running decoder for that behind the
//! same index. see `ARCHITECTURE.md` section 22.

use std::path::Path;
use std::time::Duration;

use ::image::RgbaImage;

use crate::control::{Control, Unattended};
use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::Ffmpeg;
use crate::ffmpeg::process::{self, Running};

#[derive(Debug, Clone, PartialEq)]
pub struct FrameIndex {
    /// presentation time of every frame in seconds, in order
    pub times: Vec<f64>,
    /// the file's start time, which ffmpeg seeks relative to
    pub start: f64,
}

const MAGIC: &[u8; 4] = b"VFIX";

impl FrameIndex {
    /// reads the timestamp of every video packet. packets marked for discard (the hidden
    /// frames an mp4 edit list skips) are left out, as players do.
    pub fn build(ffmpeg: &Ffmpeg, path: &Path, control: &dyn Control) -> MediaResult<Self> {
        let start = start_time(ffmpeg, path)?;
        let mut cmd = ffmpeg.probe_command();
        cmd.args(["-select_streams", "v:0"])
            .args([
                "-show_entries",
                "packet=pts_time,dts_time,flags",
                "-of",
                "csv=p=0",
            ])
            .arg(path);
        let mut running = Running::spawn(cmd)?;
        let mut text = Vec::new();
        running.stream(control, |chunk| {
            text.extend_from_slice(chunk);
            Ok(())
        })?;
        running.finish()?;
        let index = Self::parse(&String::from_utf8_lossy(&text), start);
        if index.times.is_empty() {
            return Err(MediaError::decode(path, "it has no video frames"));
        }
        Ok(index)
    }

    /// `pts_time,dts_time,flags` per line, eg `0.033000,0.000000,__`
    pub fn parse(text: &str, start: f64) -> Self {
        let mut times: Vec<f64> = text
            .lines()
            .filter_map(|line| {
                let mut parts = line.trim().split(',');
                let pts = parts.next()?.parse::<f64>().ok();
                let dts = parts.next().and_then(|d| d.parse::<f64>().ok());
                let flags = parts.next().unwrap_or("");
                if flags.contains('D') {
                    return None;
                }
                pts.or(dts).filter(|t| t.is_finite())
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
        Self { times, start }
    }

    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    pub fn time_of(&self, frame: usize) -> Option<f64> {
        self.times.get(frame).copied()
    }

    /// the frame on screen at `time`: the last one that started at or before it
    pub fn frame_at(&self, time: f64) -> usize {
        self.times
            .partition_point(|t| *t <= time + 1e-6)
            .saturating_sub(1)
    }

    /// where to ask ffmpeg to seek to land exactly on `frame`: halfway between it and the
    /// frame before, so rounding in either direction cannot pick the wrong neighbour
    pub fn seek_target(&self, frame: usize) -> Option<f64> {
        let time = self.time_of(frame)?;
        let before = match frame {
            0 => time - 0.002,
            n => self.times[n - 1],
        };
        Some(((before + time) / 2.0 - self.start).max(0.0))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.times.len() * 8);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.start.to_le_bytes());
        out.extend_from_slice(&(self.times.len() as u64).to_le_bytes());
        for t in &self.times {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let rest = bytes.strip_prefix(MAGIC)?;
        let start = f64::from_le_bytes(rest.get(..8)?.try_into().ok()?);
        let count = u64::from_le_bytes(rest.get(8..16)?.try_into().ok()?);
        let body = rest.get(16..)?;
        if body.len() as u64 != count.checked_mul(8)? {
            return None;
        }
        let times = body
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c))
            .collect();
        Some(Self { times, start })
    }
}

fn start_time(ffmpeg: &Ffmpeg, path: &Path) -> MediaResult<f64> {
    let mut cmd = ffmpeg.probe_command();
    cmd.args(["-show_entries", "format=start_time", "-of", "csv=p=0"])
        .arg(path);
    let out = process::capture(cmd, Duration::from_secs(20))?;
    Ok(String::from_utf8_lossy(&out)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|t| t.is_finite())
        .unwrap_or(0.0))
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub index: usize,
    pub time: f64,
    pub image: RgbaImage,
}

/// decodes exactly frame `frame`, scaled to `size` (width and height after rotation)
pub fn read_frame(
    ffmpeg: &Ffmpeg,
    path: &Path,
    index: &FrameIndex,
    frame: usize,
    size: (u32, u32),
) -> MediaResult<VideoFrame> {
    let (Some(time), Some(target)) = (index.time_of(frame), index.seek_target(frame)) else {
        return Err(MediaError::Unsupported(format!(
            "Frame {frame} is past the end of the video, which has {} frames.",
            index.len()
        )));
    };
    let image = grab(ffmpeg, path, target, true, size)?;
    Ok(VideoFrame {
        index: frame,
        time,
        image,
    })
}

/// one frame near `seconds` from the start. `accurate` decodes forward from the keyframe
/// to the exact frame; without it the nearest keyframe is returned, which is much faster
/// and fine for thumbnails and filmstrips.
pub fn grab(
    ffmpeg: &Ffmpeg,
    path: &Path,
    seconds: f64,
    accurate: bool,
    size: (u32, u32),
) -> MediaResult<RgbaImage> {
    if accurate {
        return grab_with(ffmpeg, path, seconds, &[], size);
    }
    // decoding keyframes only skips the work frame threading would start on the frames
    // around the seek point, about 1.5 times faster for 4k hevc. ffmpeg then returns the
    // first keyframe after the seek point, and near the end of a file there may be none,
    // so then fall back to a plain fast seek.
    match grab_with(
        ffmpeg,
        path,
        seconds,
        &["-skip_frame", "nokey", "-noaccurate_seek"],
        size,
    ) {
        Ok(image) => Ok(image),
        Err(MediaError::Decode(_)) => grab_with(ffmpeg, path, seconds, &["-noaccurate_seek"], size),
        Err(err) => Err(err),
    }
}

fn grab_with(
    ffmpeg: &Ffmpeg,
    path: &Path,
    seconds: f64,
    input_args: &[&str],
    (width, height): (u32, u32),
) -> MediaResult<RgbaImage> {
    let mut cmd = ffmpeg.command();
    cmd.args(input_args)
        .args(["-ss", &format!("{:.6}", seconds.max(0.0))])
        .arg("-i")
        .arg(path)
        .args(["-map", "0:v:0", "-frames:v", "1"])
        .args(["-vf", &format!("scale={width}:{height}:flags=bilinear")])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    let expected = (width as usize) * (height as usize) * 4;
    let mut pixels = Vec::with_capacity(expected);
    let mut running = Running::spawn(cmd)?;
    running.stream(&Unattended, |chunk| {
        pixels.extend_from_slice(chunk);
        Ok(())
    })?;
    running.finish()?;
    if pixels.len() < expected {
        return Err(MediaError::decode(
            path,
            format!("there is no frame at {seconds:.2} seconds"),
        ));
    }
    pixels.truncate(expected);
    RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| MediaError::decode(path, "the frame had the wrong size"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_are_sorted_and_discarded_ones_dropped() {
        let text =
            "0.066,0.000,__\n0.000,-0.033,K_\n-0.033,-0.066,_D\n0.033,0.033,__\nN/A,0.100,__\n";
        let index = FrameIndex::parse(text, 0.0);
        assert_eq!(index.times, vec![0.0, 0.033, 0.066, 0.1]);
    }

    #[test]
    fn frames_are_found_by_time_and_targets_sit_between_frames() {
        let index = FrameIndex {
            times: vec![1.0, 1.04, 1.08, 1.1],
            start: 1.0,
        };
        assert_eq!(index.frame_at(1.05), 1);
        assert_eq!(index.frame_at(1.08), 2);
        assert_eq!(index.frame_at(0.5), 0);
        assert_eq!(index.frame_at(9.0), 3);
        let target = index.seek_target(2).unwrap();
        assert!((target - 0.06).abs() < 1e-9, "{target}");
        assert_eq!(index.seek_target(0), Some(0.0));
        assert_eq!(index.seek_target(4), None);
    }

    #[test]
    fn the_index_round_trips_through_bytes() {
        let index = FrameIndex {
            times: vec![0.0, 0.5, 1.25],
            start: 0.1,
        };
        assert_eq!(FrameIndex::from_bytes(&index.to_bytes()), Some(index));
        assert_eq!(FrameIndex::from_bytes(b"VFIXshort"), None);
        assert_eq!(FrameIndex::from_bytes(b"nope"), None);
    }
}
