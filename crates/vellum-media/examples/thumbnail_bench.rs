//! times thumbnail generation for a 4k h265 video, the case phase 2 of the plan asks about.
//!
//! cargo run --release -p vellum-media --example thumbnail_bench
//!
//! it makes a 10 second 4k hevc clip with ffmpeg (2 second keyframe interval, like most
//! cameras), then times a cold thumbnail (probe plus frame grab), a cached one, and a
//! 10 frame filmstrip.

// this is a command line report, printing is its whole job
#![allow(clippy::print_stdout)]

use std::time::Instant;

use vellum_media::cache::CacheKey;
use vellum_media::ffmpeg::{FfmpegTools, NoReporter, probe_media};
use vellum_media::sniff::Format;
use vellum_media::thumbnail::{make_filmstrip, make_thumbnail};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tools = FfmpegTools::locate(None)?;
    let dir = std::env::temp_dir().join("vellum-thumbnail-bench");
    std::fs::create_dir_all(&dir)?;
    let clip = dir.join("4k-hevc.mp4");
    if !clip.exists() {
        println!("making a 10 second 4k hevc clip, this takes a minute");
        let mut cmd = tools.ffmpeg();
        cmd.args(["-y", "-f", "lavfi", "-i", "testsrc2=s=3840x2160:r=30:d=10"])
            .args([
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-x265-params",
                "keyint=60:log-level=error",
            ])
            .args(["-pix_fmt", "yuv420p10le", "-tag:v", "hvc1"])
            .arg(&clip);
        let status = cmd.status()?;
        if !status.success() {
            return Err("ffmpeg could not make the clip (is libx265 there?)".into());
        }
    }

    println!("ffmpeg {} at {}", tools.version, tools.ffmpeg.display());
    let cold = Instant::now();
    let info = probe_media(&tools, &clip)?;
    let probe_ms = cold.elapsed().as_secs_f64() * 1000.0;
    let grab = Instant::now();
    let thumb =
        make_thumbnail(&clip, Format::Media, &info, 320, Some(&tools))?.ok_or("no thumbnail")?;
    let grab_ms = grab.elapsed().as_secs_f64() * 1000.0;
    println!(
        "probe: {probe_ms:.0} ms, thumbnail {}x{}: {grab_ms:.0} ms, total {:.0} ms",
        thumb.width(),
        thumb.height(),
        probe_ms + grab_ms
    );

    let cache = vellum_media::cache::MediaCache::open(&dir.join("cache"), 1 << 30)?;
    let key = CacheKey::for_file(&clip, "thumb", "320")?;
    cache.put_file(&key, "jpg", None, |target| {
        thumb
            .to_rgb8()
            .save_with_format(target, image::ImageFormat::Jpeg)
            .map_err(|err| vellum_media::MediaError::decode(err.to_string()))
    })?;
    let hit = Instant::now();
    let found = cache.get(&key)?.is_some();
    println!(
        "cached thumbnail lookup: {:.2} ms (found: {found})",
        hit.elapsed().as_secs_f64() * 1000.0
    );

    let strip = Instant::now();
    let (image, _) = make_filmstrip(&tools, &clip, &info, 10, 90, &NoReporter)?;
    println!(
        "filmstrip of 10 frames ({}x{}): {:.0} ms",
        image.width(),
        image.height(),
        strip.elapsed().as_secs_f64() * 1000.0
    );
    print_cpu();
    Ok(())
}

fn print_cpu() {
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    println!("on {threads} hardware threads");
}
