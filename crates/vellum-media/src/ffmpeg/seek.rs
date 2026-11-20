//! frame accurate seeking.
//!
//! a [`FrameIndex`] lists the presentation time of every frame in a video stream, read
//! from the packet headers (no decoding, so it is quick even for long files). with it,
//! "frame 1234" becomes an exact time, which works for variable frame rate video too,
//! where `frame / fps` drifts.
//!
//! [`extract_frame`] then asks ffmpeg for the frame at that time. it seeks to half a frame
//! before the frame's own timestamp: ffmpeg decodes from the keyframe before it and drops
//! every frame that starts before the seek point, so the first frame out is the one asked
//! for, without any rounding risk at the exact boundary.

use std::path::Path;

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::process::read_full;
use super::{FfmpegTools, Reporter, run_lines, run_output, run_reading};
use crate::error::{MediaError, MediaResult};
use crate::info::Rational;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameIndex {
    pub stream_index: u32,
    pub time_base: Rational,
    /// the file's start time in seconds. ffmpeg measures `-ss` from here.
    pub file_start: f64,
    /// presentation timestamps in `time_base` units, in display order
    pub pts: Vec<i64>,
    /// positions in `pts` that are keyframes
    pub keyframes: Vec<u32>,
}

impl FrameIndex {
    /// reads the timestamps of every frame in `stream` (the first video stream when none).
    /// `expected_frames` only drives the progress bar.
    pub fn build(
        tools: &FfmpegTools,
        path: &Path,
        stream: Option<u32>,
        expected_frames: Option<u64>,
        reporter: &dyn Reporter,
    ) -> MediaResult<Self> {
        let selector = stream.map_or_else(|| "v:0".to_owned(), |s| s.to_string());
        let (stream_index, time_base, file_start) = stream_details(tools, path, &selector)?;

        let mut cmd = tools.ffprobe();
        cmd.args(["-select_streams", &selector])
            .args(["-show_entries", "packet=pts,dts,flags"])
            .args(["-of", "csv=p=0"])
            .arg(path);
        let mut frames: Vec<(i64, bool)> = Vec::new();
        run_lines(&mut cmd, "ffprobe", reporter, |line| {
            if let Some(frame) = parse_packet(line) {
                frames.push(frame);
                if let Some(total) = expected_frames.filter(|t| *t > 0)
                    && frames.len().is_multiple_of(500)
                {
                    reporter.progress((frames.len() as f64 / total as f64).min(0.99) as f32);
                }
            }
        })?;
        if frames.is_empty() {
            return Err(MediaError::Unsupported(
                "This video has no frame timestamps, so Vellum cannot seek in it. Convert it \
                 to MP4 first."
                    .into(),
            ));
        }
        frames.sort_unstable_by_key(|(pts, _)| *pts);
        frames.dedup_by_key(|(pts, _)| *pts);
        let keyframes = frames
            .iter()
            .enumerate()
            .filter(|(_, (_, key))| *key)
            .filter_map(|(i, _)| u32::try_from(i).ok())
            .collect();
        Ok(Self {
            stream_index,
            time_base,
            file_start,
            pts: frames.into_iter().map(|(pts, _)| pts).collect(),
            keyframes,
        })
    }

