//! probing, seeking, previews, waveforms, encoding and metadata stripping on real files.

mod common;

use std::cell::Cell;

use common::{fixture, mean};
use vellum_media::control::{Control, Unattended};
use vellum_media::ffmpeg::encode::{
    self, AudioCodec, AudioEncode, EncodeRequest, StreamPlan, VideoEncode, VideoQuality,
};
use vellum_media::ffmpeg::hw::{self, VideoCodec};
use vellum_media::probe::{self, HdrFormat, MediaKind};
use vellum_media::seek::{self, FrameIndex};
use vellum_media::waveform::Waveform;
use vellum_media::{MediaError, metadata, thumbs};

#[test]
fn videos_are_probed_with_codecs_rates_and_streams() {
    let Some(fx) = fixture() else { return };
    let cases = [
        ("h264.mp4", "h264", 30.0, Some("aac")),
        ("h265.mp4", "hevc", 25.0, None),
        ("vp9.webm", "vp9", 24.0, Some("opus")),
    ];
    for (name, codec, fps, audio) in cases {
        let Some(path) = fx.file(name) else { continue };
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        assert_eq!(info.kind, MediaKind::Video, "{name}");
        assert_eq!((info.width, info.height), (Some(320), Some(240)), "{name}");
        let duration = info.duration_secs.unwrap();
        assert!((duration - 2.0).abs() < 0.1, "{name} lasts {duration}");
        let video = info.video.as_ref().unwrap();
        assert_eq!(video.codec, codec);
        assert!((video.frame_rate.unwrap() - fps).abs() < 0.01, "{name}");
        assert!(!video.variable_frame_rate, "{name}");
        assert_eq!(video.rotation, 0);
        assert_eq!(
            info.audio.first().map(|a| a.codec.as_str()),
            audio,
            "{name}"
        );
    }
    if let Some(path) = fx.file("h264.mp4") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        assert_eq!(info.audio[0].channels, 2);
        assert_eq!(info.audio[0].sample_rate, Some(48_000));
        assert!(
            info.tags
                .iter()
                .any(|t| t.key == "title" && t.value == "Test clip")
        );
        assert_eq!(info.format, "MP4 / QuickTime");
    }
}

#[test]
fn variable_rate_rotation_and_hdr_are_noticed() {
    let Some(fx) = fixture() else { return };
    if let Some(path) = fx.file("vfr.mkv") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        assert!(
            info.video.unwrap().variable_frame_rate,
            "vfr.mkv looks constant"
        );
    }
    if let Some(path) = fx.file("rotated.mp4") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        let video = info.video.as_ref().unwrap();
        assert_eq!(video.rotation, 90);
        assert_eq!((video.coded_width, video.coded_height), (320, 240));
        assert_eq!((info.width, info.height), (Some(240), Some(320)));
    }
    if let Some(path) = fx.file("hdr10.mp4") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        let video = info.video.unwrap();
        assert_eq!(video.bit_depth, Some(10));
        assert_eq!(video.color.primaries.as_deref(), Some("bt2020"));
        let hdr = video.hdr.expect("hdr10.mp4 is hdr");
        assert_eq!(hdr.format, HdrFormat::Hdr10);
        assert_eq!(hdr.max_cll, Some(1000));
        assert_eq!(hdr.max_fall, Some(400));
        assert!(hdr.mastering_display.unwrap().starts_with("max 1000 nits"));
    }
}

