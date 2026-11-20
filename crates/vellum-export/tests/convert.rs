//! the converter on real files.

mod common;

use std::path::Path;

use common::fixture;
use vellum_export::ToolResult;
use vellum_export::convert::{self, ConvertOptions, ConvertTarget};
use vellum_media::Unattended;
use vellum_media::probe::{self, MediaInfo, MediaKind};

fn run(name: &str, options: &ConvertOptions) -> (tempfile::TempDir, ToolResult, MediaInfo) {
    let fx = fixture().unwrap();
    let out = tempfile::tempdir().unwrap();
    let (path, info) = fx.probe(name);
    let result = convert::convert(&fx.env(out.path()), &path, &info, options, &Unattended)
        .unwrap_or_else(|err| panic!("{name} to {:?}: {err}", options.target));
    let back = if result.folder {
        info.clone()
    } else {
        probe::probe(Path::new(&result.path), Some(&fx.ffmpeg)).unwrap()
    };
    (out, result, back)
}

fn to(target: ConvertTarget) -> ConvertOptions {
    ConvertOptions {
        target: Some(target),
        ..ConvertOptions::default()
    }
}

#[test]
fn changing_only_the_container_copies_the_streams() {
    if fixture().is_none() {
        return;
    }
    let (_a, result, info) = run("h264.mp4", &to(ConvertTarget::Mkv));
    assert!(result.path.ends_with(".mkv"));
    assert!(result.changes[0].starts_with("Copied without re-encoding"));
    assert_eq!(info.video.unwrap().codec, "h264");

    // vp9 fits in mp4 as it is
    let (_b, result, info) = run("vp9.webm", &to(ConvertTarget::Mp4));
    assert!(result.changes[0].starts_with("Copied"));
    assert_eq!(info.video.unwrap().codec, "vp9");

    // aac sound comes out of a video untouched
    let (_c, result, info) = run("h264.mp4", &to(ConvertTarget::M4a));
    assert!(result.changes[0].starts_with("Copied"));
    assert_eq!(info.kind, MediaKind::Audio);
}

#[test]
fn other_codecs_are_encoded() {
    if fixture().is_none() {
        return;
    }
    let (_a, result, info) = run("h264.mp4", &to(ConvertTarget::Webm));
    assert!(result.changes.is_empty());
    assert_eq!(info.video.unwrap().codec, "vp9");
    assert_eq!(info.audio[0].codec, "opus");

    let (_b, _, info) = run("h264.mp4", &to(ConvertTarget::Mp3));
    assert_eq!(info.audio[0].codec, "mp3");
    assert!(info.video.is_none());

    let (_c, _, info) = run("tone.mp3", &to(ConvertTarget::Flac));
    assert_eq!(info.audio[0].codec, "flac");
}

#[test]
fn trims_speed_and_turns_apply_to_video() {
    if fixture().is_none() {
        return;
    }
    let options = ConvertOptions {
        target: Some(ConvertTarget::Mp4),
        start: Some(0.5),
        end: Some(1.5),
        speed: 2.0,
        rotate: 90,
        mute: true,
        ..ConvertOptions::default()
    };
    let (_a, _, info) = run("h264.mp4", &options);
    // one second at double speed
    assert!(
        (info.duration_secs.unwrap() - 0.5).abs() < 0.15,
        "{:?}",
        info.duration_secs
    );
    assert_eq!((info.width, info.height), (Some(240), Some(320)));
    assert!(info.audio.is_empty());
}

#[test]
fn frames_come_out_as_numbered_pictures() {
    if fixture().is_none() {
        return;
    }
    let options = ConvertOptions {
        target: Some(ConvertTarget::Frames),
        frame_interval: 0.5,
        frames_as_jpg: true,
        ..ConvertOptions::default()
    };
    let (_a, result, _) = run("h264.mp4", &options);
    assert!(result.folder);
    let count = std::fs::read_dir(&result.path).unwrap().count();
    assert!((4..=5).contains(&count), "{count} frames");
    assert!(Path::new(&result.path).join("frame 00001.jpg").is_file());

    let (_b, gif, _) = run("anim.gif", &to(ConvertTarget::Frames));
    assert!(std::fs::read_dir(&gif.path).unwrap().count() > 1);
}