    pub fn len(&self) -> usize {
        self.pts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    fn seconds(&self, pts: i64) -> f64 {
        pts as f64 * self.time_base.num as f64 / self.time_base.den as f64
    }

    /// when `frame` appears, in seconds from the start of the file
    pub fn time_of(&self, frame: usize) -> Option<f64> {
        self.pts
            .get(frame)
            .map(|pts| self.seconds(*pts) - self.file_start)
    }

    /// the frame on screen at `secs` from the start of the file
    pub fn frame_at(&self, secs: f64) -> usize {
        let target = secs + self.file_start;
        let after = self
            .pts
            .partition_point(|pts| self.seconds(*pts) <= target + 1e-9);
        after.saturating_sub(1)
    }

    /// the keyframe decoding has to start from to show `frame`
    pub fn keyframe_before(&self, frame: usize) -> usize {
        let after = self.keyframes.partition_point(|k| (*k as usize) <= frame);
        after
            .checked_sub(1)
            .and_then(|i| self.keyframes.get(i))
            .map_or(0, |k| *k as usize)
    }
}

/// the `-ss` value that lands exactly on `frame`
pub fn frame_seek_time(index: &FrameIndex, frame: usize) -> Option<f64> {
    let time = index.time_of(frame)?;
    let gap = match frame.checked_sub(1).and_then(|prev| index.time_of(prev)) {
        Some(prev) => time - prev,
        // nothing before the first frame to land on by mistake
        None => 0.002,
    };
    Some((time - gap / 2.0).max(0.0))
}

/// what to pull out of a video
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameRequest {
    pub stream: u32,
    /// seconds from the start of the file
    pub seek: f64,
    /// true for the exact frame at `seek`; false for the nearest keyframe before it, which
    /// is many times faster and fine for thumbnails
    pub accurate: bool,
    /// output size. ffmpeg turns the picture upright first, so this is the shown size.
    pub width: u32,
    pub height: u32,
}

/// decodes one frame as rgba
pub fn extract_frame(
    tools: &FfmpegTools,
    path: &Path,
    req: &FrameRequest,
) -> MediaResult<RgbaImage> {
    let (width, height) = (req.width.max(1), req.height.max(1));
    let mut cmd = tools.ffmpeg();
    if !req.accurate {
        cmd.args(["-noaccurate_seek", "-skip_frame", "nokey"]);
    }
    cmd.args(["-ss", &format!("{:.6}", req.seek)])
        .arg("-i")
        .arg(path)
        .args(["-map", &format!("0:{}", req.stream)])
        .args(["-frames:v", "1", "-an", "-sn"])
        // raw output would otherwise use constant frame rate sync, which throws away the
        // keyframe before the seek point that a quick seek is asking for
        .args(["-fps_mode", "passthrough"])
        .args([
            "-vf",
            &format!("scale={width}:{height}:flags=bicubic,setsar=1"),
        ])
        .args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"]);
    let expected = width as usize * height as usize * 4;
    let bytes = run_reading(&mut cmd, "ffmpeg", |stdout| {
        let mut buf = vec![0u8; expected];
        let filled = read_full(stdout, &mut buf).map_err(|err| MediaError::io(path, err))?;
        if filled < expected {
            return Err(MediaError::decode(format!(
                "there is no frame at {:.3} seconds",
                req.seek
            )));
        }
        Ok(buf)
    })?;
    RgbaImage::from_raw(width, height, bytes)
        .ok_or_else(|| MediaError::decode("the frame had the wrong size"))
}

fn stream_details(
    tools: &FfmpegTools,
    path: &Path,
    selector: &str,
) -> MediaResult<(u32, Rational, f64)> {
    let mut cmd = tools.ffprobe();
    cmd.args(["-select_streams", selector])
        .args(["-show_entries", "stream=index,time_base:format=start_time"])
        .args(["-print_format", "json"])
        .arg(path);
    let out = run_output(&mut cmd, "ffprobe")?;
    let json: Value =
        serde_json::from_slice(&out).map_err(|err| MediaError::tool("ffprobe", err.to_string()))?;
    let stream = json
        .get("streams")
        .and_then(Value::as_array)
        .and_then(|s| s.first())
        .ok_or_else(|| MediaError::Unsupported("This file has no video to seek in.".into()))?;
    let index = stream
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|i| u32::try_from(i).ok())
        .unwrap_or(0);
    let time_base = stream
        .get("time_base")
        .and_then(Value::as_str)
        .and_then(Rational::parse)
        .filter(|tb| tb.num > 0 && tb.den > 0)
        .ok_or_else(|| MediaError::tool("ffprobe", "the video has no time base"))?;
    let start = json
        .pointer("/format/start_time")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    Ok((index, time_base, start))
}

