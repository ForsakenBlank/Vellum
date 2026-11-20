//! probes every file in a folder and makes its thumbnail, reporting what happened. handy
//! with the files from `tests/media/generate.sh`:
//!
//! cargo run -p vellum-media --example probe_folder -- tests/media/out

// this is a command line report, printing is its whole job
#![allow(clippy::print_stdout)]

use std::path::PathBuf;
use std::time::Instant;

use vellum_media::ffmpeg::{FfmpegTools, probe_media};
use vellum_media::images::probe_image;
use vellum_media::sniff::sniff;
use vellum_media::thumbnail::make_thumbnail;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "tests/media/out".into()),
    );
    let tools = FfmpegTools::locate(None).ok();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let started = Instant::now();
        let format = sniff(&path)?;
        let probed = if format.is_image() {
            probe_image(&path, format, tools.as_ref())
        } else {
            tools
                .as_ref()
                .ok_or(vellum_media::MediaError::FfmpegMissing)
                .and_then(|t| probe_media(t, &path))
        };
        let info = match probed {
            Ok(info) => info,
            Err(err) => {
                println!("{name:24} probe failed: {err}");
                continue;
            }
        };
        let size = info
            .display_size()
            .map_or("-".into(), |(w, h)| format!("{w}x{h}"));
        let thumb = match make_thumbnail(&path, format, &info, 160, tools.as_ref()) {
            Ok(Some(t)) => format!("thumb {}x{}", t.width(), t.height()),
            Ok(None) => "no picture".into(),
            Err(err) => format!("thumb failed: {err}"),
        };
        let extra = info
            .video
            .as_ref()
            .map(|v| {
                let hdr = v.hdr.as_ref().map_or("", |h| h.format.label());
                format!(
                    "{} {:?} rot {} {hdr}",
                    v.codec, v.frame_rate_mode, v.rotation
                )
            })
            .unwrap_or_default();
        println!(
            "{name:24} {:10} {:16} {size:10} {thumb:18} {extra} ({} tags, {} ms)",
            format!("{:?}", info.kind),
            info.format,
            info.tags.len(),
            started.elapsed().as_millis()
        );
    }
    Ok(())
}
