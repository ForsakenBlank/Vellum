//! which hardware encoders and decoders actually work on this machine.
//!
//! an encoder being listed by `ffmpeg -encoders` only means ffmpeg was built with it. nvenc
//! is listed on machines without an nvidia card, quicksync on amd machines and so on. so
//! every listed one is tried for real with a tiny encode, in parallel, with a time limit.
//! decoders are checked by opening their device.

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{FfmpegTools, run_output};
use crate::error::MediaResult;

/// how long one trial encode may take before it counts as not working
const TRIAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
    Vp9,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 4] = [Self::H264, Self::Hevc, Self::Av1, Self::Vp9];

    /// the prefix ffmpeg uses in hardware encoder names, eg `hevc` in `hevc_nvenc`
    fn prefix(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
            Self::Vp9 => "vp9",
        }
    }

    /// software encoders in order of preference
    pub fn software_encoders(self) -> &'static [&'static str] {
        match self {
            Self::H264 => &["libx264", "libopenh264"],
            Self::Hevc => &["libx265"],
            Self::Av1 => &["libsvtav1", "libaom-av1", "librav1e"],
            Self::Vp9 => &["libvpx-vp9"],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "H.265",
            Self::Av1 => "AV1",
            Self::Vp9 => "VP9",
        }
    }
}

/// a hardware api, eg nvenc, and how to talk to it
struct Api {
    suffix: &'static str,
    label: &'static str,
    codecs: &'static [VideoCodec],
}

const ENCODER_APIS: &[Api] = &[
    Api {
        suffix: "nvenc",
        label: "NVIDIA NVENC",
        codecs: &[VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1],
    },
    Api {
        suffix: "qsv",
        label: "Intel Quick Sync",
        codecs: &VideoCodec::ALL,
    },
    Api {
        suffix: "amf",
        label: "AMD AMF",
        codecs: &[VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1],
    },
    Api {
        suffix: "videotoolbox",
        label: "Apple VideoToolbox",
        codecs: &[VideoCodec::H264, VideoCodec::Hevc],
    },
    Api {
        suffix: "vaapi",
        label: "VA-API",
        codecs: &VideoCodec::ALL,
    },
];

const DECODER_APIS: &[(&str, &str)] = &[
    ("cuda", "NVIDIA CUDA"),
    ("qsv", "Intel Quick Sync"),
    ("d3d11va", "Direct3D 11"),
    ("dxva2", "DirectX VA 2"),
    ("videotoolbox", "Apple VideoToolbox"),
    ("vaapi", "VA-API"),
];

