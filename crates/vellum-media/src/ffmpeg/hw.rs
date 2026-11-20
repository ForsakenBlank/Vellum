//! which encoders and hardware decoders this machine can really use.
//!
//! ffmpeg lists every encoder it was built with, whether or not the machine has the gpu
//! for it (a stock linux build lists nvenc on a laptop with no nvidia card). so every
//! hardware encoder gets a tiny test encode, and only the ones that work are offered.
//! software encoders are trusted from the list, they always work.

use std::path::Path;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{Ffmpeg, process};
use crate::error::MediaResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum VideoCodec {
    H264,
    H265,
    Av1,
    Vp9,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HardwareApi {
    Nvenc,
    QuickSync,
    Amf,
    VideoToolbox,
    Vaapi,
}

impl HardwareApi {
    /// for "Encoded with NVIDIA NVENC"
    pub fn label(self) -> &'static str {
        match self {
            Self::Nvenc => "NVIDIA NVENC",
            Self::QuickSync => "Intel Quick Sync",
            Self::Amf => "AMD AMF",
            Self::VideoToolbox => "Apple VideoToolbox",
            Self::Vaapi => "VA-API",
        }
    }

    /// whether this api can exist on the current os at all
    fn possible_here(self) -> bool {
        match self {
            Self::VideoToolbox => cfg!(target_os = "macos"),
            Self::Vaapi => cfg!(target_os = "linux") && Path::new(VAAPI_DEVICE).exists(),
            Self::Nvenc | Self::QuickSync | Self::Amf => !cfg!(target_os = "macos"),
        }
    }
}

/// the render node vaapi encoders open
pub const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EncoderInfo {
    /// the ffmpeg name, eg "h264_nvenc"
    pub name: String,
    pub codec: VideoCodec,
    /// none for software encoders
    pub hardware: Option<HardwareApi>,
}

impl EncoderInfo {
    /// arguments that go before `-i`, for encoders that need a device opened
    pub fn input_args(&self) -> Vec<String> {
        match self.hardware {
            Some(HardwareApi::Vaapi) => vec!["-vaapi_device".into(), VAAPI_DEVICE.into()],
            _ => Vec::new(),
        }
    }

    /// the last filters before the encoder: pixel format, plus an upload for vaapi
    pub fn final_filters(&self) -> &'static str {
        match self.hardware {
            Some(HardwareApi::Vaapi) => "format=nv12,hwupload",
            Some(HardwareApi::QuickSync) => "format=nv12",
            _ => "format=yuv420p",
        }
    }
}

/// what the machine can do, found once at start-up
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HardwareSupport {
    /// working video encoders, hardware first within each codec
    pub encoders: Vec<EncoderInfo>,
    /// audio encoders ffmpeg has, eg "aac", "libopus"
    pub audio_encoders: Vec<String>,
    /// hardware decoding methods ffmpeg lists, eg "cuda", "vaapi"
    pub hardware_decoders: Vec<String>,
}

impl HardwareSupport {
    /// the best encoder for a codec: a working hardware one if allowed, else software
    pub fn pick(&self, codec: VideoCodec, allow_hardware: bool) -> Option<&EncoderInfo> {
        let mut options = self.encoders.iter().filter(|e| e.codec == codec);
        if allow_hardware {
            options.next()
        } else {
            options.find(|e| e.hardware.is_none())
        }
    }

    pub fn software(&self, codec: VideoCodec) -> Option<&EncoderInfo> {
        self.pick(codec, false)
    }

    pub fn has_hardware_encoder(&self) -> bool {
        self.encoders.iter().any(|e| e.hardware.is_some())
    }
}

/// every encoder vellum knows how to drive, in order of preference within a codec
const CANDIDATES: &[(&str, VideoCodec, Option<HardwareApi>)] = &[
    ("h264_nvenc", VideoCodec::H264, Some(HardwareApi::Nvenc)),
    ("h264_qsv", VideoCodec::H264, Some(HardwareApi::QuickSync)),
    ("h264_amf", VideoCodec::H264, Some(HardwareApi::Amf)),
    (
        "h264_videotoolbox",
        VideoCodec::H264,
        Some(HardwareApi::VideoToolbox),
    ),
    ("h264_vaapi", VideoCodec::H264, Some(HardwareApi::Vaapi)),
    ("libx264", VideoCodec::H264, None),
    ("libopenh264", VideoCodec::H264, None),
    ("hevc_nvenc", VideoCodec::H265, Some(HardwareApi::Nvenc)),
    ("hevc_qsv", VideoCodec::H265, Some(HardwareApi::QuickSync)),
    ("hevc_amf", VideoCodec::H265, Some(HardwareApi::Amf)),
    (
        "hevc_videotoolbox",
        VideoCodec::H265,
        Some(HardwareApi::VideoToolbox),
    ),
    ("hevc_vaapi", VideoCodec::H265, Some(HardwareApi::Vaapi)),
    ("libx265", VideoCodec::H265, None),
    ("av1_nvenc", VideoCodec::Av1, Some(HardwareApi::Nvenc)),
    ("av1_qsv", VideoCodec::Av1, Some(HardwareApi::QuickSync)),
    ("av1_amf", VideoCodec::Av1, Some(HardwareApi::Amf)),
    ("av1_vaapi", VideoCodec::Av1, Some(HardwareApi::Vaapi)),
    ("libsvtav1", VideoCodec::Av1, None),
    ("libaom-av1", VideoCodec::Av1, None),
    ("vp9_qsv", VideoCodec::Vp9, Some(HardwareApi::QuickSync)),
    ("vp9_vaapi", VideoCodec::Vp9, Some(HardwareApi::Vaapi)),
    ("libvpx-vp9", VideoCodec::Vp9, None),
];

