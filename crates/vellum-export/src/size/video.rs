//! fitting a video under a size: plan the bitrates ([`super::budget`]), encode in two passes
//! with a software encoder (the only way to land on a size reliably), check the file, and
//! if it came out over, run the second pass again at a corrected bitrate.

use std::path::Path;

use vellum_media::control::Span;
use vellum_media::ffmpeg::encode::{
    self, AudioCodec, AudioEncode, EncodeRequest, Passes, StreamPlan, VideoEncode, VideoQuality,
    supports_two_pass,
};
use vellum_media::ffmpeg::hw::VideoCodec;
use vellum_media::probe::MediaInfo;
use vellum_media::{Control, MediaError, MediaResult};

use super::budget::{self, VideoBudget, VideoSource};
use super::{SizeJob, already_fits, too_big};
use crate::{Env, ToolResult, naming};

/// encodes after the first that may be needed to get under
const CORRECTIONS: usize = 3;

/// roughly how many bits each codec needs for the same picture as h.264
fn efficiency(codec: VideoCodec) -> f64 {
    match codec {
        VideoCodec::H264 => 1.0,
        VideoCodec::H265 => 0.65,
        VideoCodec::Vp9 => 0.7,
        VideoCodec::Av1 => 0.55,
    }
}

pub fn fit(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    job: &SizeJob,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let ffmpeg = env.ffmpeg()?;
    let options = job.options;
    let goal = job.goal;
    let video = info.video.as_ref().ok_or_else(|| {
        MediaError::Unsupported(format!("{} has no picture to shrink.", info.name))
    })?;
    if !options.trimmed() && !options.mute && !options.mono && info.size_bytes <= goal {
        return Ok(already_fits(input, goal));
    }
    let full = info.duration_secs.unwrap_or(0.0);
    let start = options.start.unwrap_or(0.0).clamp(0.0, full.max(0.0));
    let end = options.end.unwrap_or(full).clamp(start, full.max(start));
    let duration = end - start;
    let (Some(width), Some(height)) = (info.width, info.height) else {
        return Err(MediaError::Unsupported(format!(
            "Vellum cannot read the size of the picture in {}.",
            info.name
        )));
    };

    let source = VideoSource {
        width,
        height,
        fps: video.frame_rate,
        duration,
        has_audio: !info.audio.is_empty(),
        audio_channels: info.audio.first().map_or(2, |a| a.channels),
    };
    let plan = budget::plan_video(
        &source,
        goal,
        &VideoBudget {
            strategy: options.strategy,
            efficiency: efficiency(options.codec),
            mute: options.mute,
            mono: options.mono,
        },
    )
    .map_err(|err| MediaError::Unsupported(err.to_string()))?;

    let software = env.hardware.software(options.codec).ok_or_else(|| {
        MediaError::Unsupported(format!(
            "This copy of FFmpeg cannot encode {}. Pick another codec.",
            encode::codec_label(options.codec)
        ))
    })?;
    let two_pass = supports_two_pass(&software.name);
    let webm = options.codec == VideoCodec::Vp9;
    let output = naming::output_for(
        input,
        env.output_folder,
        Some(&job.label),
        if webm { "webm" } else { "mp4" },
    );
    let logs = tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
    let log = logs.path().join("pass");

    let mut request = EncodeRequest::new(input, output.path());
    if options.trimmed() {
        request.start = Some(start);
        request.end = Some(end);
    }
    request.duration = Some(full);
    request.strip_metadata = true;
    request.allow_hardware = !two_pass && env.allow_hardware;
    request.passes = Passes::Two { log: log.clone() };
    request.audio = match plan.audio {
        Some(audio) => StreamPlan::Encode {
            settings: AudioEncode {
                codec: if webm {
                    AudioCodec::Opus
                } else {
                    AudioCodec::Aac
                },
                kbps: Some(audio.kbps),
                channels: Some(audio.channels),
                sample_rate: None,
            },
        },
        None => StreamPlan::Drop,
    };
    if options.codec == VideoCodec::H265 {
        // apple players only play hevc in mp4 under this tag
        request.extra_args = vec!["-tag:v".into(), "hvc1".into()];
    }
    let scaled = (plan.width, plan.height) != (width, height);
    let mut video_kbps = plan.video_kbps;
    let audio_kbps = plan.audio.map_or(0, |a| a.kbps);

    for attempt in 0..=CORRECTIONS {
        request.video = StreamPlan::Encode {
            settings: VideoEncode {
                codec: options.codec,
                quality: VideoQuality::Bitrate { kbps: video_kbps },
                width: scaled.then_some(plan.width),
                height: scaled.then_some(plan.height),
                fps: plan.fps,
                filters: Vec::new(),
            },
        };
        let span = if attempt == 0 {
            Span {
                inner: control,
                start: 0.0,
                end: 0.9,
            }
        } else {
            control.message("A little over, encoding again at a lower bitrate");
            request.passes = Passes::Second { log: log.clone() };
            Span {
                inner: control,
                start: 0.9,
                end: 1.0,
            }
        };
        let outcome = encode::encode(ffmpeg, env.hardware, &request, &span)?;
        let bytes = std::fs::metadata(output.path())
            .map_err(|err| MediaError::io(output.path(), err))?
            .len();
        tracing::debug!(attempt, video_kbps, bytes, goal, "size target attempt");
        if bytes <= goal {
            let mut result = ToolResult::file(input, output.path());
            result.target_bytes = Some(goal);
            result.changes = plan.changes.clone();
            if let Some(reason) = outcome.fell_back {
                result.notes.push(format!(
                    "The hardware encoder failed, so the processor was used: {reason}"
                ));
            }
            return Ok(result);
        }
        video_kbps = budget::corrected_kbps(video_kbps, bytes, goal, audio_kbps, duration);
        if f64::from(video_kbps) < budget::MIN_VIDEO_KBPS {
            break;
        }
    }
    let _ = std::fs::remove_file(output.path());
    Err(too_big(goal, &info.name))
}
