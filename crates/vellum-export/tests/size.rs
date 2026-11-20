//! export to size on real files: every result must land under its limit, whatever the
//! file and whatever the strategy.

mod common;

use common::fixture;
use vellum_core::settings::SizeUnit;
use vellum_export::size::{self, AudioTarget, ImageTarget, SizeJob, SizeOptions, Strategy};
use vellum_media::Unattended;
use vellum_media::ffmpeg::hw::VideoCodec;
use vellum_media::probe;

fn job(options: &SizeOptions, goal: u64) -> SizeJob<'_> {
    SizeJob {
        options,
        goal,
        label: format!("{} KB", goal / 1000),
    }
}

/// runs one fit and checks the file on disk
fn fits(name: &str, options: &SizeOptions, goal: u64) -> vellum_export::ToolResult {
    let fx = fixture().unwrap();
    let out = tempfile::tempdir().unwrap();
    let (path, info) = fx.probe(name);
    let result = size::fit(
        &fx.env(out.path()),
        &path,
        &info,
        &job(options, goal),
        &Unattended,
    )
    .unwrap_or_else(|err| panic!("{name} at {goal}: {err}"));
    let bytes = std::fs::metadata(&result.path).unwrap().len();
    assert!(bytes <= goal, "{name}: {bytes} bytes is over {goal}");
    assert_eq!(result.bytes, Some(bytes));
    // the result opens as the kind of file it should be
    let back = probe::probe(std::path::Path::new(&result.path), Some(&fx.ffmpeg)).unwrap();
    assert_eq!(back.kind, info.kind, "{name}");
    // dropped into a tempdir that is about to go, so keep what the caller checks
    let kept = std::env::temp_dir().join(format!("vellum-kept-{}", std::process::id()));
    std::fs::create_dir_all(&kept).unwrap();
    let copy = kept.join(std::path::Path::new(&result.path).file_name().unwrap());
    std::fs::copy(&result.path, &copy).unwrap();
    vellum_export::ToolResult {
        path: copy.display().to_string(),
        ..result
    }
}

#[test]
fn videos_land_under_every_target() {
    if fixture().is_none() {
        return;
    }
    let options = SizeOptions::default();
    for goal in [1_200_000, 600_000, 250_000] {
        let result = fits("noisy.mp4", &options, goal);
        // two pass lands close, not just under
        assert!(
            result.bytes.unwrap() > goal * 6 / 10,
            "{goal}: only {:?}",
            result.bytes
        );
    }
}

#[test]
fn strategies_codecs_and_trims_still_fit() {
    if fixture().is_none() {
        return;
    }
    let goal = 300_000;
    for strategy in [
        Strategy::KeepResolution,
        Strategy::KeepFps,
        Strategy::KeepAudio,
    ] {
        let options = SizeOptions {
            strategy,
            ..SizeOptions::default()
        };
        fits("noisy.mp4", &options, goal);
    }
    for codec in [VideoCodec::H265, VideoCodec::Vp9] {
        let options = SizeOptions {
            codec,
            ..SizeOptions::default()
        };
        let result = fits("noisy.mp4", &options, goal);
        let ext = if codec == VideoCodec::Vp9 {
            ".webm"
        } else {
            ".mp4"
        };
        assert!(result.path.ends_with(ext), "{}", result.path);
    }
    let options = SizeOptions {
        start: Some(1.0),
        end: Some(3.5),
        mute: true,
        ..SizeOptions::default()
    };
    let result = fits("noisy.mp4", &options, 200_000);
    let info = probe::probe(
        std::path::Path::new(&result.path),
        fixture().map(|f| &f.ffmpeg),
    )
    .unwrap();
    assert!(info.audio.is_empty());
    assert!(
        (info.duration_secs.unwrap() - 2.5).abs() < 0.2,
        "{:?}",
        info.duration_secs
    );
}

#[test]
fn sixty_fps_drops_to_thirty_when_tight() {
    if fixture().is_none() {
        return;
    }
    let result = fits("fast.mp4", &SizeOptions::default(), 250_000);
    assert!(
        result
            .changes
            .iter()
            .any(|c| c.contains("frame rate from 60 to 30")),
        "{:?}",
        result.changes
    );
}