/// linux render node used for va-api. most machines with one gpu have exactly this one.
const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HwEncoder {
    /// the ffmpeg encoder name, eg "hevc_nvenc"
    pub name: String,
    pub codec: VideoCodec,
    /// eg "NVIDIA NVENC"
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HwDecoder {
    /// the ffmpeg hwaccel name, eg "cuda"
    pub name: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HardwareSupport {
    /// hardware encoders that passed a trial encode
    pub encoders: Vec<HwEncoder>,
    /// hardware decoders whose device opened
    pub decoders: Vec<HwDecoder>,
    /// software encoders this ffmpeg build has, eg "libx264", "libopus", "gif"
    pub software_encoders: Vec<String>,
    /// the ffmpeg version this was checked with, so an upgrade checks again
    pub ffmpeg_version: String,
}

impl HardwareSupport {
    /// the encoder to use for `codec`: hardware when allowed and working, software
    /// otherwise. none when this ffmpeg cannot write the codec at all.
    pub fn encoder_for(&self, codec: VideoCodec, allow_hardware: bool) -> Option<String> {
        if allow_hardware && let Some(hw) = self.encoders.iter().find(|e| e.codec == codec) {
            return Some(hw.name.clone());
        }
        self.software_encoder_for(codec)
    }

    /// encoders to hand to [`super::run_encode`] for `codec`: the working hardware ones
    /// first when allowed, then the software one, so a failing driver falls back cleanly
    pub fn encoder_chain(&self, codec: VideoCodec, allow_hardware: bool) -> Vec<String> {
        let mut chain: Vec<String> = if allow_hardware {
            self.encoders
                .iter()
                .filter(|e| e.codec == codec)
                .map(|e| e.name.clone())
                .collect()
        } else {
            Vec::new()
        };
        chain.extend(self.software_encoder_for(codec));
        chain
    }

    /// the fallback when a hardware encode fails part way
    pub fn software_encoder_for(&self, codec: VideoCodec) -> Option<String> {
        codec
            .software_encoders()
            .iter()
            .find(|name| self.software_encoders.iter().any(|have| have == *name))
            .map(|name| (*name).to_owned())
    }
}

/// tries every hardware encoder and decoder this ffmpeg was built with. takes a second or
/// two, so callers cache the answer per ffmpeg version.
pub fn detect_hardware(tools: &FfmpegTools) -> MediaResult<HardwareSupport> {
    let mut cmd = tools.ffmpeg();
    cmd.arg("-encoders");
    let listed = parse_encoder_list(&String::from_utf8_lossy(&run_output(&mut cmd, "ffmpeg")?));
    let mut cmd = tools.ffmpeg();
    cmd.arg("-hwaccels");
    let hwaccels = parse_hwaccels(&String::from_utf8_lossy(&run_output(&mut cmd, "ffmpeg")?));

    let candidates: Vec<HwEncoder> = ENCODER_APIS
        .iter()
        .flat_map(|api| {
            api.codecs.iter().map(move |codec| HwEncoder {
                name: format!("{}_{}", codec.prefix(), api.suffix),
                codec: *codec,
                label: api.label.to_owned(),
            })
        })
        .filter(|candidate| listed.contains(&candidate.name))
        .collect();
    let decoder_candidates: Vec<&(&str, &str)> = DECODER_APIS
        .iter()
        .filter(|(name, _)| hwaccels.iter().any(|h| h == name))
        .collect();

    // trials run side by side; on a machine without the hardware each fails in milliseconds
    let (encoders, decoders) = thread::scope(|scope| {
        let encoder_trials: Vec<_> = candidates
            .into_iter()
            .map(|candidate| {
                scope.spawn(move || {
                    trial_encode(&tools.ffmpeg, &candidate.name).then_some(candidate)
                })
            })
            .collect();
        let decoder_trials: Vec<_> = decoder_candidates
            .into_iter()
            .map(|(name, label)| {
                scope.spawn(move || {
                    trial_device(&tools.ffmpeg, name).then(|| HwDecoder {
                        name: (*name).to_owned(),
                        label: (*label).to_owned(),
                    })
                })
            })
            .collect();
        let encoders: Vec<HwEncoder> = encoder_trials
            .into_iter()
            .filter_map(|t| t.join().ok().flatten())
            .collect();
        let decoders: Vec<HwDecoder> = decoder_trials
            .into_iter()
            .filter_map(|t| t.join().ok().flatten())
            .collect();
        (encoders, decoders)
    });

    let software_encoders = SOFTWARE_ENCODERS
        .iter()
        .filter(|name| listed.iter().any(|l| l == *name))
        .map(|name| (*name).to_owned())
        .collect();
    tracing::info!(
        encoders = ?encoders.iter().map(|e| &e.name).collect::<Vec<_>>(),
        decoders = ?decoders.iter().map(|d| &d.name).collect::<Vec<_>>(),
        "checked hardware acceleration"
    );
    Ok(HardwareSupport {
        encoders,
        decoders,
        software_encoders,
        ffmpeg_version: tools.version.clone(),
    })
}

/// software encoders later phases use, reported so the ui can hide what is missing
const SOFTWARE_ENCODERS: &[&str] = &[
    "libx264",
    "libopenh264",
    "libx265",
    "libsvtav1",
    "libaom-av1",
    "librav1e",
    "libvpx-vp9",
    "libvpx",
    "prores_ks",
    "aac",
    "libopus",
    "libmp3lame",
    "flac",
    "libvorbis",
    "gif",
    "libwebp",
    "apng",
    "png",
    "mjpeg",
];

/// names from `ffmpeg -encoders`, whose lines look like " V....D h264_nvenc  NVIDIA ..."
fn parse_encoder_list(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| !line.trim_start().starts_with("------"))
        .skip(1)
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(str::to_owned)
        .collect()
}

/// names from `ffmpeg -hwaccels`, one per line after a heading
fn parse_hwaccels(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.ends_with(':'))
        .map(str::to_owned)
        .collect()
}

