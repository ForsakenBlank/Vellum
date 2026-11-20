//! shared set up for the tests that need real media files.

#![allow(dead_code)] // each test binary uses a different part of this

use std::path::Path;
use std::sync::OnceLock;

use vellum_media::ffmpeg::Ffmpeg;
use vellum_media::testmedia::TestMedia;

pub struct Fixture {
    pub ffmpeg: Ffmpeg,
    pub media: TestMedia,
}

impl Fixture {
    /// the path of a test file, or none (with a note) if this ffmpeg could not make it
    pub fn file(&self, name: &str) -> Option<&Path> {
        let found = self.media.get(name);
        if found.is_none() {
            let reason = self
                .media
                .skipped
                .get(name)
                .map_or("unknown", String::as_str);
            eprintln!("skipping {name}: {reason}");
        }
        found
    }
}

/// generated once per test binary into cargo's temp folder. without ffmpeg the tests that
/// need it are skipped, unless VELLUM_REQUIRE_FFMPEG is set (ci sets it) so a broken
/// install cannot pass silently. VELLUM_REQUIRE_ALL_TEST_MEDIA also fails if any test file
/// could not be made; ci sets it on linux, where the ffmpeg build is known to have every
/// encoder the files need.
pub fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let ffmpeg = match Ffmpeg::locate() {
                Ok(ffmpeg) => ffmpeg,
                Err(err) => {
                    assert!(
                        !flag("VELLUM_REQUIRE_FFMPEG"),
                        "ffmpeg is required here: {err}"
                    );
                    eprintln!("skipping media tests: {err}");
                    return None;
                }
            };
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-media");
            let media = TestMedia::make(&ffmpeg, &dir).expect("the test media folder can be made");
            if flag("VELLUM_REQUIRE_ALL_TEST_MEDIA") {
                assert!(
                    media.skipped.is_empty(),
                    "every test file should be made in ci: {:?}",
                    media.skipped
                );
            }
            Some(Fixture { ffmpeg, media })
        })
        .as_ref()
}

/// an environment variable that is set to something, so ci can pass an empty value to mean no
fn flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

/// the average colour of an image
pub fn mean(image: &image::RgbaImage) -> [f64; 4] {
    let mut sum = [0.0; 4];
    for p in image.pixels() {
        for (s, c) in sum.iter_mut().zip(p.0) {
            *s += f64::from(c);
        }
    }
    let n = image.pixels().len().max(1) as f64;
    sum.map(|s| s / n)
}