#[test]
fn audio_images_and_animations_are_probed_too() {
    let Some(fx) = fixture() else { return };
    for (name, codec) in [
        ("tone.wav", "pcm_s16le"),
        ("tone.mp3", "mp3"),
        ("tone.flac", "flac"),
        ("tone.opus", "opus"),
    ] {
        let Some(path) = fx.file(name) else { continue };
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        assert_eq!(info.kind, MediaKind::Audio, "{name}");
        assert_eq!(info.audio[0].codec, codec, "{name}");
        assert!((info.duration_secs.unwrap() - 2.0).abs() < 0.1, "{name}");
    }
    if let Some(path) = fx.file("tone.mp3") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        assert!(
            info.tags
                .iter()
                .any(|t| t.key == "artist" && t.value == "Vellum")
        );
    }
    if let Some(path) = fx.file("exif.jpg") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        assert_eq!(info.kind, MediaKind::Image);
        assert_eq!(info.format, "JPEG");
        assert_eq!((info.width, info.height), (Some(480), Some(640)));
        assert!(info.has_location);
        assert!(
            info.tags
                .iter()
                .any(|t| t.group == "GPS" && t.key == "GPSLatitude")
        );
        assert_eq!(
            info.image.unwrap().color.as_deref(),
            Some("RGB, 8 bits per channel")
        );
    }
    if let Some(path) = fx.file("anim.gif") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        assert_eq!(info.kind, MediaKind::Animation);
        assert_eq!(info.image.unwrap().frames, Some(10));
        assert!((info.duration_secs.unwrap() - 1.0).abs() < 0.1);
    }
    if let Some(path) = fx.file("drawing.svg") {
        let info = probe::probe(path, None).unwrap();
        assert_eq!(info.kind, MediaKind::Vector);
    }
    if let Some(path) = fx.file("h264.mp4") {
        assert!(matches!(
            probe::probe(path, None),
            Err(MediaError::FfmpegMissing)
        ));
    }
}

#[test]
fn seeking_lands_on_the_exact_frame() {
    let Some(fx) = fixture() else { return };
    let Some(path) = fx.file("frames.mp4") else {
        return;
    };
    let index = FrameIndex::build(&fx.ffmpeg, path, &Unattended).unwrap();
    assert_eq!(index.len(), 30);
    assert_eq!(index.frame_at(0.5), 15);
    // every frame is grey at 8 times its number, so the value says which frame came back
    for frame in [0, 1, 2, 7, 15, 16, 28, 29] {
        let got = seek::read_frame(&fx.ffmpeg, path, &index, frame, (64, 48)).unwrap();
        assert_eq!(got.index, frame);
        let grey = mean(&got.image)[0];
        let want = (frame * 8) as f64;
        assert!(
            (grey - want).abs() < 2.5,
            "frame {frame} came back as grey {grey}, not {want}"
        );
    }
    assert!(seek::read_frame(&fx.ffmpeg, path, &index, 30, (64, 48)).is_err());
}

#[test]
fn variable_rate_frames_are_found_by_their_real_time() {
    let Some(fx) = fixture() else { return };
    let Some(path) = fx.file("vfr.mkv") else {
        return;
    };
    let index = FrameIndex::build(&fx.ffmpeg, path, &Unattended).unwrap();
    assert_eq!(index.len(), 90);
    // frames 0 to 29 are 1/30 s apart, after that 1/60 s
    let t45 = index.time_of(45).unwrap();
    assert!((t45 - 1.25).abs() < 0.002, "frame 45 is at {t45}");
    let frame = seek::read_frame(&fx.ffmpeg, path, &index, 45, (160, 120)).unwrap();
    assert!((frame.time - 1.25).abs() < 0.002);
}

