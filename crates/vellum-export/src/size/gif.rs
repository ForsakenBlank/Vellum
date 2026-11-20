//! fitting a gif under a size. gifs have no bitrate to set, so the size comes from a ladder
//! of ever smaller settings (lossy compression, fewer colours, narrower, fewer frames) and a
//! binary search over it for the first step that fits. every probe is a real render, so
//! the result is exactly the file that was measured.
//!
//! the same search serves the "fit to x mb" box in video to gif, starting from whatever the
//! user picked there.

use std::path::{Path, PathBuf};

use vellum_media::control::Span;
use vellum_media::probe::MediaInfo;
use vellum_media::{Control, MediaError, MediaResult};

use super::budget::human_bytes;
use super::{SizeJob, already_fits, too_big};
use crate::anim::{self, AnimFormat, AnimOptions, AnimSource};
use crate::{Env, ToolResult, naming};

/// one rung of the ladder, relative to where the user started
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    /// share of the starting width
    pub scale: f64,
    /// frames per second, none keeps the start's
    pub fps: Option<f64>,
    pub colors: u32,
    pub lossy: u32,
    /// for webp, avif and mp4
    pub quality: u32,
}

/// the ladder from `base` down, each step a little smaller than the one before
pub fn ladder(base: &AnimOptions, source_fps: Option<f64>) -> Vec<Step> {
    let fps = base.fps.or(source_fps).unwrap_or(15.0);
    let start = Step {
        scale: 1.0,
        fps: base.fps,
        colors: base.colors.clamp(2, 256),
        lossy: base.lossy.min(100),
        quality: base.quality.min(100),
    };
    let mut steps = vec![start];
    let mut push = |change: &dyn Fn(&mut Step)| {
        let mut next = *steps.last().unwrap_or(&start);
        change(&mut next);
        if steps.last() != Some(&next) {
            steps.push(next);
        }
    };
    let slower = |limit: f64| {
        move |s: &mut Step| {
            if fps > limit {
                s.fps = Some(limit);
            }
        }
    };
    push(&|s| {
        s.lossy = s.lossy.max(30);
        s.quality = s.quality.min(65);
    });
    push(&|s| {
        s.lossy = s.lossy.max(60);
        s.quality = s.quality.min(55);
    });
    push(&|s| s.scale = 0.85);
    push(&|s| s.colors = s.colors.min(128));
    push(&|s| s.scale = 0.7);
    push(&slower(12.0));
    push(&|s| {
        s.scale = 0.6;
        s.lossy = s.lossy.max(80);
        s.quality = s.quality.min(45);
    });
    push(&slower(10.0));
    push(&|s| s.scale = 0.5);
    push(&|s| s.colors = s.colors.min(64));
    push(&|s| s.scale = 0.4);
    push(&slower(8.0));
    push(&|s| {
        s.scale = 0.33;
        s.quality = s.quality.min(35);
    });
    push(&|s| s.scale = 0.25);
    push(&slower(6.0));
    push(&|s| s.scale = 0.18);
    steps
}

/// the first index whose probe says it fits, assuming everything after a fitting step fits
/// too. probes index 0 first, since most of the time nothing needs to change.
pub fn first_fitting(
    len: usize,
    mut fits: impl FnMut(usize) -> MediaResult<bool>,
) -> MediaResult<Option<usize>> {
    if len == 0 {
        return Ok(None);
    }
    if fits(0)? {
        return Ok(Some(0));
    }
    let last = len - 1;
    if last == 0 || !fits(last)? {
        return Ok(None);
    }
    // index 0 does not fit, `last` does
    let (mut over, mut under) = (0, last);
    while under - over > 1 {
        let mid = over + (under - over) / 2;
        if fits(mid)? {
            under = mid;
        } else {
            over = mid;
        }
    }
    Ok(Some(under))
}

/// the options for one step
pub fn apply(base: &AnimOptions, step: &Step, source: &AnimSource) -> AnimOptions {
    let (width, _) = anim::frame_size(source, base);
    let scaled = (f64::from(width) * step.scale).round();
    let scaled = (scaled as u32).max(16);
    AnimOptions {
        width: Some(scaled),
        fps: step.fps,
        colors: step.colors,
        lossy: step.lossy,
        quality: step.quality,
        ..base.clone()
    }
}

/// what changed between the start and the step that fit, in plain words
fn describe(base: &AnimOptions, step: &Step, source: &AnimSource) -> Vec<String> {
    let mut changes = Vec::new();
    let (width, height) = anim::frame_size(source, base);
    if step.scale < 0.999 {
        let chosen = anim::frame_size(source, &apply(base, step, source));
        changes.push(format!(
            "Scaled down from {width}×{height} to {}×{}.",
            chosen.0, chosen.1
        ));
    }
    if step.fps != base.fps
        && let Some(fps) = step.fps
    {
        changes.push(format!("Lowered to {} frames a second.", crate::num(fps)));
    }
    if base.format == AnimFormat::Gif {
        if step.colors < base.colors.clamp(2, 256) {
            changes.push(format!("Reduced to {} colours.", step.colors));
        }
        if step.lossy > base.lossy {
            changes.push("Used lossy compression.".to_owned());
        }
    } else if step.quality < base.quality {
        changes.push(format!("Lowered the quality to {}.", step.quality));
    }
    changes
}

