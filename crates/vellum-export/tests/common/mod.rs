//! shared set up for the tests that run ffmpeg: the encoders this machine has, and hard to
//! compress test files made once per test binary.

#![allow(dead_code)] // each test binary uses a different part of this
#![allow(clippy::unwrap_used)] // a failure while setting up is a failed test

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use vellum_export::Env;
use vellum_media::ffmpeg::Ffmpeg;
use vellum_media::ffmpeg::hw::{self, HardwareSupport};
use vellum_media::probe::{self, MediaInfo};

pub struct Fixture {
    pub ffmpeg: Ffmpeg,
    pub hardware: HardwareSupport,
    pub dir: PathBuf,
}

impl Fixture {
    pub fn env<'a>(&'a self, out: &'a Path) -> Env<'a> {
        Env {
            ffmpeg: Some(&self.ffmpeg),
            hardware: &self.hardware,
            allow_hardware: false,
            output_folder: Some(out),
        }
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn probe(&self, name: &str) -> (PathBuf, MediaInfo) {
        let path = self.file(name);
        let info = probe::probe(&path, Some(&self.ffmpeg)).expect("test files probe");
        (path, info)
    }
}

/// noise compresses badly, so these files are big for their length and every tool has to
/// work for its size
const FILES: &[(&str, &[&str])] = &[
    (
        "noisy.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=30:d=6,noise=alls=20:allf=t",
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=d=6:c=pink:r=48000:a=0.3",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "22",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ac",
            "2",
            "-shortest",
        ],
    ),
    (
        "fast.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1280x720:r=60:d=4,noise=alls=12:allf=t",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "26",
            "-pix_fmt",
            "yuv420p",
        ],
    ),
    (
        "noisy.gif",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=400x300:r=12:d=4,noise=alls=25:allf=t",
            "-filter_complex",
            "[0:v]split[a][b];[a]palettegen[p];[b][p]paletteuse",
        ],
    ),
    (
        "clip.gif",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=480x270:r=15:d=4",
            "-filter_complex",
            "[0:v]split[a][b];[a]palettegen[p];[b][p]paletteuse",
        ],
    ),
    (
        "speech.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=d=30:c=brown:r=44100:a=0.4",
            "-ac",
            "2",
        ],
    ),
    (
        "photo.png",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1200x800:r=1:d=1,noise=alls=30:allf=t",
            "-frames:v",
            "1",
        ],
    ),
    (
        "photo.jpg",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1600x1200:r=1:d=1,noise=alls=30:allf=t",
            "-frames:v",
            "1",
            "-q:v",
            "2",
        ],
    ),
];

pub fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let ffmpeg = match Ffmpeg::locate() {
                Ok(ffmpeg) => ffmpeg,
                Err(err) => {
                    assert!(
                        std::env::var_os("VELLUM_REQUIRE_FFMPEG").is_none_or(|v| v.is_empty()),
                        "ffmpeg is required here: {err}"
                    );
                    eprintln!("skipping export tests: {err}");
                    return None;
                }
            };
            let hardware = hw::detect(&ffmpeg).expect("encoders can be listed");
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("export-media");
            std::fs::create_dir_all(&dir).unwrap();
            for (name, args) in FILES {
                let path = dir.join(name);
                if path.exists() {
                    continue;
                }
                let partial = dir.join(format!("part-{name}"));
                let status = ffmpeg
                    .command()
                    .args(*args)
                    .arg("-y")
                    .arg(&partial)
                    .status()
                    .expect("ffmpeg runs");
                assert!(status.success(), "could not make {name}");
                std::fs::rename(&partial, &path).unwrap();
            }
            // a couple of the phase 2 files, made the same way
            let media = vellum_media::testmedia::TestMedia::make(&ffmpeg, &dir.join("media"))
                .expect("phase 2 test media");
            for name in [
                "frames.mp4",
                "h264.mp4",
                "vp9.webm",
                "drawing.svg",
                "exif.jpg",
                "photo.dng",
                "layers.psd",
                "anim.gif",
                "anim.webp",
                "tone.mp3",
                "alpha.png",
            ] {
                if let Some(file) = media.get(name) {
                    let _ = std::fs::copy(file, dir.join(name));
                }
            }
            Some(Fixture {
                ffmpeg,
                hardware,
                dir,
            })
        })
        .as_ref()
}
