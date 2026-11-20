//! export to x mb: makes any video, gif, image or song fit under a size limit.
//!
//! [`budget`] holds the maths, and each kind of file has its own way of getting there:
//! two pass encoding with a correction for video ([`video`]), a quality search then
//! scaling for images ([`image`]), a ladder of smaller settings for gifs ([`gif`]), and a
//! bitrate for audio ([`audio`]). every result is checked on disk, and a result that is
//! still over the limit is never handed back as a success.

pub mod audio;
pub mod budget;
pub mod gif;
pub mod image;
pub mod video;

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_core::settings::{ExportSettings, SizeUnit};
use vellum_media::ffmpeg::hw::VideoCodec;
use vellum_media::probe::{MediaInfo, MediaKind};
use vellum_media::{Control, MediaError, MediaResult};

use crate::{Env, ToolResult};
pub use budget::Strategy;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TargetSize {
    pub amount: f64,
    pub unit: SizeUnit,
}

impl TargetSize {
    pub fn bytes(self) -> u64 {
        // clamped to sane sizes before this is called
        (self.amount.clamp(0.001, 1e6) * self.unit.bytes() as f64) as u64
    }

    /// "10 MB", "9.5 MiB", for file names and messages
    pub fn label(self) -> String {
        let unit = match self.unit {
            SizeUnit::Mb => "MB",
            SizeUnit::Mib => "MiB",
        };
        format!(
            "{} {unit}",
            crate::num((self.amount * 100.0).round() / 100.0)
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ImageTarget {
    /// the same format where it can be squeezed, otherwise jpeg (or webp with transparency)
    #[default]
    Auto,
    Jpeg,
    Webp,
    Avif,
    Png,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AudioTarget {
    /// plays everywhere
    #[default]
    Mp3,
    /// better at low bitrates
    Opus,
}

/// the options of an "export to size" session, as the quick tool document stores them
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct SizeOptions {
    /// none uses the size from settings
    pub target: Option<TargetSize>,
    /// the preset chip the target came from, if any
    pub preset: Option<String>,
    /// none uses the margin from settings
    pub margin_percent: Option<f64>,
    pub strategy: Strategy,
    pub codec: VideoCodec,
    pub mute: bool,
    pub mono: bool,
    /// trim, in seconds. only used when the session has one file.
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub image_format: ImageTarget,
    pub audio_format: AudioTarget,
}

impl Default for SizeOptions {
    fn default() -> Self {
        Self {
            target: None,
            preset: None,
            margin_percent: None,
            strategy: Strategy::BestQuality,
            codec: VideoCodec::H264,
            mute: false,
            mono: false,
            start: None,
            end: None,
            image_format: ImageTarget::Auto,
            audio_format: AudioTarget::Mp3,
        }
    }
}

impl SizeOptions {
    /// the target in effect, with settings filling the gaps
    pub fn target(&self, settings: &ExportSettings) -> TargetSize {
        self.target.unwrap_or(TargetSize {
            amount: settings.target_amount,
            unit: settings.unit,
        })
    }

    /// the bytes to aim for: the limit less the safety margin
    pub fn goal_bytes(&self, settings: &ExportSettings) -> u64 {
        let margin = self
            .margin_percent
            .filter(|m| m.is_finite())
            .unwrap_or(settings.safety_margin_percent);
        budget::target_bytes(self.target(settings).bytes(), margin)
    }

    pub fn trimmed(&self) -> bool {
        self.start.is_some_and(|s| s > 0.0) || self.end.is_some()
    }
}

/// what one run needs besides the file
pub struct SizeJob<'a> {
    pub options: &'a SizeOptions,
    /// the limit after the margin, what the result must stay under
    pub goal: u64,
    /// "10 MB", used in the result's name
    pub label: String,
}

/// fits one file under the goal, picking the method from what the file is
pub fn fit(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    job: &SizeJob,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    match info.kind {
        MediaKind::Video => video::fit(env, input, info, job, control),
        MediaKind::Audio => audio::fit(env, input, info, job, control),
        MediaKind::Animation => gif::fit(env, input, info, job, control),
        MediaKind::Image | MediaKind::Vector => image::fit(env, input, info, job, control),
        MediaKind::Unknown => Err(MediaError::Unsupported(format!(
            "Vellum cannot shrink {} because it does not know what kind of file it is.",
            info.name
        ))),
    }
}

/// the result for a file that is already small enough and needs nothing done
fn already_fits(input: &Path, goal: u64) -> ToolResult {
    let mut result = ToolResult::file(input, input);
    result.already_fits = true;
    result.target_bytes = Some(goal);
    result
}

fn too_big(goal: u64, what: &str) -> MediaError {
    MediaError::Unsupported(format!(
        "Vellum could not get {what} under {} without it falling apart. Pick a bigger size, or trim it first.",
        budget::human_bytes(goal)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_read_from_options_or_settings() {
        let settings = ExportSettings::default();
        let mut options = SizeOptions::default();
        assert_eq!(options.target(&settings).label(), "10 MB");
        assert_eq!(options.goal_bytes(&settings), 9_700_000);
        options.target = Some(TargetSize {
            amount: 20.0,
            unit: SizeUnit::Mib,
        });
        options.margin_percent = Some(0.0);
        assert_eq!(options.goal_bytes(&settings), 20 * 1024 * 1024);
        assert_eq!(options.target(&settings).label(), "20 MiB");
        options.target = Some(TargetSize {
            amount: 9.5,
            unit: SizeUnit::Mb,
        });
        assert_eq!(options.target(&settings).label(), "9.5 MB");
    }

    #[test]
    fn options_from_an_old_session_fill_in_defaults() {
        let options: SizeOptions =
            serde_json::from_value(serde_json::json!({ "mute": true })).unwrap();
        assert!(options.mute);
        assert_eq!(options.codec, VideoCodec::H264);
        assert!(!options.trimmed());
    }
}
