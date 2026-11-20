//! [`MediaInfo`]: everything vellum knows about a file after probing it, in one shape for
//! images, animations, video and audio. the ui shows it in the info panel and later phases
//! read it to pick encoders, frame rates and colour handling.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MediaKind {
    Image,
    /// gif, animated webp, apng
    Animation,
    Video,
    Audio,
    /// something ffprobe opened that has no picture or sound, like a subtitle file
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaInfo {
    pub path: String,
    pub kind: MediaKind,
    /// short format id, eg "mp4", "png", "heic"
    pub format: String,
    /// eg "MPEG-4 video", "PNG image"
    pub format_label: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    pub duration_secs: Option<f64>,
    /// overall bits per second
    #[ts(type = "number | null")]
    pub bit_rate: Option<u64>,
    /// the first video stream that is not cover art
    pub video: Option<VideoInfo>,
    /// the first audio stream
    pub audio: Option<AudioInfo>,
    /// set for stills and animations
    pub image: Option<ImageInfo>,
    pub streams: Vec<StreamInfo>,
    pub tags: Vec<MetadataTag>,
    /// things that could not be read, shown so a half read file does not look complete
    pub warnings: Vec<String>,
}

impl MediaInfo {
    /// the size a picture is shown at, after rotation and non square pixels
    pub fn display_size(&self) -> Option<(u32, u32)> {
        if let Some(video) = &self.video {
            return Some((video.display_width, video.display_height));
        }
        self.image
            .as_ref()
            .map(|image| (image.display_width, image.display_height))
    }

    pub fn has_metadata(&self) -> bool {
        !self.tags.is_empty()
    }
}

