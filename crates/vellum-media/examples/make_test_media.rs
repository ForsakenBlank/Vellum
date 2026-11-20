//! writes the generated test media into a folder, for trying files by hand.
//!
//! `cargo run -p vellum-media --example make_test_media -- some/folder`

use std::path::PathBuf;
use std::process::ExitCode;

use vellum_media::ffmpeg::Ffmpeg;
use vellum_media::testmedia::TestMedia;

#[allow(clippy::print_stdout)] // a command line tool talks on stdout
fn main() -> ExitCode {
    let dir = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("test-media"), PathBuf::from);
    let result = Ffmpeg::locate().and_then(|ffmpeg| TestMedia::make(&ffmpeg, &dir));
    match result {
        Ok(media) => {
            for name in media.names() {
                println!("made    {name}");
            }
            for (name, reason) in &media.skipped {
                println!("skipped {name}: {reason}");
            }
            println!("in {}", media.dir().display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