#[test]
fn thumbnails_and_filmstrips_are_made_for_every_kind() {
    let Some(fx) = fixture() else { return };
    for name in [
        "h264.mp4",
        "rotated.mp4",
        "photo.jpg",
        "anim.gif",
        "drawing.svg",
        "image.avif",
    ] {
        let Some(path) = fx.file(name) else { continue };
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        let thumb = thumbs::thumbnail(Some(&fx.ffmpeg), path, &info, 128)
            .unwrap_or_else(|err| panic!("{name}: {err}"))
            .unwrap_or_else(|| panic!("{name} has no thumbnail"));
        assert_eq!(thumb.width().max(thumb.height()), 128, "{name}");
        if name == "rotated.mp4" {
            assert!(
                thumb.height() > thumb.width(),
                "the rotated video thumbnail is sideways"
            );
        }
    }
    if let Some(path) = fx.file("tone.wav") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        assert!(
            thumbs::thumbnail(Some(&fx.ffmpeg), path, &info, 128)
                .unwrap()
                .is_none()
        );
    }
    if let Some(path) = fx.file("h264.mp4") {
        let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
        let strip = thumbs::filmstrip(&fx.ffmpeg, path, &info, 4, 60, &Unattended).unwrap();
        assert_eq!((strip.frame_width, strip.frame_height), (80, 60));
        assert_eq!(strip.image.dimensions(), (320, 60));
        assert_eq!(strip.times.len(), 4);
        assert!(strip.times.windows(2).all(|t| t[0] < t[1]));
    }
}

#[test]
fn waveforms_follow_the_audio() {
    let Some(fx) = fixture() else { return };
    let Some(path) = fx.file("tone.wav") else {
        return;
    };
    let info = probe::probe(path, Some(&fx.ffmpeg)).unwrap();
    let wave = Waveform::generate(&fx.ffmpeg, path, &info, &Unattended).unwrap();
    assert_eq!(wave.channels, 2);
    assert_eq!(wave.sample_rate, 48_000);
    assert!((wave.duration_secs() - 2.0).abs() < 0.02);
    // ffmpeg's sine source peaks at an eighth of full scale, and turning it from mono to
    // stereo spreads it over both channels at 1 / sqrt(2)
    let peak = 0.125 * std::f32::consts::FRAC_1_SQRT_2;
    let overview = wave.overview(50);
    let top = overview.peaks.iter().copied().fold(f32::MIN, f32::max);
    let bottom = overview.peaks.iter().copied().fold(f32::MAX, f32::min);
    assert!((top - peak).abs() < 0.005, "top {top}");
    assert!((bottom + peak).abs() < 0.005, "bottom {bottom}");
}

#[test]
fn stripped_copies_lose_their_metadata_but_not_their_picture() {
    let Some(fx) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "exif.jpg",
        "h264.mp4",
        "rotated.mp4",
        "tone.mp3",
        "image.png",
        "image.webp",
    ] {
        let Some(source) = fx.file(name) else {
            continue;
        };
        let copy = dir.path().join(name);
        std::fs::copy(source, &copy).unwrap();
        let out = metadata::strip_metadata(Some(&fx.ffmpeg), &copy, &Unattended)
            .unwrap_or_else(|err| panic!("{name}: {err}"));
        assert!(
            out.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("(no metadata)")
        );
        let before = probe::probe(&copy, Some(&fx.ffmpeg)).unwrap();
        let after = probe::probe(&out, Some(&fx.ffmpeg)).unwrap();
        assert_eq!(
            (after.width, after.height),
            (before.width, before.height),
            "{name}"
        );
        assert_eq!(after.kind, before.kind, "{name}");
        assert!(!after.has_location, "{name} still has a location");
        let personal = ["title", "artist", "GPSLatitude", "encoder", "comment"];
        let left: Vec<_> = after
            .tags
            .iter()
            .filter(|t| personal.contains(&t.key.as_str()))
            .collect();
        assert!(left.is_empty(), "{name} kept {left:?}");
        // the original is untouched
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            std::fs::read(source).unwrap()
        );
    }
    if let Some(source) = fx.file("rotated.mp4") {
        let copy = dir.path().join("rotated (no metadata).mp4");
        let info = probe::probe(&copy, Some(&fx.ffmpeg)).unwrap();
        assert_eq!(info.video.unwrap().rotation, 90, "{}", source.display());
    }
    if let Some(gif) = fx.file("anim.gif") {
        let err = metadata::strip_metadata(Some(&fx.ffmpeg), gif, &Unattended).unwrap_err();
        assert!(
            err.to_string().contains("cannot strip metadata from GIF"),
            "{err}"
        );
    }
}