fn trial_encode(ffmpeg: &Path, encoder: &str) -> bool {
    let mut cmd = super::process::command(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    let vaapi = encoder.ends_with("_vaapi");
    if vaapi {
        cmd.args(["-vaapi_device", VAAPI_DEVICE]);
    }
    // 256x256 clears every encoder's minimum size
    cmd.args(["-f", "lavfi", "-i", "color=c=black:s=256x256:r=30:d=0.2"]);
    if vaapi {
        cmd.args(["-vf", "format=nv12,hwupload"]);
    } else {
        cmd.args(["-pix_fmt", "nv12"]);
    }
    cmd.args(["-frames:v", "3", "-c:v", encoder, "-f", "null", "-"]);
    run_quietly(cmd)
}

fn trial_device(ffmpeg: &Path, api: &str) -> bool {
    let mut cmd = super::process::command(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    let device = if api == "vaapi" {
        format!("vaapi=hw:{VAAPI_DEVICE}")
    } else {
        api.to_owned()
    };
    cmd.args(["-init_hw_device", &device]);
    cmd.args(["-f", "lavfi", "-i", "nullsrc=s=16x16:d=0.04"]);
    cmd.args(["-frames:v", "1", "-f", "null", "-"]);
    run_quietly(cmd)
}

/// true when the command exits cleanly within the trial time limit
fn run_quietly(mut cmd: Command) -> bool {
    let Ok(mut child) = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    let deadline = Instant::now() + TRIAL_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                // a driver that hangs counts as not working
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::testing;

    #[test]
    fn encoder_lists_are_parsed() {
        let text = "Encoders:\n V..... = Video\n ------\n V....D libx264              libx264 H.264\n \
                    V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)\n \
                    A....D aac                  AAC (Advanced Audio Coding)\n";
        assert_eq!(parse_encoder_list(text), ["libx264", "h264_nvenc", "aac"]);
        let accels = "Hardware acceleration methods:\nvdpau\ncuda\nvaapi\n\n";
        assert_eq!(parse_hwaccels(accels), ["vdpau", "cuda", "vaapi"]);
    }

    #[test]
    fn hardware_is_preferred_only_when_allowed_and_software_is_the_fallback() {
        let support = HardwareSupport {
            encoders: vec![HwEncoder {
                name: "hevc_nvenc".into(),
                codec: VideoCodec::Hevc,
                label: "NVIDIA NVENC".into(),
            }],
            decoders: Vec::new(),
            software_encoders: vec!["libx264".into(), "libx265".into(), "libaom-av1".into()],
            ffmpeg_version: "7.1".into(),
        };
        assert_eq!(
            support.encoder_for(VideoCodec::Hevc, true).as_deref(),
            Some("hevc_nvenc")
        );
        assert_eq!(
            support.encoder_for(VideoCodec::Hevc, false).as_deref(),
            Some("libx265")
        );
        assert_eq!(
            support.encoder_for(VideoCodec::H264, true).as_deref(),
            Some("libx264")
        );
        // svt-av1 is preferred but missing, so aom is used
        assert_eq!(
            support.encoder_for(VideoCodec::Av1, true).as_deref(),
            Some("libaom-av1")
        );
        assert_eq!(support.encoder_for(VideoCodec::Vp9, true), None);
        assert_eq!(
            support.encoder_chain(VideoCodec::Hevc, true),
            ["hevc_nvenc", "libx265"]
        );
        assert_eq!(support.encoder_chain(VideoCodec::Hevc, false), ["libx265"]);
        assert!(support.encoder_chain(VideoCodec::Vp9, true).is_empty());
    }

    #[test]
    fn detection_runs_against_the_real_ffmpeg() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let support = detect_hardware(&tools).unwrap();
        assert_eq!(support.ffmpeg_version, tools.version);
        // every build vellum supports can write gif and png
        assert!(support.software_encoders.iter().any(|e| e == "gif"));
        // whatever passed must really be a hardware encoder name
        for encoder in &support.encoders {
            assert!(encoder.name.contains('_'), "{}", encoder.name);
        }
    }
}
