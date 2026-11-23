//! ffmpeg and ffprobe, run as separate programs.
//!
//! vellum talks to ffmpeg through its command line rather than linking its libraries. see
//! `ARCHITECTURE.md` section 22 for why. everything here finds the programs, runs them with
//! cancel and progress, and works out what the machine can encode.

pub mod encode;
pub mod hw;
pub mod process;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use ts_rs::TS;

use crate::error::{MediaError, MediaResult};

/// where to look for ffmpeg before the system path, eg a portable build
pub const FFMPEG_DIR_ENV: &str = "VELLUM_FFMPEG_DIR";

/// what the ui shows about the ffmpeg vellum found
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FfmpegInfo {
    pub path: String,
    /// eg "6.1.1"
    pub version: String,
    /// true for builds that include gpl only parts like x264
    pub gpl: bool,
}

#[derive(Debug, Clone)]
pub struct Ffmpeg {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    info: FfmpegInfo,
}

impl Ffmpeg {
    /// finds ffmpeg in `VELLUM_FFMPEG_DIR`, next to the vellum program (where the installer
    /// puts it), then on the system path
    pub fn locate() -> MediaResult<Self> {
        let mut dirs = Vec::new();
        if let Some(dir) = env::var_os(FFMPEG_DIR_ENV) {
            dirs.push(PathBuf::from(dir));
        }
        if let Some(dir) = env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
        {
            dirs.push(dir);
        }
        if let Some(path) = env::var_os("PATH") {
            dirs.extend(env::split_paths(&path));
        }
        dirs.iter()
            .find_map(|dir| Self::in_dir(dir).ok())
            .ok_or(MediaError::FfmpegMissing)
    }

    /// uses the ffmpeg and ffprobe in one folder
    pub fn in_dir(dir: &Path) -> MediaResult<Self> {
        let ffmpeg = dir.join(program_name("ffmpeg"));
        let ffprobe = dir.join(program_name("ffprobe"));
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            return Err(MediaError::FfmpegMissing);
        }
        let mut cmd = process::command(&ffmpeg);
        cmd.arg("-hide_banner").arg("-version");
        let out = process::capture(cmd, process::QUICK_QUERY)?;
        let text = String::from_utf8_lossy(&out);
        let info = FfmpegInfo {
            path: ffmpeg.display().to_string(),
            version: parse_version(&text).unwrap_or_else(|| "unknown".to_owned()),
            gpl: text.contains("--enable-gpl"),
        };
        Ok(Self {
            ffmpeg,
            ffprobe,
            info,
        })
    }

    pub fn info(&self) -> &FfmpegInfo {
        &self.info
    }

    /// `ffmpeg` with the flags every run wants: quiet, no prompts, no reading the terminal
    pub fn command(&self) -> Command {
        let mut cmd = process::command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-v", "error"]);
        cmd
    }

    pub fn probe_command(&self) -> Command {
        let mut cmd = process::command(&self.ffprobe);
        cmd.args(["-hide_banner", "-v", "error"]);
        cmd
    }
}

fn program_name(name: &str) -> String {
    format!("{name}{}", env::consts::EXE_SUFFIX)
}

/// "ffmpeg version 6.1.1-3ubuntu5 Copyright..." gives "6.1.1-3ubuntu5"
fn parse_version(text: &str) -> Option<String> {
    let line = text.lines().next()?;
    let rest = line.strip_prefix("ffmpeg version ")?;
    rest.split_whitespace().next().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_is_parsed() {
        assert_eq!(
            parse_version("ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023"),
            Some("6.1.1-3ubuntu5".to_owned())
        );
        assert_eq!(
            parse_version("ffmpeg version n7.1 Copyright"),
            Some("n7.1".to_owned())
        );
        assert_eq!(parse_version("something else"), None);
    }

    #[test]
    fn a_folder_without_ffmpeg_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            Ffmpeg::in_dir(dir.path()),
            Err(MediaError::FfmpegMissing)
        ));
    }
}
