//! encoding a file with ffmpeg: pick an encoder, build the arguments, report progress, and
//! fall back to software if a hardware encoder fails partway.
//!
//! the output is written to a temporary name next to the target and renamed only when
//! ffmpeg succeeds, so a cancelled or failed encode never leaves half a file behind under
//! the real name. phase 3 (export to a size, gif, converter) builds on this.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::Ffmpeg;
use super::hw::{EncoderInfo, HardwareApi, HardwareSupport, VideoCodec};
use super::process::{ProgressParser, Running};
use crate::control::Control;
use crate::error::{MediaError, MediaResult};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum VideoQuality {
    /// constant quality, lower is better. mapped onto each encoder's own scale.
    Quality { value: u8 },
    /// average bitrate in kilobits per second
    Bitrate { kbps: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VideoEncode {
    pub codec: VideoCodec,
    pub quality: VideoQuality,
    /// scale to this width, keeping the aspect ratio if height is none
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AudioCodec {
    Aac,
    Opus,
    Mp3,
    Flac,
    Pcm,
}

impl AudioCodec {
    pub fn encoder(self) -> &'static str {
        match self {
            Self::Aac => "aac",
            Self::Opus => "libopus",
            Self::Mp3 => "libmp3lame",
            Self::Flac => "flac",
            Self::Pcm => "pcm_s16le",
        }
    }

    fn lossless(self) -> bool {
        matches!(self, Self::Flac | Self::Pcm)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioEncode {
    pub codec: AudioCodec,
    pub kbps: Option<u32>,
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum StreamPlan<T> {
    /// keep the stream as it is, no quality loss
    Copy,
    /// leave it out
    Drop,
    Encode {
        settings: T,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncodeRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    /// seconds from the start of the input
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub video: StreamPlan<VideoEncode>,
    pub audio: StreamPlan<AudioEncode>,
    pub allow_hardware: bool,
    pub strip_metadata: bool,
    /// the input's duration, for progress. none shows indeterminate progress.
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EncodeOutcome {
    pub output: String,
    /// the video encoder used, none when video was copied or dropped
    pub encoder: Option<String>,
    pub hardware: Option<HardwareApi>,
    /// set when a hardware encoder failed and software finished the job, with the reason
    pub fell_back: Option<String>,
}

pub fn encode(
    ffmpeg: &Ffmpeg,
    support: &HardwareSupport,
    request: &EncodeRequest,
    control: &dyn Control,
) -> MediaResult<EncodeOutcome> {
    let encoder = match &request.video {
        StreamPlan::Encode { settings } => Some(
            support
                .pick(settings.codec, request.allow_hardware)
                .ok_or_else(|| missing_encoder(settings.codec))?,
        ),
        _ => None,
    };
    let first = run_once(ffmpeg, request, encoder, control);
    let (encoder, fell_back) = match (first, encoder) {
        (Ok(()), _) => (encoder, None),
        (Err(MediaError::Cancelled), _) => return Err(MediaError::Cancelled),
        (Err(err), Some(hw)) if hw.hardware.is_some() => {
            let software = support
                .software(hw.codec)
                .ok_or_else(|| missing_encoder(hw.codec))?;
            tracing::warn!(encoder = %hw.name, %err, "hardware encode failed, trying software");
            control.message("Hardware encoding failed, using the processor instead");
            control.progress(0.0);
            run_once(ffmpeg, request, Some(software), control)?;
            (Some(software), Some(err.to_string()))
        }
        (Err(err), _) => return Err(err),
    };
    Ok(EncodeOutcome {
        output: request.output.display().to_string(),
        encoder: encoder.map(|e| e.name.clone()),
        hardware: encoder.and_then(|e| e.hardware),
        fell_back,
    })
}

fn missing_encoder(codec: VideoCodec) -> MediaError {
    MediaError::Unsupported(format!(
        "This copy of FFmpeg cannot encode {}. Pick another format or install a fuller FFmpeg build.",
        codec_label(codec)
    ))
}

pub fn codec_label(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "H.264",
        VideoCodec::H265 => "H.265",
        VideoCodec::Av1 => "AV1",
        VideoCodec::Vp9 => "VP9",
    }
}

fn run_once(
    ffmpeg: &Ffmpeg,
    request: &EncodeRequest,
    encoder: Option<&EncoderInfo>,
    control: &dyn Control,
) -> MediaResult<()> {
    let partial = partial_path(&request.output);
    let mut cmd = ffmpeg.command();
    cmd.args(build_args(request, encoder, &partial));
    let span = match (request.start, request.end.or(request.duration)) {
        (start, Some(end)) => Some(end - start.unwrap_or(0.0)),
        _ => None,
    };
    let parser = ProgressParser::new(span);
    let result = (|| {
        let mut running = Running::spawn(cmd)?;
        running.lines(control, |line| {
            if let Some(fraction) = parser.and_then(|p| p.parse(line)) {
                control.progress(fraction);
            }
        })?;
        running.finish()
    })();
    match result {
        Ok(()) => std::fs::rename(&partial, &request.output)
            .map_err(|err| MediaError::io(&request.output, err)),
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

/// `clip.mp4` becomes `clip.vellum-part.mp4`, keeping the extension so ffmpeg still picks
/// the right container
pub fn partial_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match output.extension() {
        Some(ext) => format!("{stem}.vellum-part.{}", ext.to_string_lossy()),
        None => format!("{stem}.vellum-part"),
    };
    output.with_file_name(name)
}

pub fn build_args(
    request: &EncodeRequest,
    encoder: Option<&EncoderInfo>,
    output: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if let Some(encoder) = encoder {
        args.extend(encoder.input_args());
    }
    if let Some(start) = request.start.filter(|s| *s > 0.0) {
        args.extend(["-ss".into(), seconds(start)]);
    }
    if let Some(end) = request.end {
        args.extend(["-to".into(), seconds(end)]);
    }
    args.extend(["-i".into(), request.input.display().to_string()]);
    args.extend([
        "-map".into(),
        "0:v:0?".into(),
        "-map".into(),
        "0:a:0?".into(),
    ]);

    match (&request.video, encoder) {
        (StreamPlan::Drop, _) => args.push("-vn".into()),
        (StreamPlan::Copy, _) => args.extend(["-c:v".into(), "copy".into()]),
        (StreamPlan::Encode { settings }, Some(encoder)) => {
            args.extend(video_args(settings, encoder));
        }
        (StreamPlan::Encode { .. }, None) => args.push("-vn".into()),
    }
    match &request.audio {
        StreamPlan::Drop => args.push("-an".into()),
        StreamPlan::Copy => args.extend(["-c:a".into(), "copy".into()]),
        StreamPlan::Encode { settings } => {
            args.extend(["-c:a".into(), settings.codec.encoder().into()]);
            if let Some(kbps) = settings.kbps.filter(|_| !settings.codec.lossless()) {
                args.extend(["-b:a".into(), format!("{kbps}k")]);
            }
            if let Some(channels) = settings.channels {
                args.extend(["-ac".into(), channels.to_string()]);
            }
            if let Some(rate) = settings.sample_rate {
                args.extend(["-ar".into(), rate.to_string()]);
            }
        }
    }
    if request.strip_metadata {
        args.extend([
            "-map_metadata".into(),
            "-1".into(),
            "-map_chapters".into(),
            "-1".into(),
        ]);
    }
    if output
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("mov"))
    {
        // moves the index to the front so the file plays while it downloads
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.extend([
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        "-y".into(),
        output.display().to_string(),
    ]);
    args
}

fn video_args(settings: &VideoEncode, encoder: &EncoderInfo) -> Vec<String> {
    let mut filters = Vec::new();
    match (settings.width, settings.height) {
        (Some(w), Some(h)) => filters.push(format!("scale={w}:{h}")),
        (Some(w), None) => filters.push(format!("scale={w}:-2")),
        (None, Some(h)) => filters.push(format!("scale=-2:{h}")),
        (None, None) => {}
    }
    if let Some(fps) = settings.fps.filter(|f| f.is_finite() && *f > 0.0) {
        filters.push(format!("fps={fps}"));
    }
    filters.push(encoder.final_filters().to_owned());

    let mut args = vec![
        "-vf".to_owned(),
        filters.join(","),
        "-c:v".to_owned(),
        encoder.name.clone(),
    ];
    match settings.quality {
        VideoQuality::Bitrate { kbps } => {
            args.extend(["-b:v".into(), format!("{kbps}k")]);
            // keeps hardware encoders close to the average instead of spiking
            if encoder.hardware.is_some() {
                args.extend([
                    "-maxrate".into(),
                    format!("{}k", kbps.saturating_mul(3) / 2),
                ]);
                args.extend(["-bufsize".into(), format!("{}k", kbps.saturating_mul(2))]);
            }
        }
        VideoQuality::Quality { value } => args.extend(quality_args(encoder, value)),
    }
    args
}

/// each encoder has its own idea of constant quality. `value` uses the x264 crf scale
/// (0 to 51, 23 is the usual default) and is mapped onto the rest as closely as they allow.
fn quality_args(encoder: &EncoderInfo, value: u8) -> Vec<String> {
    let value = value.min(51);
    let v = value.to_string();
    match encoder.hardware {
        Some(HardwareApi::Nvenc) => vec![
            "-rc".into(),
            "vbr".into(),
            "-cq".into(),
            v,
            "-b:v".into(),
            "0".into(),
        ],
        Some(HardwareApi::QuickSync) => vec!["-global_quality".into(), v],
        Some(HardwareApi::Amf) => vec![
            "-rc".into(),
            "cqp".into(),
            "-qp_i".into(),
            v.clone(),
            "-qp_p".into(),
            v,
        ],
        // videotoolbox takes 1 to 100 where higher is better
        Some(HardwareApi::VideoToolbox) => {
            let q = 100_u32.saturating_sub(u32::from(value) * 2).max(1);
            vec!["-q:v".into(), q.to_string()]
        }
        Some(HardwareApi::Vaapi) => vec!["-qp".into(), v],
        None => match encoder.name.as_str() {
            // vp9 and aom need a zero bitrate for crf to mean constant quality
            "libvpx-vp9" | "libaom-av1" => {
                let crf = (u32::from(value) * 63 / 51).to_string();
                vec!["-crf".into(), crf, "-b:v".into(), "0".into()]
            }
            "libsvtav1" => vec!["-crf".into(), (u32::from(value) * 63 / 51).to_string()],
            "libopenh264" => vec!["-qp".into(), v],
            _ => vec!["-crf".into(), v],
        },
    }
}

fn seconds(value: f64) -> String {
    format!("{value:.6}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(video: StreamPlan<VideoEncode>) -> EncodeRequest {
        EncodeRequest {
            input: "in.mov".into(),
            output: "out.mp4".into(),
            start: Some(1.5),
            end: Some(4.0),
            video,
            audio: StreamPlan::Encode {
                settings: AudioEncode {
                    codec: AudioCodec::Opus,
                    kbps: Some(96),
                    channels: Some(1),
                    sample_rate: None,
                },
            },
            allow_hardware: true,
            strip_metadata: true,
            duration: Some(10.0),
        }
    }

    fn x264() -> EncoderInfo {
        EncoderInfo {
            name: "libx264".into(),
            codec: VideoCodec::H264,
            hardware: None,
        }
    }

    #[test]
    fn arguments_cover_trim_scale_audio_and_metadata() {
        let req = request(StreamPlan::Encode {
            settings: VideoEncode {
                codec: VideoCodec::H264,
                quality: VideoQuality::Quality { value: 23 },
                width: Some(1280),
                height: None,
                fps: Some(30.0),
            },
        });
        let args = build_args(&req, Some(&x264()), Path::new("out.vellum-part.mp4")).join(" ");
        assert!(
            args.contains("-ss 1.500000 -to 4.000000 -i in.mov"),
            "{args}"
        );
        assert!(args.contains("-vf scale=1280:-2,fps=30,format=yuv420p -c:v libx264 -crf 23"));
        assert!(args.contains("-c:a libopus -b:a 96k -ac 1"));
        assert!(args.contains("-map_metadata -1"));
        assert!(args.contains("-movflags +faststart"));
        assert!(args.ends_with("-y out.vellum-part.mp4"));
    }

    #[test]
    fn copy_and_drop_need_no_encoder() {
        let mut req = request(StreamPlan::Copy);
        req.audio = StreamPlan::Drop;
        req.strip_metadata = false;
        let args = build_args(&req, None, Path::new("x.mkv")).join(" ");
        assert!(args.contains("-c:v copy"));
        assert!(args.contains("-an"));
        assert!(!args.contains("faststart"));
        assert!(!args.contains("map_metadata"));
    }

    #[test]
    fn vaapi_opens_its_device_and_uploads_frames() {
        let vaapi = EncoderInfo {
            name: "h264_vaapi".into(),
            codec: VideoCodec::H264,
            hardware: Some(HardwareApi::Vaapi),
        };
        let req = request(StreamPlan::Encode {
            settings: VideoEncode {
                codec: VideoCodec::H264,
                quality: VideoQuality::Bitrate { kbps: 2000 },
                width: None,
                height: None,
                fps: None,
            },
        });
        let args = build_args(&req, Some(&vaapi), Path::new("o.mp4")).join(" ");
        assert!(args.starts_with("-vaapi_device"));
        assert!(args.contains("format=nv12,hwupload"));
        assert!(args.contains("-b:v 2000k -maxrate 3000k -bufsize 4000k"));
    }

    #[test]
    fn partial_files_keep_their_extension() {
        assert_eq!(
            partial_path(Path::new("/a/clip.mp4")),
            PathBuf::from("/a/clip.vellum-part.mp4")
        );
        assert_eq!(
            partial_path(Path::new("noext")),
            PathBuf::from("noext.vellum-part")
        );
    }
}
