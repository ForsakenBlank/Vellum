//! the maths behind "export to x mb": how many bytes to aim for, how to split them between
//! picture and sound, and when a video has to get smaller or slower to still look good.
//!
//! everything here is a pure function so it can be tested without encoding anything, and
//! so the ui can show the plan before the encode starts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// share of the budget kept back for the container (indexes, headers), on top of the margin
const CONTAINER_SHARE: f64 = 0.02;
/// bytes kept back for the container however small the file
const CONTAINER_BYTES: f64 = 2048.0;
/// below this the picture falls apart whatever the size
pub const MIN_VIDEO_KBPS: f64 = 20.0;
/// bits per pixel per frame where h.264 still looks good. newer codecs need less.
const GOOD_BITS_PER_PIXEL: f64 = 0.045;
/// how much of the budget audio may take before it is turned down
const AUDIO_SHARE: f64 = 0.15;

/// what to keep when something has to give
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Strategy {
    /// picks resolution, frame rate and audio itself
    #[default]
    BestQuality,
    KeepResolution,
    KeepFps,
    KeepAudio,
}

/// the bytes to aim for: the limit minus the safety margin
pub fn target_bytes(limit: u64, margin_percent: f64) -> u64 {
    let keep = 1.0 - margin_percent.clamp(0.0, 50.0) / 100.0;
    // limits are far below 2^52, so the float is exact enough
    (limit as f64 * keep).floor() as u64
}

