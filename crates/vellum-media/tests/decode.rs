//! every image format decodes, through one api, at the right size and the right way up.

mod common;

use common::{fixture, mean};
use vellum_media::picture::{self, DecodeOptions, PictureFormat};

#[test]
fn every_still_format_decodes_at_full_size() {
    let Some(fx) = fixture() else { return };
    let cases = [
        ("image.png", PictureFormat::Png, 320, 240),
        ("photo.jpg", PictureFormat::Jpeg, 640, 480),
        ("image.webp", PictureFormat::Webp, 320, 240),
        ("image.avif", PictureFormat::Avif, 320, 240),
        ("image.jxl", PictureFormat::Jxl, 320, 240),
        ("image.tiff", PictureFormat::Tiff, 320, 240),
        ("image.bmp", PictureFormat::Bmp, 320, 240),
        ("image.tga", PictureFormat::Tga, 320, 240),
        ("image.qoi", PictureFormat::Qoi, 320, 240),
        ("drawing.svg", PictureFormat::Svg, 120, 80),
        ("layers.psd", PictureFormat::Psd, 48, 32),
        ("photo.dng", PictureFormat::Raw, 96, 64),
    ];
    for (name, format, w, h) in cases {
        let Some(path) = fx.file(name) else { continue };
        let picture = picture::decode(path, DecodeOptions::default(), Some(&fx.ffmpeg))
            .unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(picture.format, format, "{name}");
        assert_eq!((picture.width, picture.height), (w, h), "{name}");
        assert_eq!(picture.first().dimensions(), (w, h), "{name}");
        let (_, dw, dh) = picture::dimensions(path, Some(&fx.ffmpeg)).unwrap();
        assert_eq!((dw, dh), (w, h), "dimensions of {name}");
        let colour = mean(picture.first());
        assert!(
            colour[0] + colour[1] + colour[2] > 30.0,
            "{name} decoded to black"
        );
    }
}

#[test]
fn colours_and_transparency_come_through() {
    let Some(fx) = fixture() else { return };
    if let Some(path) = fx.file("layers.psd") {
        let picture = picture::decode(path, DecodeOptions::default(), None).unwrap();
        let [r, g, b, a] = mean(picture.first());
        assert!((r - 200.0).abs() < 1.0 && (g - 40.0).abs() < 1.0 && (b - 30.0).abs() < 1.0);
        assert!((a - 255.0).abs() < 0.5);
    }
    if let Some(path) = fx.file("alpha.png") {
        let picture = picture::decode(path, DecodeOptions::default(), None).unwrap();
        let [r, _, _, a] = mean(picture.first());
        assert!(r > 250.0, "red is {r}");
        assert!((a - 128.0).abs() < 2.0, "alpha is {a}");
    }
    if let Some(path) = fx.file("photo.dng") {
        // the dng is a left to right gradient in red, so the right edge is redder
        let picture = picture::decode(path, DecodeOptions::default(), None).unwrap();
        let image = picture.first();
        let left = image.get_pixel(2, 30)[0];
        let middle = image.get_pixel(image.width() / 2, 30)[0];
        let right = image.get_pixel(image.width() - 3, 30)[0];
        assert!(
            left < middle && middle < right,
            "left {left}, middle {middle}, right {right}"
        );
    }
}

#[test]
fn photos_are_turned_upright_from_their_exif() {
    let Some(fx) = fixture() else { return };
    let Some(path) = fx.file("exif.jpg") else {
        return;
    };
    let picture = picture::decode(path, DecodeOptions::default(), None).unwrap();
    assert_eq!((picture.width, picture.height), (480, 640));
    assert_eq!(picture.first().dimensions(), (480, 640));
    let (_, w, h) = picture::dimensions(path, None).unwrap();
    assert_eq!((w, h), (480, 640));
    // the fast scaled path must agree
    let preview = picture::decode(path, DecodeOptions::preview(200), None).unwrap();
    assert_eq!(preview.first().dimensions(), (150, 200));
}

#[test]
fn previews_of_big_images_shrink_while_decoding() {
    let Some(fx) = fixture() else { return };
    for name in ["big.jpg", "big.png"] {
        let Some(path) = fx.file(name) else { continue };
        let picture = picture::decode(path, DecodeOptions::preview(400), None).unwrap();
        assert_eq!((picture.width, picture.height), (4000, 3000), "{name}");
        assert_eq!(picture.first().dimensions(), (400, 300), "{name}");
        let colour = mean(picture.first());
        assert!(
            colour[0] + colour[1] + colour[2] > 30.0,
            "{name} preview is black"
        );
    }
}

#[test]
fn svg_renders_at_any_size() {
    let Some(fx) = fixture() else { return };
    let Some(path) = fx.file("drawing.svg") else {
        return;
    };
    let big = picture::decode(path, DecodeOptions::preview(2400), None).unwrap();
    assert_eq!(big.first().dimensions(), (2400, 1600));
    // the circle in the middle is amber
    let centre = big.first().get_pixel(1200, 800);
    assert!(
        centre[0] > 230 && centre[1] > 150 && centre[2] < 90,
        "{centre:?}"
    );
}

#[test]
fn animations_decode_every_frame() {
    let Some(fx) = fixture() else { return };
    let options = DecodeOptions {
        size: None,
        all_frames: true,
    };
    for (name, frames) in [("anim.gif", 10), ("anim.png", 5), ("anim.webp", 5)] {
        let Some(path) = fx.file(name) else { continue };
        let picture =
            picture::decode(path, options, None).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(picture.frames.len(), frames, "{name}");
        assert!(
            picture
                .frames
                .iter()
                .all(|f| f.image.dimensions() == (160, 120))
        );
        assert!(
            picture
                .frames
                .iter()
                .all(|f| (90..=110).contains(&f.delay_ms)),
            "{name}"
        );
    }
    // without all_frames only the first comes back
    if let Some(path) = fx.file("anim.gif") {
        let picture = picture::decode(path, DecodeOptions::default(), None).unwrap();
        assert_eq!(picture.frames.len(), 1);
    }
}

#[test]
fn ffmpeg_formats_say_so_when_ffmpeg_is_missing() {
    let Some(fx) = fixture() else { return };
    let Some(path) = fx.file("image.avif") else {
        return;
    };
    let err = picture::decode(path, DecodeOptions::default(), None).unwrap_err();
    assert!(err.to_string().contains("FFmpeg"), "{err}");
}

#[test]
fn files_that_are_not_images_are_refused_plainly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "hello").unwrap();
    let err = picture::decode(&path, DecodeOptions::default(), None).unwrap_err();
    assert_eq!(
        err.to_string(),
        "notes.txt is not an image format Vellum can read."
    );
    let broken = dir.path().join("broken.png");
    std::fs::write(&broken, b"\x89PNG\r\n\x1a\nnot really").unwrap();
    let err = picture::decode(&broken, DecodeOptions::default(), None).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("Vellum could not read broken.png"),
        "{err}"
    );
}
