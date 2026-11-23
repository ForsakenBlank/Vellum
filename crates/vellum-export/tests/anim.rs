//! video to gif and the other loop formats, rendered for real.

// test helpers unwrap too, a failure in one is a failed test
#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;

use common::fixture;
use vellum_export::anim::{self, AnimFormat, AnimOptions, AnimSource, Caption, CaptionStyle, Crop};
use vellum_export::size::gif::fit_anim;
use vellum_media::picture::{self, DecodeOptions};
use vellum_media::probe::{self, MediaKind};
use vellum_media::{Control, MediaError, Unattended};

fn render(name: &str, options: &AnimOptions) -> (tempfile::TempDir, std::path::PathBuf) {
    let fx = fixture().unwrap();
    let out = tempfile::tempdir().unwrap();
    let (path, info) = fx.probe(name);
    let source = AnimSource::of(&info).unwrap();
    let output = out
        .path()
        .join(format!("out.{}", options.format.extension()));
    anim::render(
        &fx.env(out.path()),
        &path,
        &source,
        options,
        &output,
        &Unattended,
    )
    .unwrap_or_else(|err| panic!("{name} as {:?}: {err}", options.format));
    (out, output)
}

fn frames(path: &Path) -> Vec<picture::Frame> {
    picture::decode(
        path,
        DecodeOptions {
            size: None,
            all_frames: true,
        },
        fixture().map(|f| &f.ffmpeg),
    )
    .unwrap()
    .frames
}

#[test]
fn a_clip_becomes_a_small_looping_gif() {
    if fixture().is_none() {
        return;
    }
    let options = AnimOptions {
        start: Some(1.0),
        end: Some(3.0),
        width: Some(240),
        fps: Some(10.0),
        ..AnimOptions::default()
    };
    let (_dir, path) = render("noisy.mp4", &options);
    let frames = frames(&path);
    // 2 seconds at 10 fps
    assert!((19..=21).contains(&frames.len()), "{} frames", frames.len());
    assert_eq!(frames[0].image.dimensions(), (240, 135));
    assert!(frames.iter().all(|f| (90..=110).contains(&f.delay_ms)));
}

#[test]
fn every_format_comes_out_as_what_it_says() {
    let Some(fx) = fixture() else { return };
    for format in [
        AnimFormat::Webp,
        AnimFormat::Apng,
        AnimFormat::Avif,
        AnimFormat::Mp4,
    ] {
        if format == AnimFormat::Webp && !fx.has_encoder("libwebp_anim") {
            continue;
        }
        let options = AnimOptions {
            end: Some(1.0),
            width: Some(160),
            format,
            ..AnimOptions::default()
        };
        let (_dir, path) = render("noisy.mp4", &options);
        let info = probe::probe(&path, Some(&fx.ffmpeg)).unwrap();
        assert!(info.size_bytes > 0, "{format:?}");
        let expected = match format {
            AnimFormat::Mp4 => MediaKind::Video,
            _ => MediaKind::Image,
        };
        assert_eq!(info.kind, expected, "{format:?}: {}", info.format);
        if format == AnimFormat::Mp4 {
            assert_eq!(info.width, Some(160));
            assert!(info.audio.is_empty());
        }
    }
}

#[test]
fn speed_boomerang_and_reverse_change_the_timing() {
    if fixture().is_none() {
        return;
    }
    let base = AnimOptions {
        end: Some(2.0),
        width: Some(120),
        fps: Some(10.0),
        ..AnimOptions::default()
    };
    let fast = AnimOptions {
        speed: 2.0,
        ..base.clone()
    };
    let (_a, path) = render("noisy.mp4", &fast);
    assert!((9..=11).contains(&frames(&path).len()));

    let boomerang = AnimOptions {
        boomerang: true,
        ..base.clone()
    };
    let (_b, path) = render("noisy.mp4", &boomerang);
    let there_and_back = frames(&path);
    // 20 frames out and 19 back, the turning frame shows once
    assert!(
        (38..=40).contains(&there_and_back.len()),
        "{}",
        there_and_back.len()
    );

    // the reversed clip starts where the forward one ends
    let (_c, forward) = render(
        "frames.mp4",
        &AnimOptions {
            width: None,
            ..base.clone()
        },
    );
    let reverse = AnimOptions {
        reverse: true,
        width: None,
        ..base.clone()
    };
    let (_d, backward) = render("frames.mp4", &reverse);
    let f = frames(&forward);
    let b = frames(&backward);
    let grey = |frame: &picture::Frame| frame.image.get_pixel(4, 4)[0];
    assert!(grey(&f[0]) < grey(f.last().unwrap()));
    assert!(grey(&b[0]) > grey(b.last().unwrap()));
}