/// renders `input` with `base`, stepping down until it fits `goal`. returns what changed.
pub fn fit_anim(
    env: &Env,
    input: &Path,
    source: &AnimSource,
    base: &AnimOptions,
    goal: u64,
    output: &Path,
    control: &dyn Control,
) -> MediaResult<Vec<String>> {
    let steps = ladder(base, source.fps);
    let temp = tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
    let mut best: Option<(usize, PathBuf)> = None;
    let mut tries = 0;
    // a ladder of 18 steps takes at most 6 renders
    let found = first_fitting(steps.len(), |i| {
        control.checkpoint()?;
        tries += 1;
        control.message(&format!("Trying settings {tries}"));
        let path = temp
            .path()
            .join(format!("step{i}.{}", base.format.extension()));
        let span = Span {
            inner: control,
            start: (tries as f32 - 1.0).min(5.0) / 6.0,
            end: (tries as f32).min(6.0) / 6.0,
        };
        anim::render(
            env,
            input,
            source,
            &apply(base, &steps[i], source),
            &path,
            &span,
        )?;
        let bytes = std::fs::metadata(&path)
            .map_err(|err| MediaError::io(&path, err))?
            .len();
        tracing::debug!(step = i, bytes, goal, "gif size probe");
        let fits = bytes <= goal;
        if fits && best.as_ref().is_none_or(|(b, _)| i < *b) {
            best = Some((i, path));
        }
        Ok(fits)
    })?;
    let (Some(index), Some((_, path))) = (found, best) else {
        return Err(too_big(goal, "this animation"));
    };
    std::fs::copy(&path, output).map_err(|err| MediaError::io(output, err))?;
    control.progress(1.0);
    let mut changes = describe(base, &steps[index], source);
    if changes.is_empty() && base.format == AnimFormat::Gif {
        changes.push("Rebuilt the colours and only redraws what moves between frames.".to_owned());
    }
    Ok(changes)
}

/// export to size for a gif: keeps its timing and size where it can
pub fn fit(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    job: &SizeJob,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let goal = job.goal;
    if !job.options.trimmed() && info.size_bytes <= goal {
        return Ok(already_fits(input, goal));
    }
    let source = AnimSource::of(info)?;
    let base = AnimOptions {
        start: job.options.start,
        end: job.options.end,
        width: None,
        fps: None,
        loops: 0,
        ..AnimOptions::default()
    };
    let output = naming::output_for(input, env.output_folder, Some(&job.label), "gif");
    let changes = fit_anim(env, input, &source, &base, goal, output.path(), control)?;
    let mut result = ToolResult::file(input, output.path());
    result.target_bytes = Some(goal);
    result.changes = changes;
    if let Some(note) = mp4_comparison(env, input, &source, &base, result.bytes.unwrap_or(0))? {
        result.notes.push(note);
    }
    Ok(result)
}

/// a short note when the same clip as an mp4 would be far smaller, which it usually is
pub fn mp4_comparison(
    env: &Env,
    input: &Path,
    source: &AnimSource,
    base: &AnimOptions,
    gif_bytes: u64,
) -> MediaResult<Option<String>> {
    if gif_bytes == 0 {
        return Ok(None);
    }
    let temp = tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
    let path = temp.path().join("compare.mp4");
    let options = AnimOptions {
        format: AnimFormat::Mp4,
        quality: 70,
        lossy: 0,
        ..base.clone()
    };
    // a missing h.264 encoder or a strange size only means no hint
    if anim::render(
        env,
        input,
        source,
        &options,
        &path,
        &vellum_media::Unattended,
    )
    .is_err()
    {
        return Ok(None);
    }
    let mp4 = std::fs::metadata(&path).map_or(0, |m| m.len());
    // the mp4 keeps full size and frame rate, so even a modest saving is worth a mention
    if mp4 == 0 || mp4 * 3 / 2 > gif_bytes {
        return Ok(None);
    }
    Ok(Some(format!(
        "As an MP4 this would be about {} instead of {}, at full size and frame rate. Discord and most chats play MP4s just like GIFs.",
        human_bytes(mp4),
        human_bytes(gif_bytes)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_only_ever_gets_smaller() {
        let steps = ladder(&AnimOptions::default(), Some(30.0));
        assert!(steps.len() > 12);
        for pair in steps.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(b.scale <= a.scale && b.colors <= a.colors && b.lossy >= a.lossy);
            assert!(b.fps.unwrap_or(99.0) <= a.fps.unwrap_or(99.0));
            assert_ne!(a, b);
        }
        // a slow gif never gets a higher frame rate
        let slow = AnimOptions {
            fps: Some(8.0),
            ..AnimOptions::default()
        };
        assert!(
            ladder(&slow, None)
                .iter()
                .all(|s| s.fps == Some(8.0) || s.fps.unwrap() < 8.0)
        );
    }

    #[test]
    fn the_search_finds_the_first_rung_that_fits() {
        for answer in 0..18 {
            let mut probes = 0;
            let found = first_fitting(18, |i| {
                probes += 1;
                Ok(i >= answer)
            })
            .unwrap();
            assert_eq!(found, Some(answer));
            assert!(probes <= 7, "{probes} probes for {answer}");
        }
        assert_eq!(first_fitting(18, |_| Ok(false)).unwrap(), None);
    }
}