#[test]
fn pictures_convert_to_every_still_format() {
    if fixture().is_none() {
        return;
    }
    for target in [
        ConvertTarget::Jpg,
        ConvertTarget::Webp,
        ConvertTarget::Avif,
        ConvertTarget::Bmp,
        ConvertTarget::Tiff,
        ConvertTarget::Ico,
        ConvertTarget::Gif,
    ] {
        let (_dir, result, info) = run("alpha.png", &to(target));
        assert!(result.path.ends_with(target.extension()), "{}", result.path);
        assert!(info.size_bytes > 0);
    }
    // camera raw, photoshop and the sideways phone photo all come out right
    let (_a, _, raw) = run("photo.dng", &to(ConvertTarget::Jpg));
    assert_eq!(raw.kind, MediaKind::Image);
    let (_b, _, psd) = run("layers.psd", &to(ConvertTarget::Png));
    assert_eq!((psd.width, psd.height), (Some(48), Some(32)));
    let (_c, _, upright) = run("exif.jpg", &to(ConvertTarget::Png));
    let (_, original) = fixture().unwrap().probe("exif.jpg");
    assert_eq!(
        (upright.width, upright.height),
        (original.width, original.height)
    );
}

#[test]
fn drawings_render_at_any_size() {
    if fixture().is_none() {
        return;
    }
    let options = ConvertOptions {
        target: Some(ConvertTarget::Png),
        max_width: Some(2000),
        ..ConvertOptions::default()
    };
    let (_dir, _, info) = run("drawing.svg", &options);
    assert_eq!(info.width, Some(2000));
}

#[test]
fn gifs_become_videos_and_videos_become_gifs() {
    if fixture().is_none() {
        return;
    }
    let (_a, _, info) = run("anim.gif", &to(ConvertTarget::Mp4));
    assert_eq!(info.kind, MediaKind::Video);
    assert_eq!(info.video.unwrap().codec, "h264");

    let (_b, _, info) = run("h264.mp4", &to(ConvertTarget::Gif));
    assert_eq!(info.kind, MediaKind::Animation);
    let (_c, result, _) = run("h264.mp4", &to(ConvertTarget::Webp));
    assert!(result.path.ends_with(".webp"));
}

#[test]
fn pictures_make_a_slideshow() {
    let Some(fx) = fixture() else { return };
    let out = tempfile::tempdir().unwrap();
    let inputs = vec![
        fx.file("photo.jpg"),
        fx.file("alpha.png"),
        fx.file("exif.jpg"),
    ];
    let options = ConvertOptions {
        slide_seconds: 1.5,
        ..ConvertOptions::default()
    };
    let result = convert::slideshow(
        &fx.env(out.path()),
        &inputs,
        ConvertTarget::Mp4,
        &options,
        &Unattended,
    )
    .unwrap();
    assert!(
        result.path.ends_with("photo (slideshow).mp4"),
        "{}",
        result.path
    );
    let info = probe::probe(Path::new(&result.path), Some(&fx.ffmpeg)).unwrap();
    assert!(
        (info.duration_secs.unwrap() - 4.5).abs() < 0.2,
        "{:?}",
        info.duration_secs
    );
}

#[test]
fn impossible_conversions_say_so() {
    let Some(fx) = fixture() else { return };
    let out = tempfile::tempdir().unwrap();
    let (path, info) = fx.probe("tone.mp3");
    let err = convert::convert(
        &fx.env(out.path()),
        &path,
        &info,
        &to(ConvertTarget::Jpg),
        &Unattended,
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        err,
        "tone.mp3 cannot be turned into JPG. Pick another format for it."
    );
}