#[test]
fn crops_and_captions_land_in_the_picture() {
    if fixture().is_none() {
        return;
    }
    let options = AnimOptions {
        end: Some(0.5),
        width: None,
        crop: Some(Crop {
            x: 100,
            y: 50,
            width: 200,
            height: 100,
        }),
        caption: Caption {
            top: "Top".into(),
            bottom: String::new(),
            style: CaptionStyle::Bar,
            scale: 1.0,
        },
        format: AnimFormat::Apng,
        ..AnimOptions::default()
    };
    let (_dir, path) = render("noisy.mp4", &options);
    let first = &frames(&path)[0].image;
    assert_eq!(first.width(), 200);
    assert!(first.height() > 100, "the bar adds height");
    // the bar is white at its edge, give or take the colour conversion of the ffmpeg build
    let edge = first.get_pixel(1, 1).0;
    assert!(edge.iter().all(|&v| v >= 250), "{edge:?}");
}

#[test]
fn lossy_gifs_are_smaller() {
    if fixture().is_none() {
        return;
    }
    let base = AnimOptions {
        end: Some(2.0),
        width: Some(320),
        ..AnimOptions::default()
    };
    let (_a, plain) = render("h264.mp4", &base);
    let (_b, lossy) = render("h264.mp4", &AnimOptions { lossy: 80, ..base });
    let plain = std::fs::metadata(plain).unwrap().len();
    let lossy = std::fs::metadata(lossy).unwrap().len();
    assert!(lossy < plain * 9 / 10, "lossy {lossy} vs plain {plain}");
}

#[test]
fn fit_to_size_steps_down_from_the_users_choice() {
    let Some(fx) = fixture() else { return };
    let out = tempfile::tempdir().unwrap();
    let (path, info) = fx.probe("noisy.mp4");
    let source = AnimSource::of(&info).unwrap();
    let base = AnimOptions {
        end: Some(3.0),
        width: Some(480),
        ..AnimOptions::default()
    };
    let output = out.path().join("fit.gif");
    let goal = 400_000;
    let changes = fit_anim(
        &fx.env(out.path()),
        &path,
        &source,
        &base,
        goal,
        &output,
        &Unattended,
    )
    .unwrap();
    assert!(std::fs::metadata(&output).unwrap().len() <= goal);
    assert!(!changes.is_empty());
}

#[derive(Default)]
struct CancelAfter(std::cell::Cell<u32>);

impl Control for CancelAfter {
    fn checkpoint(&self) -> Result<(), MediaError> {
        self.0.set(self.0.get() + 1);
        if self.0.get() > 3 {
            Err(MediaError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn progress(&self, _fraction: f32) {}
}

#[test]
fn cancelling_leaves_no_file() {
    let Some(fx) = fixture() else { return };
    let out = tempfile::tempdir().unwrap();
    let (path, info) = fx.probe("noisy.mp4");
    let source = AnimSource::of(&info).unwrap();
    let output = out.path().join("never.gif");
    let err = anim::render(
        &fx.env(out.path()),
        &path,
        &source,
        &AnimOptions::default(),
        &output,
        &CancelAfter::default(),
    )
    .unwrap_err();
    assert!(matches!(err, MediaError::Cancelled));
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}
