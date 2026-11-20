//! ffmpeg and ffprobe, driven as separate programs.
//!
//! vellum runs the `ffmpeg` and `ffprobe` executables instead of linking the libraries. see
//! `ARCHITECTURE.md` section 22 for why. this module finds them, runs them so cancel and
//! pause work, and turns their output into vellum types.

mod hw;
mod probe;
mod process;
mod seek;

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use ts_rs::TS;

use crate::error::{MediaError, MediaResult};

pub use hw::{HardwareSupport, HwDecoder, HwEncoder, VideoCodec, detect_hardware};
pub use probe::{frame_rate_mode, probe_media};
pub use process::{NoReporter, Reporter};
pub(crate) use process::{command, read_full, run_lines, run_output, run_reading};
pub use seek::{FrameIndex, FrameRequest, extract_frame, frame_seek_time};

/// where ffmpeg and ffprobe live and which version they are
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// eg "6.1.1-3ubuntu5" or "n7.1"
    pub version: String,
}

/// what the settings and info panels show about ffmpeg
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FfmpegStatus {
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub version: String,
}

impl From<&FfmpegTools> for FfmpegStatus {
    fn from(tools: &FfmpegTools) -> Self {
        Self {
            ffmpeg_path: tools.ffmpeg.display().to_string(),
            ffprobe_path: tools.ffprobe.display().to_string(),
            version: tools.version.clone(),
        }
    }
}

impl FfmpegTools {
    /// finds a working ffmpeg and ffprobe. `custom` is the file or folder from settings and
    /// wins over everything else. after that come `VELLUM_FFMPEG_DIR`, the folder vellum
    /// itself is in (where installers put the bundled copy), then the usual install places
    /// and `PATH`.
    pub fn locate(custom: Option<&Path>) -> MediaResult<Self> {
        let mut tried = Vec::new();
        for dir in search_dirs(custom) {
            let ffmpeg = dir.join(exe("ffmpeg"));
            let ffprobe = dir.join(exe("ffprobe"));
            if tried.contains(&ffmpeg) || !ffmpeg.is_file() || !ffprobe.is_file() {
                continue;
            }
            tried.push(ffmpeg.clone());
            match version_of(&ffmpeg) {
                Ok(version) => {
                    tracing::info!(path = %ffmpeg.display(), %version, "found ffmpeg");
                    return Ok(Self {
                        ffmpeg,
                        ffprobe,
                        version,
                    });
                }
                Err(err) => tracing::warn!(path = %ffmpeg.display(), %err, "ffmpeg does not run"),
            }
        }
        Err(MediaError::FfmpegMissing)
    }

    /// `ffmpeg` with the flags every call wants: no banner, no keyboard input, errors only
    pub fn ffmpeg(&self) -> Command {
        let mut cmd = process::command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
        cmd
    }

    pub fn ffprobe(&self) -> Command {
        let mut cmd = process::command(&self.ffprobe);
        cmd.args(["-hide_banner", "-loglevel", "error"]);
        cmd
    }

    /// major and minor version, when the build says. git builds ("N-11234-g...") do not.
    pub fn version_number(&self) -> Option<(u32, u32)> {
        parse_version_number(&self.version)
    }
}

/// finds another command line tool (like libheif's `heif-dec`) the same way ffmpeg is found
pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    search_dirs(None)
        .into_iter()
        .map(|dir| dir.join(exe(name)))
        .find(|path| path.is_file())
}

fn exe(name: &str) -> String {
    format!("{name}{}", env::consts::EXE_SUFFIX)
}