/// the bits the streams may use once the container has had its share
fn stream_bits(target: u64) -> f64 {
    (target as f64 * (1.0 - CONTAINER_SHARE) - CONTAINER_BYTES).max(0.0) * 8.0
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoSource {
    /// upright size
    pub width: u32,
    pub height: u32,
    /// none when the file does not say
    pub fps: Option<f64>,
    /// of the part being exported
    pub duration: f64,
    pub has_audio: bool,
    pub audio_channels: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoBudget {
    pub strategy: Strategy,
    /// 1 for h.264, lower for codecs that need fewer bits for the same picture
    pub efficiency: f64,
    pub mute: bool,
    pub mono: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioPlan {
    pub kbps: u32,
    pub channels: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VideoPlan {
    pub width: u32,
    pub height: u32,
    /// none keeps the source frame rate
    pub fps: Option<f64>,
    pub video_kbps: u32,
    pub audio: Option<AudioPlan>,
    /// what was changed to make it fit, in plain words
    pub changes: Vec<String>,
    /// the size is too small for the picture to look good even after scaling down
    pub low_quality: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BudgetError {
    #[error(
        "{target} is too small for {length} of video. Pick a bigger size, or trim it to about {fits} first."
    )]
    TooSmall {
        target: String,
        length: String,
        fits: String,
    },
    #[error("This video has no length Vellum can read, so it cannot work out a size for it.")]
    NoDuration,
}

/// works out size, frame rate and bitrates for a video to land on `target` bytes
pub fn plan_video(
    source: &VideoSource,
    target: u64,
    budget: &VideoBudget,
) -> Result<VideoPlan, BudgetError> {
    if !(source.duration.is_finite() && source.duration > 0.0) {
        return Err(BudgetError::NoDuration);
    }
    let total = stream_bits(target);
    let mut changes = Vec::new();

    let audio = if source.has_audio && !budget.mute {
        let plan = plan_audio_share(source, total, budget);
        if plan.channels == 1 && source.audio_channels > 1 {
            changes.push("Turned the sound to mono to save space.".to_owned());
        }
        if plan.kbps < 96 {
            changes.push(format!(
                "Lowered the sound to {} kbps to leave room for the picture.",
                plan.kbps
            ));
        }
        Some(plan)
    } else {
        None
    };
    let audio_bits = audio.map_or(0.0, |a| f64::from(a.kbps) * 1000.0 * source.duration);
    let video_bps = (total - audio_bits) / source.duration;
    if video_bps < MIN_VIDEO_KBPS * 1000.0 {
        let per_second = MIN_VIDEO_KBPS * 1000.0 + audio_bits / source.duration;
        let fits = total / per_second;
        return Err(BudgetError::TooSmall {
            target: human_bytes(target),
            length: human_duration(source.duration),
            fits: human_duration(fits.max(0.1)),
        });
    }

    let threshold = GOOD_BITS_PER_PIXEL * budget.efficiency;
    let candidates = candidates(source, budget.strategy);
    let good = candidates.iter().find(|c| {
        let pixels = f64::from(c.width) * f64::from(c.height) * c.fps;
        video_bps / pixels >= threshold
    });
    let (chosen, mut low_quality) = match good {
        Some(c) => (*c, false),
        // the smallest option is the best that can be done
        None => (candidates.last().copied().unwrap_or(candidates[0]), true),
    };
    // below 360p or 15 fps it fits, but nobody would call it good
    let short = chosen.width.min(chosen.height);
    if short < 360.min(source.width.min(source.height))
        || chosen.fps < 15.0_f64.min(source.fps.unwrap_or(30.0)) - 0.5
    {
        low_quality = true;
    }

    if (chosen.width, chosen.height) != (source.width, source.height) {
        changes.insert(
            0,
            format!(
                "Scaled down from {}×{} to {}×{} so it stays sharp at this size.",
                source.width, source.height, chosen.width, chosen.height
            ),
        );
    }
    let source_fps = source.fps.unwrap_or(chosen.fps);
    let fps = if (chosen.fps - source_fps).abs() > 0.5 {
        changes.push(format!(
            "Lowered the frame rate from {} to {} fps.",
            fps_label(source_fps),
            fps_label(chosen.fps)
        ));
        Some(chosen.fps)
    } else {
        None
    };
    if low_quality {
        changes.push(format!(
            "{} is very little for {} of video, so it will look soft. Trimming it would help.",
            human_bytes(target),
            human_duration(source.duration)
        ));
    }
    // bitrates are small positive numbers here
    let video_kbps = (video_bps / 1000.0).floor() as u32;
    Ok(VideoPlan {
        width: chosen.width,
        height: chosen.height,
        fps,
        video_kbps,
        audio,
        changes,
        low_quality,
    })
}

fn plan_audio_share(source: &VideoSource, total: f64, budget: &VideoBudget) -> AudioPlan {
    let stereo = source.audio_channels.clamp(1, 2);
    let ladder: [(u32, u32); 6] = [
        (128, stereo),
        (112, stereo),
        (96, stereo),
        (64, stereo),
        (48, 1),
        (32, 1),
    ];
    let share = if budget.strategy == Strategy::KeepAudio {
        0.5
    } else {
        AUDIO_SHARE
    };
    let fits = |kbps: u32| f64::from(kbps) * 1000.0 * source.duration <= total * share;
    let (kbps, channels) = ladder
        .iter()
        .copied()
        .find(|(kbps, _)| fits(*kbps))
        .unwrap_or((32, 1));
    let channels = if budget.mono { 1 } else { channels };
    AudioPlan { kbps, channels }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    width: u32,
    height: u32,
    fps: f64,
}

/// the sizes and frame rates to try, best first
fn candidates(source: &VideoSource, strategy: Strategy) -> Vec<Candidate> {
    let fps = source
        .fps
        .filter(|f| f.is_finite() && *f > 0.0)
        .unwrap_or(30.0);
    let short = source.width.min(source.height);
    let mut edges = vec![short];
    edges.extend(
        [1080, 720, 540, 480, 360, 240, 144]
            .into_iter()
            .filter(|e| *e < short),
    );
    let capped = fps.min(30.0);
    let slow: Vec<f64> = [24.0, 20.0, 15.0, 12.0, 10.0]
        .into_iter()
        .filter(|f| *f < capped - 0.5)
        .collect();
    let at = |edge: u32, fps: f64| {
        let (width, height) = scaled(source.width, source.height, edge);
        Candidate { width, height, fps }
    };

    let mut list = Vec::new();
    match strategy {
        Strategy::KeepResolution => {
            list.push(at(short, fps));
            if fps > 31.0 {
                list.push(at(short, 30.0));
            }
            list.extend(slow.iter().map(|f| at(short, *f)));
        }
        Strategy::KeepFps => list.extend(edges.iter().map(|e| at(*e, fps))),
        Strategy::BestQuality | Strategy::KeepAudio => {
            list.push(at(short, fps));
            if fps > 31.0 {
                list.push(at(short, 30.0));
            }
            // shrink down to 360 at up to 30 fps, then slow down, then shrink further
            for edge in edges.iter().skip(1).filter(|e| **e >= 360) {
                list.push(at(*edge, capped));
            }
            let floor = edges
                .iter()
                .copied()
                .filter(|e| *e >= 360)
                .min()
                .unwrap_or(short);
            list.extend(slow.iter().take(3).map(|f| at(floor, *f)));
            let last = slow.get(2).copied().unwrap_or(capped);
            for edge in edges.iter().filter(|e| **e < 360) {
                list.push(at(*edge, last));
            }
        }
    }
    list
}

/// scales so the short edge is `edge`, keeping the shape, with even sides for the encoders
pub fn scaled(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let even = |v: f64| ((v / 2.0).round() * 2.0).max(2.0) as u32;
    if width.min(height) <= edge {
        return (even(f64::from(width)), even(f64::from(height)));
    }
    let scale = f64::from(edge) / f64::from(width.min(height));
    (
        even(f64::from(width) * scale),
        even(f64::from(height) * scale),
    )
}

/// the next video bitrate after an encode came out at `actual` bytes instead of under
/// `target`, aiming a little lower than the straight ratio says
pub fn corrected_kbps(
    video_kbps: u32,
    actual: u64,
    target: u64,
    audio_kbps: u32,
    duration: f64,
) -> u32 {
    let audio_bytes = f64::from(audio_kbps) * 1000.0 / 8.0 * duration;
    let actual_video = (actual as f64 - audio_bytes).max(1.0);
    let wanted_video = (target as f64 * (1.0 - CONTAINER_SHARE) - audio_bytes).max(1.0);
    let ratio = (wanted_video / actual_video * 0.97).clamp(0.2, 0.99);
    let next = (f64::from(video_kbps) * ratio).floor() as u32;
    next.max(1)
}

/// the bitrate for a sound file of `duration` to land on `target`
pub fn audio_kbps(target: u64, duration: f64) -> f64 {
    stream_bits(target) / duration / 1000.0
}

/// "9.7 MB", "512 KB", "800 bytes". always decimal, like file managers on macos and phones.
pub fn human_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} bytes")
    }
}

