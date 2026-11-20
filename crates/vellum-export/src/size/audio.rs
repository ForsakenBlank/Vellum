//! fitting a song or recording under a size: the bitrate follows from the length, then the
//! file is checked and encoded again a step lower if it came out over.

use std::path::Path;

use vellum_media::control::Span;
use vellum_media::ffmpeg::encode::{self, AudioCodec, AudioEncode, EncodeRequest, StreamPlan};
use vellum_media::probe::MediaInfo;
use vellum_media::{Control, MediaError, MediaResult};

use super::budget::{self, human_bytes, human_duration};
use super::{AudioTarget, SizeJob, already_fits, too_big};
use crate::{Env, ToolResult, naming};

/// the bitrates mp3 allows, best first
const MP3_RATES: [u32; 17] = [
    320, 256, 224, 192, 160, 128, 112, 96, 80, 64, 56, 48, 40, 32, 24, 16, 8,
];

/// the bitrate to use for `wanted` kbps, or none if even the lowest is too much
pub fn pick_kbps(format: AudioTarget, wanted: f64) -> Option<u32> {
    match format {
        AudioTarget::Mp3 => MP3_RATES
            .iter()
            .copied()
            .find(|r| f64::from(*r) <= wanted * 0.98),
        AudioTarget::Opus => {
            // opus goes as low as 6 kbps and sounds fine for speech at 16
            let kbps = (wanted * 0.97).floor().min(256.0) as u32;
            (kbps >= 6).then_some(kbps)
        }
    }
}

/// mp3 below 32 kbps only exists at lower sample rates (mpeg 2 and 2.5), and lame quietly
/// raises the bitrate if the rate is left at 44.1 khz
fn mp3_sample_rate(kbps: u32) -> Option<u32> {
    match kbps {
        0..=8 => Some(8000),
        9..=16 => Some(16000),
        17..=31 => Some(22050),
        _ => None,
    }
}

/// mono below these bitrates sounds better than thin stereo
fn channels_for(format: AudioTarget, kbps: u32, source: u32) -> u32 {
    let mono_below = match format {
        AudioTarget::Mp3 => 64,
        AudioTarget::Opus => 24,
    };
    if kbps < mono_below {
        1
    } else {
        source.clamp(1, 2)
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
    if !options.trimmed() && !options.mono && info.size_bytes <= goal {
        return Ok(already_fits(input, goal));
    }
    let full = info.duration_secs.unwrap_or(0.0);
    let start = options.start.unwrap_or(0.0).clamp(0.0, full.max(0.0));
    let end = options.end.unwrap_or(full).clamp(start, full.max(start));
    let duration = end - start;
    if duration <= 0.0 {
        return Err(MediaError::Unsupported(format!(
            "Vellum cannot read how long {} is.",
            info.name
        )));
    }
    let format = options.audio_format;
    let wanted = budget::audio_kbps(goal, duration);
    let Some(mut kbps) = pick_kbps(format, wanted) else {
        return Err(MediaError::Unsupported(format!(
            "{} is too small for {} of sound. Pick a bigger size or trim it.",
            human_bytes(goal),
            human_duration(duration)
        )));
    };
    let source_channels = info.audio.first().map_or(2, |a| a.channels);
    let (codec, ext) = match format {
        AudioTarget::Mp3 => (AudioCodec::Mp3, "mp3"),
        AudioTarget::Opus => (AudioCodec::Opus, "ogg"),
    };
    let output = naming::output_for(input, env.output_folder, Some(&job.label), ext);
    let mut request = EncodeRequest::new(input, output.path());
    if options.trimmed() {
        request.start = Some(start);
        request.end = Some(end);
    }
    request.duration = Some(full);
    request.video = StreamPlan::Drop;
    if format == AudioTarget::Opus {
        // keeps the average close to the bitrate asked for
        request.extra_args = vec!["-vbr".into(), "constrained".into()];
    }

    for attempt in 0..3 {
        let mut channels = channels_for(format, kbps, source_channels);
        if options.mono {
            channels = 1;
        }
        request.audio = StreamPlan::Encode {
            settings: AudioEncode {
                codec,
                kbps: Some(kbps),
                channels: Some(channels),
                sample_rate: if format == AudioTarget::Mp3 {
                    mp3_sample_rate(kbps)
                } else {
                    None
                },
            },
        };
        let span = Span {
            inner: control,
            start: if attempt == 0 { 0.0 } else { 0.9 },
            end: if attempt == 0 { 0.9 } else { 1.0 },
        };
        encode::encode(ffmpeg, env.hardware, &request, &span)?;
        let bytes = std::fs::metadata(output.path())
            .map_err(|err| MediaError::io(output.path(), err))?
            .len();
        if bytes <= goal {
            let mut result = ToolResult::file(input, output.path());
            result.target_bytes = Some(goal);
            result.changes.push(format!(
                "Encoded at {kbps} kbps{}.",
                if channels == 1 && source_channels > 1 {
                    " in mono"
                } else {
                    ""
                }
            ));
            return Ok(result);
        }
        let ratio = goal as f64 / bytes as f64;
        match pick_kbps(format, f64::from(kbps) * ratio * 0.97).filter(|k| *k < kbps) {
            Some(next) => kbps = next,
            None => break,
        }
    }
    let _ = std::fs::remove_file(output.path());
    Err(too_big(goal, &info.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrates_snap_to_what_the_format_allows() {
        assert_eq!(pick_kbps(AudioTarget::Mp3, 400.0), Some(320));
        assert_eq!(pick_kbps(AudioTarget::Mp3, 131.0), Some(128));
        // a little headroom, so 128.5 is not quite enough for 128
        assert_eq!(pick_kbps(AudioTarget::Mp3, 128.5), Some(112));
        assert_eq!(pick_kbps(AudioTarget::Mp3, 5.0), None);
        assert_eq!(pick_kbps(AudioTarget::Opus, 50.0), Some(48));
        assert_eq!(pick_kbps(AudioTarget::Opus, 1000.0), Some(256));
        assert_eq!(pick_kbps(AudioTarget::Opus, 5.0), None);
        assert_eq!(channels_for(AudioTarget::Mp3, 48, 2), 1);
        assert_eq!(channels_for(AudioTarget::Mp3, 128, 6), 2);
    }
}