fn search_dirs(custom: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(custom) = custom.filter(|p| !p.as_os_str().is_empty()) {
        // a path to the ffmpeg binary itself is fine too
        let is_binary = custom.is_file()
            || custom
                .file_stem()
                .is_some_and(|stem| stem == OsStr::new("ffmpeg"));
        match (is_binary, custom.parent()) {
            (true, Some(parent)) => dirs.push(parent.to_owned()),
            _ => dirs.push(custom.to_owned()),
        }
    }
    if let Some(dir) = env::var_os("VELLUM_FFMPEG_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_owned))
    {
        dirs.push(dir);
    }
    // apps started from the dock or start menu do not get the shell's PATH, so look in the
    // places package managers use as well
    if cfg!(target_os = "macos") {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
    }
    if cfg!(windows) {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft\\WinGet\\Links"));
        }
        if let Some(home) = env::var_os("USERPROFILE") {
            dirs.push(PathBuf::from(home).join("scoop\\shims"));
        }
        dirs.push(PathBuf::from("C:\\ProgramData\\chocolatey\\bin"));
    }
    if let Some(path) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path));
    }
    dirs
}

fn version_of(ffmpeg: &Path) -> MediaResult<String> {
    let mut cmd = process::command(ffmpeg);
    cmd.arg("-version");
    let out = run_output(&mut cmd, "ffmpeg")?;
    let text = String::from_utf8_lossy(&out);
    parse_version_line(&text)
        .ok_or_else(|| MediaError::tool("ffmpeg", "it did not say its version"))
}

/// "ffmpeg version 6.1.1-3ubuntu5 Copyright (c) ..." gives "6.1.1-3ubuntu5"
fn parse_version_line(text: &str) -> Option<String> {
    let line = text.lines().next()?;
    let rest = line.strip_prefix("ffmpeg version ")?;
    rest.split_whitespace().next().map(str::to_owned)
}

fn parse_version_number(version: &str) -> Option<(u32, u32)> {
    let version = version.trim_start_matches('n');
    let mut parts = version.split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_from_every_kind_of_build() {
        let line = "ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023 the FFmpeg developers\n";
        assert_eq!(parse_version_line(line).as_deref(), Some("6.1.1-3ubuntu5"));
        assert_eq!(parse_version_number("6.1.1-3ubuntu5"), Some((6, 1)));
        assert_eq!(parse_version_number("n7.1"), Some((7, 1)));
        assert_eq!(parse_version_number("8.0"), Some((8, 0)));
        assert_eq!(parse_version_number("N-112233-gabcdef"), None);
        assert_eq!(parse_version_line("something else"), None);
    }

    #[test]
    fn a_custom_binary_path_searches_its_folder_first() {
        let dirs = search_dirs(Some(Path::new("/opt/ffmpeg/bin/ffmpeg")));
        assert_eq!(dirs.first(), Some(&PathBuf::from("/opt/ffmpeg/bin")));
        let dirs = search_dirs(Some(Path::new("/opt/ffmpeg/bin")));
        assert_eq!(dirs.first(), Some(&PathBuf::from("/opt/ffmpeg/bin")));
        let dirs = search_dirs(Some(Path::new("")));
        assert_ne!(dirs.first(), Some(&PathBuf::new()));
    }
}

/// shared by the tests in this crate that need the real ffmpeg
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// the tools, or none when they are not installed. ci sets `VELLUM_REQUIRE_FFMPEG` so a
    /// missing ffmpeg fails there instead of quietly skipping.
    pub fn tools() -> Option<FfmpegTools> {
        match FfmpegTools::locate(None) {
            Ok(tools) => Some(tools),
            Err(err) => {
                assert!(
                    env::var_os("VELLUM_REQUIRE_FFMPEG").is_none(),
                    "ffmpeg is required here: {err}"
                );
                eprintln!("skipping: {err}");
                None
            }
        }
    }

    /// runs ffmpeg to make a test file. `args` are split on whitespace, so no argument may
    /// contain a space (quotes are passed through as they are, ffmpeg filters accept them).
    pub fn make(tools: &FfmpegTools, args: &str, out: &Path) {
        let mut cmd = tools.ffmpeg();
        cmd.arg("-y").args(args.split_whitespace()).arg(out);
        if let Err(err) = run_output(&mut cmd, "ffmpeg") {
            panic!("could not make {}: {err}", out.display());
        }
    }
}