/// "512,0,K__" gives (512, true). packets ffmpeg will throw away ("D" flag) are skipped.
fn parse_packet(line: &str) -> Option<(i64, bool)> {
    let mut parts = line.trim().split(',');
    let pts = parts.next()?.parse::<i64>().ok();
    let dts = parts.next()?.parse::<i64>().ok();
    let flags = parts.next().unwrap_or_default();
    if flags.contains('D') {
        return None;
    }
    Some((pts.or(dts)?, flags.starts_with('K')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{NoReporter, testing};

    fn index(pts: Vec<i64>, keyframes: Vec<u32>) -> FrameIndex {
        FrameIndex {
            stream_index: 0,
            time_base: Rational::new(1, 1000),
            file_start: 0.0,
            pts,
            keyframes,
        }
    }

    #[test]
    fn packets_parse_and_discarded_ones_are_skipped() {
        assert_eq!(parse_packet("512,0,K__"), Some((512, true)));
        assert_eq!(parse_packet("1024,512,___"), Some((1024, false)));
        assert_eq!(parse_packet("N/A,300,K__"), Some((300, true)));
        assert_eq!(parse_packet("-1024,-2048,_D_"), None);
        assert_eq!(parse_packet("N/A,N/A,K__"), None);
    }

    #[test]
    fn times_and_frames_convert_both_ways_with_uneven_timing() {
        // 30 fps, then a slow stretch: frames at 0, 33, 67, 100, 200, 300 ms
        let idx = index(vec![0, 33, 67, 100, 200, 300], vec![0, 4]);
        assert_eq!(idx.time_of(4), Some(0.2));
        assert_eq!(idx.frame_at(0.0), 0);
        assert_eq!(idx.frame_at(0.15), 3);
        assert_eq!(idx.frame_at(0.2), 4);
        assert_eq!(idx.frame_at(99.0), 5);
        assert_eq!(idx.keyframe_before(3), 0);
        assert_eq!(idx.keyframe_before(5), 4);
        // half way between frame 3 and 4
        assert!((frame_seek_time(&idx, 4).unwrap() - 0.15).abs() < 1e-9);
        assert!(frame_seek_time(&idx, 0).unwrap() <= 0.0);
    }

    #[test]
    fn the_file_start_offset_is_taken_out() {
        let mut idx = index(vec![1400, 1440, 1480], vec![0]);
        idx.file_start = 1.4;
        assert!((idx.time_of(1).unwrap() - 0.04).abs() < 1e-9);
        assert_eq!(idx.frame_at(0.05), 1);
    }

    /// every frame of the test clip is a flat grey whose level gives its frame number
    fn level_of(frame: usize) -> f64 {
        let luma = 16.0 + frame as f64 * 4.0;
        (luma - 16.0) * 255.0 / 219.0
    }

    #[test]
    fn every_frame_is_reached_exactly_through_b_frames_and_keyframe_gaps() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("numbered.mp4");
        testing::make(
            &tools,
            "-f lavfi -i color=c=black:s=64x48:r=25:d=2,geq=lum=16+N*4:cb=128:cr=128 \
             -c:v libx264 -qp 0 -g 12 -bf 2 -pix_fmt yuv420p",
            &path,
        );
        let idx = FrameIndex::build(&tools, &path, None, Some(50), &NoReporter).unwrap();
        assert_eq!(idx.len(), 50);
        assert_eq!(idx.keyframes.first(), Some(&0));
        for frame in [0, 1, 7, 11, 12, 13, 24, 25, 37, 49] {
            let req = FrameRequest {
                stream: idx.stream_index,
                seek: frame_seek_time(&idx, frame).unwrap(),
                accurate: true,
                width: 32,
                height: 24,
            };
            let image = extract_frame(&tools, &path, &req).unwrap();
            let got = f64::from(image.get_pixel(16, 12)[0]);
            assert!(
                (got - level_of(frame)).abs() <= 3.0,
                "frame {frame}: got level {got}, expected {}",
                level_of(frame)
            );
        }
    }

    #[test]
    fn keyframe_seeks_land_on_the_keyframe_before() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("numbered.mp4");
        testing::make(
            &tools,
            "-f lavfi -i color=c=black:s=64x48:r=25:d=2,geq=lum=16+N*4:cb=128:cr=128 \
             -c:v libx264 -qp 0 -g 12 -bf 0 -pix_fmt yuv420p",
            &path,
        );
        let req = FrameRequest {
            stream: 0,
            // frame 17 is between keyframes 12 and 24
            seek: 17.0 / 25.0,
            accurate: false,
            width: 32,
            height: 24,
        };
        let image = extract_frame(&tools, &path, &req).unwrap();
        let got = f64::from(image.get_pixel(16, 12)[0]);
        assert!((got - level_of(12)).abs() <= 3.0, "got {got}");
    }

    #[test]
    fn seeking_past_the_end_is_a_clear_error() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.mp4");
        testing::make(
            &tools,
            "-f lavfi -i testsrc2=s=64x48:r=10:d=1 -c:v libx264",
            &path,
        );
        let req = FrameRequest {
            stream: 0,
            seek: 30.0,
            accurate: true,
            width: 16,
            height: 12,
        };
        let err = extract_frame(&tools, &path, &req).unwrap_err();
        assert!(err.to_string().contains("no frame"), "{err}");
    }
}