/// a frame rate as a fraction, so 30000/1001 stays exact
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub fn new(num: i64, den: i64) -> Self {
        Self { num, den }
    }

    pub fn as_f64(self) -> Option<f64> {
        (self.den != 0 && self.num != 0).then(|| self.num as f64 / self.den as f64)
    }

    /// parses "30000/1001", "25" or "1.5"
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some((num, den)) = text.split_once(['/', ':']) {
            let rational = Self::new(num.trim().parse().ok()?, den.trim().parse().ok()?);
            return (rational.den != 0).then_some(rational);
        }
        if let Ok(whole) = text.parse::<i64>() {
            return Some(Self::new(whole, 1));
        }
        let value: f64 = text.parse().ok()?;
        value
            .is_finite()
            .then(|| Self::new((value * 1000.0).round() as i64, 1000))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FrameRateMode {
    Constant,
    /// phone recordings and screen captures often change frame timing as they go
    Variable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VideoInfo {
    pub stream_index: u32,
    /// eg "h264", "hevc", "av1"
    pub codec: String,
    /// eg "H.264 / AVC"
    pub codec_label: String,
    pub profile: Option<String>,
    /// stored size
    pub width: u32,
    pub height: u32,
    /// shown size, after rotation and non square pixels
    pub display_width: u32,
    pub display_height: u32,
    /// clockwise degrees to turn the stored picture to show it upright: 0, 90, 180 or 270
    pub rotation: u32,
    pub frame_rate: Option<Rational>,
    pub fps: Option<f64>,
    pub frame_rate_mode: FrameRateMode,
    /// exact when the container records it, none otherwise
    #[ts(type = "number | null")]
    pub frame_count: Option<u64>,
    #[ts(type = "number | null")]
    pub bit_rate: Option<u64>,
    /// eg "yuv420p10le"
    pub pixel_format: Option<String>,
    pub bit_depth: Option<u32>,
    pub color: ColorInfo,
    pub hdr: Option<HdrInfo>,
    pub interlaced: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColorInfo {
    /// matrix, eg "bt709", "bt2020nc"
    pub space: Option<String>,
    pub primaries: Option<String>,
    /// eg "bt709", "smpte2084" (pq), "arib-std-b67" (hlg)
    pub transfer: Option<String>,
    /// "tv" (limited) or "pc" (full)
    pub range: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HdrFormat {
    Hdr10,
    Hdr10Plus,
    Hlg,
    DolbyVision,
}

impl HdrFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hdr10 => "HDR10",
            Self::Hdr10Plus => "HDR10+",
            Self::Hlg => "HLG",
            Self::DolbyVision => "Dolby Vision",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HdrInfo {
    pub format: HdrFormat,
    /// mastering display peak brightness in nits
    pub mastering_max_nits: Option<f64>,
    pub mastering_min_nits: Option<f64>,
    /// brightest pixel in the whole video (maxcll), in nits
    pub max_content_nits: Option<u32>,
    /// brightest frame average (maxfall), in nits
    pub max_frame_average_nits: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioInfo {
    pub stream_index: u32,
    pub codec: String,
    pub codec_label: String,
    pub profile: Option<String>,
    pub channels: u32,
    /// eg "stereo", "5.1(side)"
    pub channel_layout: Option<String>,
    pub sample_rate: u32,
    /// eg "fltp", "s16"
    pub sample_format: Option<String>,
    pub bit_depth: Option<u32>,
    #[ts(type = "number | null")]
    pub bit_rate: Option<u64>,
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    /// after exif orientation
    pub display_width: u32,
    pub display_height: u32,
    /// 1 for stills
    pub frames: u32,
    /// for animations, how many times they play. 0 is forever.
    pub loop_count: Option<u32>,
    pub has_alpha: bool,
    /// bits per channel
    pub bit_depth: Option<u32>,
    /// eg "RGB", "Grayscale", "CMYK", "Indexed"
    pub color_model: Option<String>,
    pub has_icc_profile: bool,
    /// exif orientation, 1 to 8. 1 is upright.
    pub orientation: u32,
    /// pixels per inch, when the file says
    pub dpi: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    /// cover art inside an audio or video file
    Picture,
    Data,
    Attachment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamInfo {
    pub index: u32,
    pub kind: StreamKind,
    pub codec: Option<String>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MetadataTag {
    /// where it came from, eg "File", "Stream 0", "EXIF", "GPS"
    pub group: String,
    pub key: String,
    pub value: String,
    /// things like gps position, serial numbers and owner names that people often
    /// do not mean to share
    pub sensitive: bool,
}

impl MetadataTag {
    pub fn new(group: impl Into<String>, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let group = group.into();
        let sensitive = is_sensitive(&group, &key);
        Self {
            group,
            key,
            value: value.into(),
            sensitive,
        }
    }
}

fn is_sensitive(group: &str, key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    group.eq_ignore_ascii_case("gps")
        || [
            "gps",
            "location",
            "serial",
            "owner",
            "artist",
            "author",
            "copyright",
            "creator",
            "make",
            "model",
            "lens",
            "com.apple.quicktime",
        ]
        .iter()
        .any(|word| key.contains(word))
}

/// the size after turning a picture by `rotation` degrees clockwise
pub fn rotated_size(width: u32, height: u32, rotation: u32) -> (u32, u32) {
    if rotation % 180 == 90 {
        (height, width)
    } else {
        (width, height)
    }
}

/// exif orientations 5 to 8 swap width and height
pub fn oriented_size(width: u32, height: u32, orientation: u32) -> (u32, u32) {
    if (5..=8).contains(&orientation) {
        (height, width)
    } else {
        (width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rationals_parse_every_way_ffprobe_writes_them() {
        assert_eq!(
            Rational::parse("30000/1001"),
            Some(Rational::new(30000, 1001))
        );
        assert_eq!(Rational::parse("25"), Some(Rational::new(25, 1)));
        assert_eq!(Rational::parse("16:9"), Some(Rational::new(16, 9)));
        assert_eq!(Rational::parse("0/0"), None);
        assert_eq!(Rational::parse("1.5"), Some(Rational::new(1500, 1000)));
        assert_eq!(Rational::parse("nope"), None);
        let ntsc = Rational::parse("30000/1001")
            .and_then(Rational::as_f64)
            .unwrap();
        assert!((ntsc - 29.97).abs() < 0.001);
        assert_eq!(Rational::new(0, 1).as_f64(), None);
    }

    #[test]
    fn rotation_and_orientation_swap_sides() {
        assert_eq!(rotated_size(1920, 1080, 90), (1080, 1920));
        assert_eq!(rotated_size(1920, 1080, 180), (1920, 1080));
        assert_eq!(oriented_size(4000, 3000, 6), (3000, 4000));
        assert_eq!(oriented_size(4000, 3000, 3), (4000, 3000));
    }

    #[test]
    fn location_and_identity_tags_are_flagged() {
        assert!(MetadataTag::new("GPS", "GPSLatitude", "51").sensitive);
        assert!(MetadataTag::new("File", "location", "+51.5-000.1/").sensitive);
        assert!(MetadataTag::new("EXIF", "BodySerialNumber", "123").sensitive);
        assert!(!MetadataTag::new("File", "encoder", "Lavf").sensitive);
    }
}