#[test]
fn files_that_already_fit_are_left_alone() {
    let Some(fx) = fixture() else { return };
    let out = tempfile::tempdir().unwrap();
    let (path, info) = fx.probe("photo.jpg");
    let options = SizeOptions::default();
    let result = size::fit(
        &fx.env(out.path()),
        &path,
        &info,
        &job(&options, 50_000_000),
        &Unattended,
    )
    .unwrap();
    assert!(result.already_fits);
    assert_eq!(result.path, path.display().to_string());
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}

#[test]
fn impossible_targets_explain_themselves() {
    let Some(fx) = fixture() else { return };
    let out = tempfile::tempdir().unwrap();
    let (path, info) = fx.probe("noisy.mp4");
    let options = SizeOptions::default();
    let err = size::fit(
        &fx.env(out.path()),
        &path,
        &info,
        &job(&options, 8_000),
        &Unattended,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("is too small for 6.0 seconds of video"),
        "{err}"
    );
    assert_eq!(
        std::fs::read_dir(out.path()).unwrap().count(),
        0,
        "nothing is left behind"
    );
}

#[test]
fn photos_fit_in_every_format() {
    if fixture().is_none() {
        return;
    }
    // a jpeg stays a jpeg, and scales down when quality alone is not enough
    let result = fits("photo.jpg", &SizeOptions::default(), 250_000);
    assert!(result.path.ends_with(".jpg"));
    let tiny = fits("photo.jpg", &SizeOptions::default(), 40_000);
    assert!(
        tiny.changes.iter().any(|c| c.starts_with("Scaled down")),
        "{:?}",
        tiny.changes
    );

    // a png gets a palette, then scaling, and a hint that jpeg would suit a photo
    let png = fits("photo.png", &SizeOptions::default(), 400_000);
    assert!(png.path.ends_with(".png"));
    assert!(
        png.changes.iter().any(|c| c.contains("256 colours")),
        "{:?}",
        png.changes
    );
    let small_png = fits("photo.png", &SizeOptions::default(), 120_000);
    assert!(!small_png.notes.is_empty());

    for format in [ImageTarget::Webp, ImageTarget::Avif] {
        let options = SizeOptions {
            image_format: format,
            ..SizeOptions::default()
        };
        fits("photo.png", &options, 60_000);
    }
}

#[test]
fn gifs_step_down_until_they_fit_and_mention_mp4() {
    if fixture().is_none() {
        return;
    }
    let fx = fixture().unwrap();
    let original = std::fs::metadata(fx.file("clip.gif")).unwrap().len();
    let result = fits("clip.gif", &SizeOptions::default(), original / 2);
    assert!(!result.changes.is_empty());
    assert!(
        result.notes.iter().any(|n| n.starts_with("As an MP4")),
        "{:?}",
        result.notes
    );
    // pure noise is as big as an mp4 as it is as a gif, so no hint there
    let noise = fits("noisy.gif", &SizeOptions::default(), 600_000);
    assert!(noise.notes.is_empty(), "{:?}", noise.notes);
    let small = fits("noisy.gif", &SizeOptions::default(), 150_000);
    assert!(
        small.changes.iter().any(|c| c.starts_with("Scaled down")),
        "{:?}",
        small.changes
    );
}

#[test]
fn audio_lands_under_as_mp3_and_opus() {
    if fixture().is_none() {
        return;
    }
    for goal in [500_000, 120_000] {
        let result = fits("speech.wav", &SizeOptions::default(), goal);
        assert!(result.path.ends_with(".mp3"));
    }
    let options = SizeOptions {
        audio_format: AudioTarget::Opus,
        ..SizeOptions::default()
    };
    let result = fits("speech.wav", &options, 80_000);
    assert!(result.path.ends_with(".ogg"));
}

#[test]
fn units_and_margins_are_honoured() {
    let settings = vellum_core::settings::ExportSettings::default();
    let options = SizeOptions {
        target: Some(size::TargetSize {
            amount: 1.0,
            unit: SizeUnit::Mib,
        }),
        margin_percent: Some(10.0),
        ..SizeOptions::default()
    };
    assert_eq!(options.goal_bytes(&settings), 943_718);
}
