//! times thumbnail and filmstrip generation for a 4k h265 video.
//!
//! `cargo run --release -p vellum-media --example bench_thumbnail`
//!
//! makes a 20 second 3840x2160 h265 clip with a keyframe every 2 seconds (like a phone or
//! camera) the first time, then reports the median of several runs. the result is recorded
//! in `ARCHITECTURE.md` section 22.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use vellum_media::control::Unattended;
use vellum_media::ffmpeg::Ffmpeg;
use vellum_media::ffmpeg::process;
use vellum_media::{probe, thumbs};

const RUNS: usize = 7;

#[allow(clippy::print_stdout)] // a benchmark reports on stdout
fn main() -> ExitCode {
    let ffmpeg = match Ffmpeg::locate() {
        Ok(ffmpeg) => ffmpeg,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let dir = std::env::temp_dir().join("vellum-bench");
    let clip = dir.join("4k-h265.mp4");
    if !clip.is_file() {
        println!("making a 20 second 4k h265 clip, this takes a minute...");
        if let Err(err) = make_clip(&ffmpeg, &clip) {
            eprintln!("could not make the clip: {err}");
            return ExitCode::FAILURE;
        }
    }

    let probe_time = median(|| probe::probe(&clip, Some(&ffmpeg)).map(drop));
    let info = match probe::probe(&clip, Some(&ffmpeg)) {
        Ok(info) => info,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let thumb_time =
        median(|| thumbs::thumbnail(Some(&ffmpeg), &clip, &info, thumbs::THUMB_SIZE).map(drop));
    let strip_time =
        median(|| thumbs::filmstrip(&ffmpeg, &clip, &info, 8, 90, &Unattended).map(drop));

    println!("ffmpeg {}", ffmpeg.info().version);
    println!(
        "clip   {} ({}x{})",
        clip.display(),
        info.width.unwrap_or(0),
        info.height.unwrap_or(0)
    );
    println!("probe              {:>7.1} ms", ms(probe_time));
    println!("thumbnail          {:>7.1} ms", ms(thumb_time));
    println!("filmstrip, 8 frames{:>7.1} ms", ms(strip_time));
    ExitCode::SUCCESS
}

fn make_clip(ffmpeg: &Ffmpeg, clip: &Path) -> vellum_media::MediaResult<()> {
    std::fs::create_dir_all(clip.parent().unwrap_or(Path::new(".")))
        .map_err(|err| vellum_media::MediaError::io(clip, err))?;
    let mut cmd = ffmpeg.command();
    cmd.args(["-f", "lavfi", "-i", "testsrc2=s=3840x2160:r=30:d=20"])
        .args([
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args([
            "-x265-params",
            "log-level=error:keyint=60",
            "-tag:v",
            "hvc1",
            "-y",
        ])
        .arg(clip);
    process::capture(cmd, Duration::from_secs(600)).map(drop)
}

fn median(mut run: impl FnMut() -> vellum_media::MediaResult<()>) -> Duration {
    let mut times: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            if let Err(err) = run() {
                eprintln!("a run failed: {err}");
            }
            start.elapsed()
        })
        .collect();
    times.sort();
    times[RUNS / 2]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
