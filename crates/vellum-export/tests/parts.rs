//! joining and cutting real files.

// test helpers unwrap too, a failure in one is a failed test
#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};

use common::fixture;
use vellum_export::convert::{ConvertOptions, ConvertTarget};
use vellum_export::parts;
use vellum_media::Unattended;
use vellum_media::probe::{self, MediaInfo};

fn options(target: ConvertTarget) -> ConvertOptions {
    ConvertOptions {
        target: Some(target),
        ..ConvertOptions::default()
    }
}

/// copies of test files under new names, so one file can appear twice in a join
fn copies(names: &[&str], dir: &Path) -> Vec<(PathBuf, MediaInfo)> {
    let fx = fixture().unwrap();
    names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let path = dir.join(format!("{i} {name}"));
            std::fs::copy(fx.dir.join("media").join(name), &path).unwrap();
            let info = probe::probe(&path, Some(&fx.ffmpeg)).unwrap();
            (path, info)
        })
        .collect()
}

fn duration(path: &Path) -> f64 {
    let fx = fixture().unwrap();
    probe::probe(path, Some(&fx.ffmpeg))
        .unwrap()
        .duration_secs
        .unwrap()
}

#[test]
fn matching_clips_join_without_re_encoding() {
    let Some(fx) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let inputs = copies(&["h264.mp4", "h264.mp4"], dir.path());
    let target = ConvertTarget::Mp4;
    let result = parts::merge(
        &fx.env(dir.path()),
        &inputs,
        target,
        &options(target),
        &Unattended,
    )
    .unwrap();
    assert!(
        result.path.ends_with("0 h264 (joined).mp4"),
        "{}",
        result.path
    );
    assert!(
        result
            .changes
            .iter()
            .any(|c| c.contains("without re-encoding"))
    );
    let joined = probe::probe(Path::new(&result.path), Some(&fx.ffmpeg)).unwrap();
    assert!((joined.duration_secs.unwrap() - 4.0).abs() < 0.2);
    assert_eq!(joined.video.unwrap().codec, "h264");
    assert_eq!(joined.audio[0].codec, "aac");
}

#[test]
fn different_clips_are_re_encoded_to_the_first_shape() {
    let Some(fx) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    // h265.mp4 has no sound and another frame rate, so it gets silence and new timing
    let inputs = copies(&["h264.mp4", "h265.mp4", "vp9.webm"], dir.path());
    let target = ConvertTarget::Mp4;
    let result = parts::merge(
        &fx.env(dir.path()),
        &inputs,
        target,
        &options(target),
        &Unattended,
    )
    .unwrap();
    assert!(result.changes.iter().any(|c| c.contains("Re-encoded")));
    let joined = probe::probe(Path::new(&result.path), Some(&fx.ffmpeg)).unwrap();
    assert!(
        (joined.duration_secs.unwrap() - 6.0).abs() < 0.3,
        "{:?}",
        joined.duration_secs
    );
    assert_eq!((joined.width, joined.height), (Some(320), Some(240)));
    assert_eq!(joined.video.unwrap().codec, "h264");
    assert_eq!(joined.audio.len(), 1);
}

#[test]
fn sounds_join_into_one_track() {
    let Some(fx) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let inputs = copies(&["tone.mp3", "tone.wav", "h264.mp4"], dir.path());
    let target = ConvertTarget::Mp3;
    let result = parts::merge(
        &fx.env(dir.path()),
        &inputs,
        target,
        &options(target),
        &Unattended,
    )
    .unwrap();
    let joined = probe::probe(Path::new(&result.path), Some(&fx.ffmpeg)).unwrap();
    assert!(joined.video.is_none());
    assert_eq!(joined.audio[0].codec, "mp3");
    assert!((joined.duration_secs.unwrap() - 6.0).abs() < 0.3);
}

#[test]
fn joining_explains_what_cannot_be_joined() {
    let Some(fx) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let env = fx.env(dir.path());
    let one = copies(&["h264.mp4"], dir.path());
    let err = parts::merge(
        &env,
        &one,
        ConvertTarget::Mp4,
        &ConvertOptions::default(),
        &Unattended,
    )
    .unwrap_err();
    assert!(err.to_string().contains("at least two"));

    let mixed = copies(&["h264.mp4", "tone.wav"], dir.path());
    let err = parts::merge(
        &env,
        &mixed,
        ConvertTarget::Mp4,
        &ConvertOptions::default(),
        &Unattended,
    )
    .unwrap_err();
    assert!(err.to_string().contains("is not a video"), "{err}");
}

#[test]
fn exact_cuts_make_parts_of_one_length() {
    let Some(fx) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (path, info) = copies(&["h264.mp4"], dir.path()).remove(0);
    let opts = ConvertOptions {
        split_seconds: 0.5,
        split_exact: true,
        ..options(ConvertTarget::Mp4)
    };
    let result = parts::split(
        &fx.env(dir.path()),
        &path,
        &info,
        ConvertTarget::Mp4,
        &opts,
        &Unattended,
    )
    .unwrap();
    assert!(result.folder);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&result.path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 4, "{files:?}");
    assert!(files[0].ends_with("0 h264 part 001.mp4"));
    for file in &files {
        assert!((duration(file) - 0.5).abs() < 0.1, "{file:?}");
    }
}

#[test]
fn quick_cuts_copy_and_sound_splits_too() {
    let Some(fx) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let env = fx.env(dir.path());
    // the test clip has one keyframe, so a copied cut can only make one part
    let (path, info) = copies(&["h264.mp4"], dir.path()).remove(0);
    let opts = ConvertOptions {
        split_seconds: 0.5,
        ..options(ConvertTarget::Mkv)
    };
    let result = parts::split(&env, &path, &info, ConvertTarget::Mkv, &opts, &Unattended).unwrap();
    assert!(
        result
            .changes
            .iter()
            .any(|c| c.contains("nearest keyframe"))
    );
    assert!(std::fs::read_dir(&result.path).unwrap().count() >= 1);

    let (path, info) = copies(&["tone.wav"], dir.path()).remove(0);
    let opts = ConvertOptions {
        split_seconds: 0.5,
        ..options(ConvertTarget::Wav)
    };
    let result = parts::split(&env, &path, &info, ConvertTarget::Wav, &opts, &Unattended).unwrap();
    assert_eq!(std::fs::read_dir(&result.path).unwrap().count(), 4);

    let too_long = ConvertOptions {
        split_seconds: 10.0,
        ..opts
    };
    let err = parts::split(
        &env,
        &path,
        &info,
        ConvertTarget::Wav,
        &too_long,
        &Unattended,
    )
    .unwrap_err();
    assert!(err.to_string().contains("shorter than one part"));
}
