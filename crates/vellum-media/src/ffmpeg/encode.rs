//! running an encode: progress, cancel and pause through the job, the output written under
//! a temporary name and renamed at the end (so a cancelled or failed encode never leaves a
//! broken file behind), and a hardware encoder that fails part way falling back to the next
//! encoder in the list, ending with software.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::{FfmpegTools, Reporter, run_lines};
use crate::error::{MediaError, MediaResult};

/// one encode, described as ffmpeg arguments
#[derive(Debug, Clone, PartialEq)]
pub struct Encode {
    /// everything before the encoder choice, eg `-i in.mp4 -vf scale=1280:-2`
    pub input_args: Vec<OsString>,
    /// video encoders to try in order, eg `["hevc_nvenc", "libx265"]`. empty when the
    /// arguments already say how to handle video (or there is none).
    pub video_encoders: Vec<String>,
    /// everything after the encoder choice, shared by every try, eg `-b:v 2M -c:a aac`
    pub output_args: Vec<OsString>,
    /// where the result goes. it must not exist yet: vellum never overwrites.
    pub output: PathBuf,
    /// the length of the result in seconds, for progress
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeOutcome {
    /// the video encoder that did the work
    pub encoder: Option<String>,
    /// true when an earlier encoder in the list failed and a later one took over
    pub fell_back: bool,
}

/// runs the encode, trying each video encoder in turn
pub fn run_encode(
    tools: &FfmpegTools,
    encode: &Encode,
    reporter: &dyn Reporter,
) -> MediaResult<EncodeOutcome> {
    if encode.output.exists() {
        return Err(MediaError::Unsupported(format!(
            "{} already exists, and Vellum never overwrites files.",
            encode.output.display()
        )));
    }
    let temp = part_path(&encode.output);
    let encoders: Vec<Option<&String>> = if encode.video_encoders.is_empty() {
        vec![None]
    } else {
        encode.video_encoders.iter().map(Some).collect()
    };
    let mut last_error = None;
    for (attempt, encoder) in encoders.iter().enumerate() {
        if attempt > 0 {
            reporter.message("The graphics card could not encode this, using the processor");
            reporter.progress(0.0);
        }
        let mut cmd = tools.ffmpeg();
        cmd.args(&encode.input_args);
        if let Some(encoder) = encoder {
            cmd.args(["-c:v", encoder]);
        }
        cmd.args(&encode.output_args)
            .args(["-progress", "pipe:1", "-nostats", "-y"])
            .arg(&temp);
        let result = run_lines(&mut cmd, "ffmpeg", reporter, |line| {
            if let Some(fraction) = progress_line(line, encode.duration_secs) {
                reporter.progress(fraction);
            }
        });
        match result {
            Ok(()) => {
                fs::rename(&temp, &encode.output)
                    .map_err(|err| MediaError::io(&encode.output, err))?;
                return Ok(EncodeOutcome {
                    encoder: encoder.cloned(),
                    fell_back: attempt > 0,
                });
            }
            Err(err) => {
                let _ = fs::remove_file(&temp);
                if matches!(err, MediaError::Cancelled) {
                    return Err(err);
                }
                tracing::warn!(encoder = ?encoder, %err, "encode failed");
                last_error = Some(err);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| MediaError::tool("ffmpeg", "nothing was encoded")))
}

/// `clip.mp4` is written as `.clip.part.mp4` first. the extension stays last so ffmpeg
/// still picks the right container from it.
fn part_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let name = match output.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!(".{stem}.part.{ext}"),
        None => format!(".{stem}.part"),
    };
    output.with_file_name(name)
}

/// reads `out_time_us=1234567` from `-progress` output as a fraction of the duration
pub fn progress_line(line: &str, duration_secs: Option<f64>) -> Option<f32> {
    let total = duration_secs.filter(|d| *d > 0.0)?;
    let micros: f64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    Some((micros / 1e6 / total).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use parking_lot::Mutex;

    use super::*;
    use crate::ffmpeg::{probe_media, testing};

    /// remembers progress and can cancel once some has been made
    #[derive(Default)]
    struct Recorder {
        progress: Mutex<Vec<f32>>,
        messages: Mutex<Vec<String>>,
        cancel_after_progress: bool,
        cancelled: AtomicBool,
    }

    impl Reporter for Recorder {
        fn cancelled(&self) -> bool {
            self.cancelled.load(Ordering::SeqCst)
        }
        fn pause_requested(&self) -> bool {
            false
        }
        fn checkpoint(&self) -> MediaResult<()> {
            if self.cancelled() {
                Err(MediaError::Cancelled)
            } else {
                Ok(())
            }
        }
        fn progress(&self, fraction: f32) {
            self.progress.lock().push(fraction);
            if self.cancel_after_progress && fraction > 0.0 {
                self.cancelled.store(true, Ordering::SeqCst);
            }
        }
        fn message(&self, text: &str) {
            self.messages.lock().push(text.to_owned());
        }
    }

    fn os(args: &str) -> Vec<OsString> {
        args.split_whitespace().map(OsString::from).collect()
    }

    #[test]
    fn progress_lines_turn_into_fractions() {
        assert_eq!(progress_line("out_time_us=5000000", Some(10.0)), Some(0.5));
        assert_eq!(progress_line("out_time_us=50000000", Some(10.0)), Some(1.0));
        assert_eq!(progress_line("out_time_us=N/A", Some(10.0)), None);
        assert_eq!(progress_line("frame=12", Some(10.0)), None);
        assert_eq!(progress_line("out_time_us=1", None), None);
        assert_eq!(
            part_path(Path::new("/a/clip.mp4")),
            PathBuf::from("/a/.clip.part.mp4")
        );
    }

    #[test]
    fn a_failing_encoder_falls_back_to_the_next_one() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.mp4");
        let encode = Encode {
            input_args: os("-f lavfi -i testsrc2=s=320x240:r=25:d=3"),
            // stands in for a hardware encoder whose driver is missing
            video_encoders: vec!["h264_not_a_real_encoder".into(), "libx264".into()],
            output_args: os("-pix_fmt yuv420p -preset ultrafast"),
            output: output.clone(),
            duration_secs: Some(3.0),
        };
        let recorder = Recorder::default();
        let outcome = run_encode(&tools, &encode, &recorder).unwrap();
        assert_eq!(outcome.encoder.as_deref(), Some("libx264"));
        assert!(outcome.fell_back);
        assert!(!recorder.messages.lock().is_empty());
        assert!(recorder.progress.lock().iter().any(|p| *p > 0.5));
        let info = probe_media(&tools, &output).unwrap();
        assert_eq!(info.video.unwrap().codec, "h264");
        // only the result is left, no part files
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        // and it is never overwritten
        assert!(run_encode(&tools, &encode, &recorder).is_err());
    }

    #[test]
    fn cancelling_stops_ffmpeg_and_leaves_nothing_behind() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let encode = Encode {
            // long and slow enough that cancel lands mid encode
            input_args: os("-f lavfi -i testsrc2=s=1280x720:r=30:d=120"),
            video_encoders: vec!["libx264".into()],
            output_args: os("-preset veryslow"),
            output: dir.path().join("long.mp4"),
            duration_secs: Some(120.0),
        };
        let recorder = Recorder {
            cancel_after_progress: true,
            ..Recorder::default()
        };
        let err = run_encode(&tools, &encode, &recorder).unwrap_err();
        assert!(matches!(err, MediaError::Cancelled), "{err}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