const AUDIO_CANDIDATES: &[&str] = &["aac", "libopus", "libmp3lame", "flac", "pcm_s16le"];

/// lists encoders, then test encodes every hardware one in parallel. takes a second or two
/// on most machines, so call it off the ui thread.
pub fn detect(ffmpeg: &Ffmpeg) -> MediaResult<HardwareSupport> {
    let mut cmd = ffmpeg.command();
    cmd.arg("-encoders");
    let listed =
        String::from_utf8_lossy(&process::capture(cmd, Duration::from_secs(10))?).into_owned();
    let listed = encoder_names(&listed);

    let mut cmd = ffmpeg.command();
    cmd.arg("-hwaccels");
    let hardware_decoders = process::capture(cmd, Duration::from_secs(10))
        .map(|out| hwaccel_names(&String::from_utf8_lossy(&out)))
        .unwrap_or_default();

    let candidates: Vec<EncoderInfo> = CANDIDATES
        .iter()
        .filter(|(name, _, api)| {
            listed.iter().any(|l| l == name) && api.is_none_or(HardwareApi::possible_here)
        })
        .map(|(name, codec, api)| EncoderInfo {
            name: (*name).to_owned(),
            codec: *codec,
            hardware: *api,
        })
        .collect();

    let checks: Vec<_> = candidates
        .into_iter()
        .map(|encoder| {
            let ffmpeg = ffmpeg.clone();
            thread::spawn(move || {
                let works = encoder.hardware.is_none() || test_encode(&ffmpeg, &encoder);
                if encoder.hardware.is_some() {
                    tracing::debug!(encoder = %encoder.name, works, "checked a hardware encoder");
                }
                works.then_some(encoder)
            })
        })
        .collect();
    let encoders = checks
        .into_iter()
        .filter_map(|check| check.join().ok().flatten())
        .collect();

    Ok(HardwareSupport {
        encoders,
        audio_encoders: AUDIO_CANDIDATES
            .iter()
            .filter(|name| listed.iter().any(|l| l == *name))
            .map(|name| (*name).to_owned())
            .collect(),
        hardware_decoders,
    })
}

/// encodes a few frames of black to nowhere. drivers that are missing, too old or busy all
/// show up as a failure here instead of halfway through someone's export.
fn test_encode(ffmpeg: &Ffmpeg, encoder: &EncoderInfo) -> bool {
    let mut cmd = ffmpeg.command();
    cmd.args(encoder.input_args())
        .args(["-f", "lavfi", "-i", "color=c=black:s=320x240:r=25:d=0.2"])
        .args(["-vf", encoder.final_filters()])
        .args(["-frames:v", "3", "-c:v", &encoder.name, "-f", "null", "-"]);
    process::capture(cmd, Duration::from_secs(15)).is_ok()
}

/// the names column of `ffmpeg -encoders`, eg " V....D libx264  libx264 H.264 ..."
fn encoder_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let flags = parts.next()?;
            let name = parts.next()?;
            let is_flags =
                flags.len() == 6 && flags.chars().all(|c| c == '.' || c.is_ascii_alphabetic());
            (is_flags && name != "=").then(|| name.to_owned())
        })
        .collect()
}

/// `ffmpeg -hwaccels` prints a heading then one name per line
fn hwaccel_names(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.ends_with(':'))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_list_is_parsed() {
        let text = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx264              libx264 H.264\n V....D h264_nvenc           NVIDIA NVENC H.264 encoder\n A....D aac                  AAC (Advanced Audio Coding)\n";
        assert_eq!(encoder_names(text), ["libx264", "h264_nvenc", "aac"]);
    }

    #[test]
    fn hwaccels_are_parsed() {
        let text = "Hardware acceleration methods:\nvdpau\ncuda\nvaapi\n\n";
        assert_eq!(hwaccel_names(text), ["vdpau", "cuda", "vaapi"]);
    }

    #[test]
    fn hardware_wins_only_when_allowed() {
        let support = HardwareSupport {
            encoders: vec![
                EncoderInfo {
                    name: "h264_nvenc".into(),
                    codec: VideoCodec::H264,
                    hardware: Some(HardwareApi::Nvenc),
                },
                EncoderInfo {
                    name: "libx264".into(),
                    codec: VideoCodec::H264,
                    hardware: None,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            support.pick(VideoCodec::H264, true).unwrap().name,
            "h264_nvenc"
        );
        assert_eq!(
            support.pick(VideoCodec::H264, false).unwrap().name,
            "libx264"
        );
        assert!(support.pick(VideoCodec::Av1, true).is_none());
        assert!(support.has_hardware_encoder());
    }
}
