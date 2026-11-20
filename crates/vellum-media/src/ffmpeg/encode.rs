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
    /// ffmpeg filters that run before scaling, eg a crop or a rotation
    #[serde(default)]
    pub filters: Vec<String>,
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
    pub passes: Passes,
    /// output options added after the codecs, eg `-tag:v hvc1` or `-loop 0`
    pub extra_args: Vec<String>,
}

impl EncodeRequest {
    /// copies every stream from `input` to `output`, a starting point to change from
    pub fn new(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            start: None,
            end: None,
            video: StreamPlan::Copy,
            audio: StreamPlan::Copy,
            allow_hardware: false,
            strip_metadata: false,
            duration: None,
            passes: Passes::One,
            extra_args: Vec::new(),
        }
    }
}

/// two pass encoding reads the whole video once to learn where the bits are needed, then
/// spends them there, which is how a file lands on an exact size. only software encoders
/// that support it get two passes, the rest quietly use one.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Passes {
    #[default]
    One,
    /// both passes. `log` names the statistics files, in a folder the caller cleans up.
    Two { log: PathBuf },
    /// only the second pass, reusing statistics from an earlier `Two` with the same log.
    /// used to retry at a corrected bitrate without reading the video again.
    Second { log: PathBuf },
}

/// software encoders that can do two passes from the ffmpeg command line
pub fn supports_two_pass(encoder: &str) -> bool {
    matches!(
        encoder,
        "libx264" | "libx265" | "libvpx-vp9" | "libvpx" | "libaom-av1"
    )
}

fn pass_args(encoder: &str, pass: u8, log_name: &str) -> Vec<String> {
    if encoder == "libx265" {
        return vec![
            "-x265-params".into(),
            format!("pass={pass}:stats={log_name}:log-level=error"),
        ];
    }
    vec![
        "-pass".into(),
        pass.to_string(),
        "-passlogfile".into(),
        log_name.to_owned(),
    ]
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
    let wants_passes = !matches!(request.passes, Passes::One);
    if wants_passes
        && let Some(encoder) = encoder
        && encoder.hardware.is_none()
        && supports_two_pass(&encoder.name)
    {
        run_passes(ffmpeg, request, encoder, control)?;
        return Ok(EncodeOutcome {
            output: request.output.display().to_string(),
            encoder: Some(encoder.name.clone()),
            hardware: None,
            fell_back: None,
        });
    }
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

/// runs pass 1 (unless only the second is asked for) and pass 2. the statistics files are
/// named relative to their folder, which becomes ffmpeg's working folder, because x265
/// takes its file name inside a colon separated list where a windows drive letter breaks.
fn run_passes(
    ffmpeg: &Ffmpeg,
    request: &EncodeRequest,
    encoder: &EncoderInfo,
    control: &dyn Control,
) -> MediaResult<()> {
    let (log, first_pass) = match &request.passes {
        Passes::Two { log } => (log, true),
        Passes::Second { log } => (log, false),
        Passes::One => return run_once(ffmpeg, request, Some(encoder), control),
    };
    let dir = log
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let log_name = log
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "pass".to_owned());
    let mut request = request.clone();
    request.input =
        std::path::absolute(&request.input).map_err(|err| MediaError::io(&request.input, err))?;
    request.output =
        std::path::absolute(&request.output).map_err(|err| MediaError::io(&request.output, err))?;

    if first_pass {
        control.message("Reading the video (pass 1 of 2)");
        let mut first = request.clone();
        first.audio = StreamPlan::Drop;
        first.strip_metadata = false;
        let mut args = build_args(&first, Some(encoder), Path::new("-"));
        // the output is thrown away, only the statistics matter
        let out = args.len() - 1;
        args.splice(out.., ["-f".to_owned(), "null".to_owned(), "-".to_owned()]);
        insert_before_progress(&mut args, pass_args(&encoder.name, 1, &log_name));
        let span = crate::control::Span {
            inner: control,
            start: 0.0,
            end: 0.45,
        };
        run_args(ffmpeg, &request, args, Some(&dir), &span)?;
        control.message("Encoding (pass 2 of 2)");
    }
    let partial = partial_path(&request.output);
    let mut args = build_args(&request, Some(encoder), &partial);
    insert_before_progress(&mut args, pass_args(&encoder.name, 2, &log_name));
    let start = if first_pass { 0.45 } else { 0.0 };
    let span = crate::control::Span {
        inner: control,
        start,
        end: 1.0,
    };
    match run_args(ffmpeg, &request, args, Some(&dir), &span) {
        Ok(()) => std::fs::rename(&partial, &request.output)
            .map_err(|err| MediaError::io(&request.output, err)),
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

fn insert_before_progress(args: &mut Vec<String>, extra: Vec<String>) {
    let at = args
        .iter()
        .position(|a| a == "-progress")
        .unwrap_or(args.len());
    args.splice(at..at, extra);
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
    let args = build_args(request, encoder, &partial);
    match run_args(ffmpeg, request, args, None, control) {
        Ok(()) => std::fs::rename(&partial, &request.output)
            .map_err(|err| MediaError::io(&request.output, err)),
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

/// runs ffmpeg with `args`, reporting progress across the trimmed length of the input
fn run_args(
    ffmpeg: &Ffmpeg,
    request: &EncodeRequest,
    args: Vec<String>,
    dir: Option<&Path>,
    control: &dyn Control,
) -> MediaResult<()> {
    let mut cmd = ffmpeg.command();
    cmd.args(args);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let span = match (request.start, request.end.or(request.duration)) {
        (start, Some(end)) => Some(end - start.unwrap_or(0.0)),
        _ => None,
    };
    let parser = ProgressParser::new(span);
    let mut running = Running::spawn(cmd)?;
    running.lines(control, |line| {
        if let Some(fraction) = parser.and_then(|p| p.parse(line)) {
            control.progress(fraction);
        }
    })?;
    running.finish()
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
    args.extend(request.extra_args.iter().cloned());
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
    let mut filters = settings.filters.clone();
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
            passes: Passes::One,
            extra_args: Vec::new(),
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
                filters: Vec::new(),
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
                filters: Vec::new(),
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