/// "45 seconds", "3 minutes 20 seconds", "1 hour 5 minutes"
pub fn human_duration(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    if seconds < 10.0 {
        return format!("{seconds:.1} seconds");
    }
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    let unit = |n: u64, word: &str| {
        if n == 1 {
            format!("1 {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    match (h, m, s) {
        (0, 0, s) => unit(s, "second"),
        (0, m, 0) => unit(m, "minute"),
        (0, m, s) => format!("{} {}", unit(m, "minute"), unit(s, "second")),
        (h, 0, _) => unit(h, "hour"),
        (h, m, _) => format!("{} {}", unit(h, "hour"), unit(m, "minute")),
    }
}

fn fps_label(fps: f64) -> String {
    if (fps - fps.round()).abs() < 0.05 {
        format!("{fps:.0}")
    } else {
        format!("{fps:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(width: u32, height: u32, fps: f64, duration: f64) -> VideoSource {
        VideoSource {
            width,
            height,
            fps: Some(fps),
            duration,
            has_audio: true,
            audio_channels: 2,
        }
    }

    fn budget(strategy: Strategy) -> VideoBudget {
        VideoBudget {
            strategy,
            efficiency: 1.0,
            mute: false,
            mono: false,
        }
    }

    const MB: u64 = 1_000_000;

    #[test]
    fn the_margin_comes_off_the_limit() {
        assert_eq!(target_bytes(10 * MB, 3.0), 9_700_000);
        assert_eq!(target_bytes(10 * MB, 0.0), 10 * MB);
        // a silly margin is capped at half
        assert_eq!(target_bytes(10 * MB, 400.0), 5 * MB);
    }

    #[test]
    fn a_short_clip_keeps_everything() {
        let plan = plan_video(
            &source(1920, 1080, 30.0, 10.0),
            9_700_000,
            &budget(Strategy::BestQuality),
        )
        .unwrap();
        assert_eq!((plan.width, plan.height, plan.fps), (1920, 1080, None));
        assert_eq!(
            plan.audio,
            Some(AudioPlan {
                kbps: 128,
                channels: 2
            })
        );
        assert!(plan.changes.is_empty(), "{:?}", plan.changes);
        // 9.7 MB less the container and 160 KB of audio, over 10 seconds
        assert!(
            (7400..7600).contains(&plan.video_kbps),
            "{}",
            plan.video_kbps
        );
    }

    #[test]
    fn a_long_1080p60_clip_drops_frames_then_resolution() {
        let plan = plan_video(
            &source(1920, 1080, 60.0, 60.0),
            9_700_000,
            &budget(Strategy::BestQuality),
        )
        .unwrap();
        assert_eq!(plan.fps, Some(30.0));
        assert!(plan.height < 1080, "{plan:?}");
        assert!(
            plan.changes
                .iter()
                .any(|c| c.contains("frame rate from 60 to 30"))
        );
        assert!(plan.changes[0].starts_with("Scaled down from 1920×1080"));
        assert!(!plan.low_quality);
    }

    #[test]
    fn strategies_keep_what_they_promise() {
        let src = source(1920, 1080, 60.0, 120.0);
        let res = plan_video(&src, 9_700_000, &budget(Strategy::KeepResolution)).unwrap();
        assert_eq!((res.width, res.height), (1920, 1080));
        assert!(res.fps.unwrap() < 60.0);

        let fps = plan_video(&src, 9_700_000, &budget(Strategy::KeepFps)).unwrap();
        assert_eq!(fps.fps, None);
        assert!(fps.height < 1080);

        let audio = plan_video(&src, 9_700_000, &budget(Strategy::KeepAudio)).unwrap();
        let best = plan_video(&src, 9_700_000, &budget(Strategy::BestQuality)).unwrap();
        assert!(audio.audio.unwrap().kbps > best.audio.unwrap().kbps);
    }

    #[test]
    fn portrait_video_scales_by_its_short_edge() {
        let plan = plan_video(
            &source(1080, 1920, 30.0, 90.0),
            9_700_000,
            &budget(Strategy::BestQuality),
        )
        .unwrap();
        assert!(plan.width < plan.height);
        assert_eq!(plan.width % 2, 0);
        assert_eq!(plan.height % 2, 0);
    }

    #[test]
    fn sound_gives_way_on_long_clips_and_can_be_muted() {
        let long = plan_video(
            &source(1280, 720, 30.0, 600.0),
            9_700_000,
            &budget(Strategy::BestQuality),
        )
        .unwrap();
        let audio = long.audio.unwrap();
        assert!(audio.kbps <= 64, "{audio:?}");
        let mut muted = budget(Strategy::BestQuality);
        muted.mute = true;
        let plan = plan_video(&source(1280, 720, 30.0, 600.0), 9_700_000, &muted).unwrap();
        assert_eq!(plan.audio, None);
        assert!(plan.video_kbps > long.video_kbps);
    }

    #[test]
    fn far_too_small_says_how_long_would_fit() {
        let err = plan_video(
            &source(1920, 1080, 30.0, 3600.0),
            MB,
            &budget(Strategy::BestQuality),
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(
            text.starts_with("1.0 MB is too small for 1 hour of video"),
            "{text}"
        );
        assert!(text.contains("trim it to about"), "{text}");
    }

    #[test]
    fn very_tight_budgets_warn_about_quality() {
        let plan = plan_video(
            &source(1920, 1080, 30.0, 1200.0),
            9_700_000,
            &budget(Strategy::BestQuality),
        )
        .unwrap();
        assert!(plan.low_quality);
        assert!(plan.changes.last().unwrap().contains("will look soft"));
    }

    #[test]
    fn newer_codecs_keep_more_resolution() {
        let src = source(1920, 1080, 30.0, 120.0);
        let h264 = plan_video(&src, 9_700_000, &budget(Strategy::BestQuality)).unwrap();
        let mut av1 = budget(Strategy::BestQuality);
        av1.efficiency = 0.6;
        let av1 = plan_video(&src, 9_700_000, &av1).unwrap();
        assert!(av1.height >= h264.height);
    }

    #[test]
    fn corrections_aim_below_the_target() {
        let next = corrected_kbps(1000, 11_000_000, 9_700_000, 128, 60.0);
        assert!(next < 880 && next > 700, "{next}");
    }

    #[test]
    fn sizes_and_lengths_read_naturally() {
        assert_eq!(human_bytes(9_700_000), "9.7 MB");
        assert_eq!(human_bytes(512_000), "512 KB");
        assert_eq!(human_duration(5.25), "5.2 seconds");
        assert_eq!(human_duration(200.0), "3 minutes 20 seconds");
        assert_eq!(human_duration(3900.0), "1 hour 5 minutes");
        assert_eq!(scaled(1920, 1080, 720), (1280, 720));
        assert_eq!(scaled(1081, 721, 1080), (1082, 722));
    }
}