#[test]
fn hardware_detection_always_offers_software() {
    let Some(fx) = fixture() else { return };
    let support = hw::detect(&fx.ffmpeg).unwrap();
    let software = support
        .software(VideoCodec::H264)
        .expect("a software h264 encoder");
    assert!(software.hardware.is_none());
    assert!(support.audio_encoders.iter().any(|a| a == "aac"));
}

#[test]
fn encoding_scales_reports_progress_and_lands_under_the_final_name() {
    let Some(fx) = fixture() else { return };
    let Some(input) = fx.file("h264.mp4") else {
        return;
    };
    let support = hw::detect(&fx.ffmpeg).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("small.mp4");
    let request = EncodeRequest {
        input: input.to_path_buf(),
        output: output.clone(),
        start: Some(0.5),
        end: Some(1.5),
        video: StreamPlan::Encode {
            settings: VideoEncode {
                codec: VideoCodec::H264,
                quality: VideoQuality::Quality { value: 30 },
                width: Some(160),
                height: None,
                fps: None,
            },
        },
        audio: StreamPlan::Encode {
            settings: AudioEncode {
                codec: AudioCodec::Aac,
                kbps: Some(64),
                channels: Some(1),
                sample_rate: None,
            },
        },
        allow_hardware: false,
        strip_metadata: true,
        duration: Some(2.0),
    };
    let progress = Progress::default();
    let outcome = encode::encode(&fx.ffmpeg, &support, &request, &progress).unwrap();
    assert_eq!(
        outcome.encoder.as_deref(),
        Some(support.software(VideoCodec::H264).unwrap().name.as_str())
    );
    assert!(outcome.fell_back.is_none());
    assert!(
        progress.last.get() > 0.5,
        "progress only reached {}",
        progress.last.get()
    );
    let info = probe::probe(&output, Some(&fx.ffmpeg)).unwrap();
    assert_eq!((info.width, info.height), (Some(160), Some(120)));
    assert!((info.duration_secs.unwrap() - 1.0).abs() < 0.15);
    assert_eq!(info.audio[0].channels, 1);
    assert!(!dir.path().join("small.vellum-part.mp4").exists());
}

#[test]
fn a_cancelled_encode_leaves_nothing_behind() {
    let Some(fx) = fixture() else { return };
    let Some(input) = fx.file("h264.mp4") else {
        return;
    };
    let support = hw::detect(&fx.ffmpeg).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("never.mp4");
    let request = EncodeRequest {
        input: input.to_path_buf(),
        output: output.clone(),
        start: None,
        end: None,
        video: StreamPlan::Encode {
            settings: VideoEncode {
                codec: VideoCodec::H264,
                quality: VideoQuality::Quality { value: 20 },
                width: None,
                height: None,
                fps: None,
            },
        },
        audio: StreamPlan::Drop,
        allow_hardware: false,
        strip_metadata: false,
        duration: Some(2.0),
    };
    let err = encode::encode(&fx.ffmpeg, &support, &request, &CancelSoon::default()).unwrap_err();
    assert!(matches!(err, MediaError::Cancelled), "{err}");
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[derive(Default)]
struct Progress {
    last: Cell<f32>,
}

impl Control for Progress {
    fn checkpoint(&self) -> Result<(), MediaError> {
        Ok(())
    }

    fn progress(&self, fraction: f32) {
        self.last.set(fraction);
    }
}

/// cancels on the second checkpoint, after ffmpeg has started
#[derive(Default)]
struct CancelSoon {
    calls: Cell<u32>,
}

impl Control for CancelSoon {
    fn checkpoint(&self) -> Result<(), MediaError> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() > 1 {
            Err(MediaError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn progress(&self, _fraction: f32) {}
}
