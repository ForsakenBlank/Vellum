//! the universal converter: any format to any format where that makes sense, plus the
//! one click jobs people use converters for (extract the sound, pull out frames, turn
//! photos into a slideshow, make an icon).
//!
//! changing only the container never re-encodes: if the streams already suit the new
//! container and nothing else changes, they are copied, which is instant and lossless.

use std::path::{Path, PathBuf};

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, RgbaImage};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_core::batch_rename::RenamePattern;
use vellum_media::ffmpeg::encode::{
    self, AudioCodec, AudioEncode, EncodeRequest, StreamPlan, VideoEncode, VideoQuality,
};
use vellum_media::ffmpeg::hw::VideoCodec;
use vellum_media::picture::{self, DecodeOptions, PictureFormat};
use vellum_media::probe::{MediaInfo, MediaKind};
use vellum_media::{Control, MediaError, MediaResult};

use crate::anim::{self, AnimFormat, AnimOptions, AnimSource, frames};
use crate::size::image::{Saved, jpeg};
use crate::{Env, ToolResult, naming, num, output_args, run_ffmpeg};

/// what to convert to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConvertTarget {
    Mp4,
    Webm,
    Mkv,
    Mov,
    Mp3,
    M4a,
    Ogg,
    Wav,
    Flac,
    Jpg,
    Png,
    Webp,
    Avif,
    Bmp,
    Tiff,
    Ico,
    Gif,
    /// every frame (or one every so often) as pictures in a folder
    Frames,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Video,
    Audio,
    Image,
    Gif,
    Folder,
}

impl ConvertTarget {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
            Self::Mkv => "mkv",
            Self::Mov => "mov",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Ogg => "ogg",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Jpg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Avif => "avif",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Ico => "ico",
            Self::Gif => "gif",
            Self::Frames => "",
        }
    }

    fn group(self) -> Group {
        match self {
            Self::Mp4 | Self::Webm | Self::Mkv | Self::Mov => Group::Video,
            Self::Mp3 | Self::M4a | Self::Ogg | Self::Wav | Self::Flac => Group::Audio,
            Self::Jpg
            | Self::Png
            | Self::Webp
            | Self::Avif
            | Self::Bmp
            | Self::Tiff
            | Self::Ico => Group::Image,
            Self::Gif => Group::Gif,
            Self::Frames => Group::Folder,
        }
    }

    /// the stream codecs this container can hold without re-encoding
    fn copies(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Self::Mp4 => (
                &["h264", "hevc", "av1", "vp9", "mpeg4"],
                &["aac", "mp3", "opus", "ac3", "eac3", "alac", "flac"],
            ),
            Self::Mov => (
                &["h264", "hevc", "prores", "mjpeg", "mpeg4"],
                &["aac", "alac", "mp3", "pcm_s16le", "pcm_s24le"],
            ),
            Self::Webm => (&["vp8", "vp9", "av1"], &["opus", "vorbis"]),
            Self::Mkv => (
                &[
                    "h264", "hevc", "av1", "vp8", "vp9", "mpeg4", "prores", "mjpeg", "ffv1",
                ],
                &[
                    "aac",
                    "mp3",
                    "opus",
                    "vorbis",
                    "flac",
                    "ac3",
                    "eac3",
                    "alac",
                    "pcm_s16le",
                    "pcm_s24le",
                    "dts",
                ],
            ),
            Self::Mp3 => (&[], &["mp3"]),
            Self::M4a => (&[], &["aac", "alac"]),
            Self::Ogg => (&[], &["opus", "vorbis"]),
            Self::Flac => (&[], &["flac"]),
            Self::Wav => (&[], &["pcm_s16le", "pcm_s24le", "pcm_f32le"]),
            _ => (&[], &[]),
        }
    }
}

/// true for the formats that hold only sound
pub(crate) fn is_audio(target: ConvertTarget) -> bool {
    target.group() == Group::Audio
}

/// the formats a file of `kind` can become, most useful first
pub fn targets_for(kind: MediaKind) -> Vec<ConvertTarget> {
    use ConvertTarget::*;
    match kind {
        MediaKind::Video => vec![
            Mp4, Webm, Mkv, Mov, Gif, Webp, Mp3, M4a, Ogg, Wav, Flac, Frames, Jpg, Png,
        ],
        MediaKind::Audio => vec![Mp3, M4a, Ogg, Wav, Flac],
        MediaKind::Animation => vec![Mp4, Webm, Webp, Avif, Png, Jpg, Frames],
        MediaKind::Image | MediaKind::Vector => {
            vec![Jpg, Png, Webp, Avif, Gif, Bmp, Tiff, Ico, Mp4]
        }
        MediaKind::Unknown => Vec::new(),
    }
}

/// what the converter does with the list
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConvertAction {
    /// each file into another format
    #[default]
    Convert,
    /// every file joined into one, in list order
    Merge,
    /// each file cut into parts of the same length
    Split,
    /// new names for the files themselves. this runs as a command, not a job.
    Rename,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ConvertOptions {
    pub action: ConvertAction,
    /// none picks the first target that fits each file
    pub target: Option<ConvertTarget>,
    /// 0 to 100
    pub quality: u32,
    /// shrinks to fit inside these, keeping the shape. drawings (svg) are drawn at this
    /// width, larger or smaller.
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    /// clockwise quarter turns: 0, 90, 180 or 270
    pub rotate: u32,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub mute: bool,
    pub speed: f64,
    /// for mp4, mkv and mov. webm always uses vp9.
    pub codec: VideoCodec,
    /// seconds between extracted frames, 0 for every frame
    pub frame_interval: f64,
    /// jpg or png for extracted frames
    pub frames_as_jpg: bool,
    pub ico_sizes: Vec<u32>,
    /// how long each picture shows in a slideshow
    pub slide_seconds: f64,
    /// the length of each part when splitting
    pub split_seconds: f64,
    /// re-encode so every part is exactly `split_seconds` long. without it the streams are
    /// copied and each cut lands on the next keyframe, which is instant.
    pub split_exact: bool,
    pub rename: RenamePattern,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            action: ConvertAction::Convert,
            target: None,
            quality: 85,
            max_width: None,
            max_height: None,
            rotate: 0,
            flip_horizontal: false,
            flip_vertical: false,
            start: None,
            end: None,
            mute: false,
            speed: 1.0,
            codec: VideoCodec::H264,
            frame_interval: 0.0,
            frames_as_jpg: false,
            ico_sizes: vec![16, 24, 32, 48, 64, 128, 256],
            slide_seconds: 3.0,
            split_seconds: 60.0,
            split_exact: false,
            rename: RenamePattern::default(),
        }
    }
}

impl ConvertOptions {
    fn speed(&self) -> f64 {
        if self.speed.is_finite() && self.speed > 0.0 {
            self.speed.clamp(0.25, 4.0)
        } else {
            1.0
        }
    }

    fn trimmed(&self) -> bool {
        self.start.is_some_and(|s| s > 0.0) || self.end.is_some()
    }

    /// anything that changes the picture, which rules out copying it
    fn edits_picture(&self) -> bool {
        self.max_width.is_some()
            || self.max_height.is_some()
            || !self.rotate.is_multiple_of(360)
            || self.flip_horizontal
            || self.flip_vertical
            || (self.speed() - 1.0).abs() > 1e-6
    }

    fn quality(&self) -> u32 {
        self.quality.min(100)
    }
}

/// whether `info` can go into `target` by copying its streams, which loses nothing
pub fn can_remux(info: &MediaInfo, target: ConvertTarget, options: &ConvertOptions) -> bool {
    if options.trimmed() || options.edits_picture() {
        return false;
    }
    let (video_ok, audio_ok) = target.copies();
    let video = match (&info.video, target.group()) {
        (Some(v), Group::Video) => video_ok.contains(&v.codec.as_str()),
        (None, Group::Video) => false,
        (_, Group::Audio) => true,
        _ => return false,
    };
    let audio = options.mute
        || info.audio.is_empty() && target.group() == Group::Video
        || info
            .audio
            .iter()
            .all(|a| audio_ok.contains(&a.codec.as_str()));
    let has_audio = !info.audio.is_empty();
    video && audio && (target.group() != Group::Audio || has_audio)
}

/// converts one file
pub fn convert(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let available = targets_for(info.kind);
    let target = match options.target {
        Some(t) if available.contains(&t) => t,
        Some(t) => {
            return Err(MediaError::Unsupported(format!(
                "{} cannot be turned into {}. Pick another format for it.",
                info.name,
                t.extension().to_uppercase()
            )));
        }
        None => *available.first().ok_or_else(|| {
            MediaError::Unsupported(format!("Vellum cannot convert {}.", info.name))
        })?,
    };
    match (info.kind, target.group()) {
        (MediaKind::Video | MediaKind::Audio, Group::Video | Group::Audio) => {
            av(env, input, info, target, options, control)
        }
        (MediaKind::Animation, Group::Video) => {
            gif_to_video(env, input, info, target, options, control)
        }
        (MediaKind::Video, Group::Gif) | (MediaKind::Animation, Group::Image)
            if matches!(target, ConvertTarget::Webp | ConvertTarget::Avif)
                || target == ConvertTarget::Gif =>
        {
            animate(env, input, info, target, options, control)
        }
        (MediaKind::Video | MediaKind::Animation, Group::Folder) => {
            extract_frames(env, input, info, options, control)
        }
        (MediaKind::Video, Group::Image) => video_still(env, input, info, target, options, control),
        (
            MediaKind::Image | MediaKind::Vector | MediaKind::Animation,
            Group::Image | Group::Gif,
        ) => still(env, input, info, target, options, control),
        (MediaKind::Image | MediaKind::Vector, Group::Video) => {
            slideshow(env, &[input.to_path_buf()], target, options, control)
        }
        _ => Err(MediaError::Unsupported(format!(
            "{} cannot be turned into {}.",
            info.name,
            target.extension().to_uppercase()
        ))),
    }
}

/// the filters for rotating, flipping and fitting a picture inside the size limits
fn picture_filters(options: &ConvertOptions, even: bool) -> Vec<String> {
    let mut filters = Vec::new();
    match options.rotate % 360 {
        90 => filters.push("transpose=clock".to_owned()),
        180 => filters.push("hflip,vflip".to_owned()),
        270 => filters.push("transpose=cclock".to_owned()),
        _ => {}
    }
    if options.flip_horizontal {
        filters.push("hflip".to_owned());
    }
    if options.flip_vertical {
        filters.push("vflip".to_owned());
    }
    if options.max_width.is_some() || options.max_height.is_some() {
        let w = options.max_width.map_or("iw".to_owned(), |w| w.to_string());
        let h = options
            .max_height
            .map_or("ih".to_owned(), |h| h.to_string());
        // shrink only, keeping the shape
        filters.push(format!(
            "scale='min(iw,{w})':'min(ih,{h})':force_original_aspect_ratio=decrease"
        ));
    }
    if even {
        filters.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".to_owned());
    }
    let speed = options.speed();
    if (speed - 1.0).abs() > 1e-6 {
        filters.push(format!("setpts=PTS/{}", num(speed)));
    }
    filters
}

/// atempo only takes 0.5 to 2, so bigger changes chain it
fn tempo_filter(speed: f64) -> Option<String> {
    if (speed - 1.0).abs() < 1e-6 {
        return None;
    }
    let mut parts = Vec::new();
    let mut left = speed;
    while left > 2.0 {
        parts.push("atempo=2".to_owned());
        left /= 2.0;
    }
    while left < 0.5 {
        parts.push("atempo=0.5".to_owned());
        left /= 0.5;
    }
    parts.push(format!("atempo={}", num(left)));
    Some(parts.join(","))
}

pub(crate) fn audio_settings(target: ConvertTarget, options: &ConvertOptions) -> AudioEncode {
    let q = options.quality();
    let step = |rates: [u32; 4]| match q {
        90.. => rates[0],
        70..=89 => rates[1],
        45..=69 => rates[2],
        _ => rates[3],
    };
    let (codec, kbps) = match target {
        ConvertTarget::Mp3 => (AudioCodec::Mp3, Some(step([320, 192, 128, 96]))),
        ConvertTarget::Ogg | ConvertTarget::Webm => {
            (AudioCodec::Opus, Some(step([192, 128, 96, 64])))
        }
        ConvertTarget::Wav => (AudioCodec::Pcm, None),
        ConvertTarget::Flac => (AudioCodec::Flac, None),
        _ => (AudioCodec::Aac, Some(step([256, 192, 128, 96]))),
    };
    AudioEncode {
        codec,
        kbps,
        channels: None,
        sample_rate: None,
    }
}

/// video and audio between containers and codecs, remuxing when it can
fn av(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    target: ConvertTarget,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let ffmpeg = env.ffmpeg()?;
    if target.group() == Group::Audio && info.audio.is_empty() {
        return Err(MediaError::Unsupported(format!(
            "{} has no sound to save.",
            info.name
        )));
    }
    let output = naming::output_for(input, env.output_folder, None, target.extension());
    let remux = can_remux(info, target, options);
    let mut result = ToolResult::file(input, output.path());

    if remux {
        control.message("Copying the streams, nothing is re-encoded");
        let partial = encode::partial_path(output.path());
        let mut args: Vec<String> = vec!["-i".into(), input.display().to_string()];
        if target.group() == Group::Video {
            args.extend(["-map".into(), "0:v:0".into()]);
            if !options.mute {
                args.extend(["-map".into(), "0:a?".into()]);
            }
            // matroska keeps subtitles as they are, the others would need converting
            if target == ConvertTarget::Mkv {
                args.extend(["-map".into(), "0:s?".into()]);
            }
        } else {
            args.extend(["-map".into(), "0:a:0".into(), "-vn".into()]);
        }
        args.extend(["-c".into(), "copy".into()]);
        if matches!(
            target,
            ConvertTarget::Mp4 | ConvertTarget::Mov | ConvertTarget::M4a
        ) {
            args.extend(["-movflags".into(), "+faststart".into()]);
        }
        if target == ConvertTarget::Mp4 && info.video.as_ref().is_some_and(|v| v.codec == "hevc") {
            args.extend(["-tag:v".into(), "hvc1".into()]);
        }
        args.extend(output_args(&partial));
        if let Err(err) = run_ffmpeg(ffmpeg, &args, info.duration_secs, control) {
            let _ = std::fs::remove_file(&partial);
            return Err(err);
        }
        crate::finish_partial(&partial, output.path())?;
        result.bytes = std::fs::metadata(output.path()).ok().map(|m| m.len());
        result
            .changes
            .push("Copied without re-encoding, so nothing was lost.".to_owned());
        return Ok(result);
    }

    let mut request = EncodeRequest::new(input, output.path());
    request.start = options.start.filter(|s| *s > 0.0);
    request.end = options.end;
    request.duration = info.duration_secs;
    request.allow_hardware = env.allow_hardware;
    let speed = options.speed();
    request.audio = if options.mute || info.audio.is_empty() {
        StreamPlan::Drop
    } else {
        StreamPlan::Encode {
            settings: audio_settings(target, options),
        }
    };
    if target.group() == Group::Video {
        let codec = if target == ConvertTarget::Webm {
            VideoCodec::Vp9
        } else {
            options.codec
        };
        request.video = StreamPlan::Encode {
            settings: VideoEncode {
                codec,
                // quality 100 is crf 16, 0 is crf 40
                quality: VideoQuality::Quality {
                    value: (40 - options.quality() * 24 / 100) as u8,
                },
                width: None,
                height: None,
                fps: None,
                filters: picture_filters(options, true),
            },
        };
        if codec == VideoCodec::H265 && target != ConvertTarget::Mkv {
            request.extra_args.extend(["-tag:v".into(), "hvc1".into()]);
        }
    } else {
        request.video = StreamPlan::Drop;
    }
    if let Some(tempo) = tempo_filter(speed).filter(|_| !matches!(request.audio, StreamPlan::Drop))
    {
        request.extra_args.extend(["-af".into(), tempo]);
    }
    let outcome = encode::encode(ffmpeg, env.hardware, &request, control)?;
    result.bytes = std::fs::metadata(output.path()).ok().map(|m| m.len());
    if let Some(reason) = outcome.fell_back {
        result.notes.push(format!(
            "The hardware encoder failed, so the processor was used: {reason}"
        ));
    }
    Ok(result)
}

/// a gif (or other animation) as a video file
fn gif_to_video(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    target: ConvertTarget,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let ffmpeg = env.ffmpeg()?;
    let output = naming::output_for(input, env.output_folder, None, target.extension());
    let codec = if target == ConvertTarget::Webm {
        VideoCodec::Vp9
    } else {
        options.codec
    };
    let mut request = EncodeRequest::new(input, output.path());
    request.duration = info.duration_secs;
    request.allow_hardware = env.allow_hardware;
    request.audio = StreamPlan::Drop;
    request.video = StreamPlan::Encode {
        settings: VideoEncode {
            codec,
            quality: VideoQuality::Quality {
                value: (40 - options.quality() * 24 / 100) as u8,
            },
            width: None,
            height: None,
            fps: None,
            filters: picture_filters(options, true),
        },
    };
    encode::encode(ffmpeg, env.hardware, &request, control)?;
    Ok(ToolResult::file(input, output.path()))
}

/// video or gif into an animation format through the gif engine
fn animate(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    target: ConvertTarget,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let source = AnimSource::of(info)?;
    let format = match target {
        ConvertTarget::Webp => AnimFormat::Webp,
        ConvertTarget::Avif => AnimFormat::Avif,
        _ => AnimFormat::Gif,
    };
    let from_video = info.kind == MediaKind::Video;
    let anim_options = AnimOptions {
        start: options.start,
        end: options.end,
        // a whole video as a full size gif would be enormous, a gif keeps its own size
        width: options
            .max_width
            .or(if from_video { Some(480) } else { None }),
        fps: if from_video { Some(15.0) } else { None },
        speed: options.speed(),
        format,
        quality: options.quality(),
        ..AnimOptions::default()
    };
    let output = naming::output_for(input, env.output_folder, None, format.extension());
    anim::render(env, input, &source, &anim_options, output.path(), control)?;
    Ok(ToolResult::file(input, output.path()))
}

/// every frame, or one every `frame_interval` seconds, as numbered pictures in a folder
fn extract_frames(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let folder = naming::folder_for(input, env.output_folder, "frames");
    std::fs::create_dir_all(folder.path()).map_err(|err| MediaError::io(folder.path(), err))?;
    let count = if info.kind == MediaKind::Animation
        && options.frame_interval <= 0.0
        && !options.frames_as_jpg
    {
        frames::split(input, folder.path(), control)?
    } else {
        let ffmpeg = env.ffmpeg()?;
        let mut args: Vec<String> = Vec::new();
        if let Some(start) = options.start.filter(|s| *s > 0.0) {
            args.extend(["-ss".into(), num(start)]);
        }
        if let Some(end) = options.end {
            args.extend(["-to".into(), num(end)]);
        }
        args.extend(["-i".into(), input.display().to_string()]);
        let mut filters = picture_filters(options, false);
        if options.frame_interval > 0.0 {
            filters.insert(
                0,
                format!("fps=1/{}", num(options.frame_interval.max(0.01))),
            );
        }
        if !filters.is_empty() {
            args.extend(["-vf".into(), filters.join(",")]);
        }
        let ext = if options.frames_as_jpg { "jpg" } else { "png" };
        if options.frames_as_jpg {
            args.extend(["-q:v".into(), "2".into()]);
        }
        args.extend(["-fps_mode".into(), "passthrough".into()]);
        let pattern = folder.path().join(format!("frame %05d.{ext}"));
        args.extend(output_args(&pattern));
        if let Err(err) = run_ffmpeg(ffmpeg, &args, info.duration_secs, control) {
            let _ = std::fs::remove_dir_all(folder.path());
            return Err(err);
        }
        std::fs::read_dir(folder.path()).map_or(0, Iterator::count)
    };
    let mut result = ToolResult::file(input, folder.path());
    result.folder = true;
    result.bytes = None;
    result.changes.push(format!("Saved {count} frames."));
    Ok(result)
}

/// one frame of a video as a picture, at the start point
fn video_still(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    target: ConvertTarget,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let ffmpeg = env.ffmpeg()?;
    let (Some(width), Some(height)) = (info.width, info.height) else {
        return Err(MediaError::Unsupported(format!(
            "{} has no picture.",
            info.name
        )));
    };
    let at = options.start.unwrap_or(0.0);
    let frame = vellum_media::seek::grab(ffmpeg, input, at, true, (width, height))?;
    let image = edit_still(frame, options);
    let output = naming::output_for(input, env.output_folder, None, target.extension());
    write_still(env, &image, target, options, output.path(), control)?;
    let mut result = ToolResult::file(input, output.path());
    result.changes.push(format!(
        "The frame at {} seconds.",
        num((at * 100.0).round() / 100.0)
    ));
    Ok(result)
}

/// rotate, flip and shrink a picture the way the options say
fn edit_still(image: RgbaImage, options: &ConvertOptions) -> RgbaImage {
    let mut image = DynamicImage::ImageRgba8(image);
    image = match options.rotate % 360 {
        90 => image.rotate90(),
        180 => image.rotate180(),
        270 => image.rotate270(),
        _ => image,
    };
    if options.flip_horizontal {
        image = image.fliph();
    }
    if options.flip_vertical {
        image = image.flipv();
    }
    let (w, h) = (image.width(), image.height());
    let max_w = options.max_width.unwrap_or(w);
    let max_h = options.max_height.unwrap_or(h);
    if w > max_w || h > max_h {
        image = image.resize(max_w.max(1), max_h.max(1), FilterType::Lanczos3);
    }
    image.to_rgba8()
}

/// pictures between formats, including svg at any size, camera raw, heic and psd
fn still(
    env: &Env,
    input: &Path,
    info: &MediaInfo,
    target: ConvertTarget,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let format = info.image.as_ref().map(|i| i.format);
    let decode = DecodeOptions {
        // drawings are drawn at the size asked for, which can be bigger than they say
        size: match format {
            Some(PictureFormat::Svg) => options.max_width.or(options.max_height),
            _ => None,
        },
        all_frames: target == ConvertTarget::Gif,
    };
    let picture = picture::decode(input, decode, env.ffmpeg)?;
    control.progress(0.3);
    let output = naming::output_for(input, env.output_folder, None, target.extension());
    let mut result_notes = Vec::new();
    if target == ConvertTarget::Gif {
        let images: Vec<RgbaImage> = picture
            .frames
            .iter()
            .map(|f| edit_still(f.image.clone(), options))
            .collect();
        let list: Vec<(&RgbaImage, u32)> = images
            .iter()
            .zip(&picture.frames)
            .map(|(image, frame)| (image, frame.delay_ms.max(20)))
            .collect();
        let (w, h) = images[0].dimensions();
        let data = frames::encode_frames(&list, w, h, 0, control)?;
        crate::write_atomic(output.path(), &data)?;
    } else {
        if picture.frames.len() > 1 {
            result_notes
                .push("Only the first frame was kept, since this format does not move.".to_owned());
        }
        let first = picture.first().clone();
        let image = if format == Some(PictureFormat::Svg) {
            // already drawn at the asked size, only turn and flip it
            edit_still(
                first,
                &ConvertOptions {
                    max_width: None,
                    max_height: None,
                    ..options.clone()
                },
            )
        } else {
            edit_still(first, options)
        };
        write_still(env, &image, target, options, output.path(), control)?;
    }
    let mut result = ToolResult::file(input, output.path());
    result.notes = result_notes;
    control.progress(1.0);
    Ok(result)
}

/// writes a picture in the target format
fn write_still(
    env: &Env,
    image: &RgbaImage,
    target: ConvertTarget,
    options: &ConvertOptions,
    output: &Path,
    control: &dyn Control,
) -> MediaResult<()> {
    let fail = |err: image::ImageError| {
        MediaError::Unsupported(format!("The picture could not be saved: {err}"))
    };
    let quality = options.quality().clamp(1, 100) as u8;
    let data: Vec<u8> = match target {
        ConvertTarget::Jpg => {
            let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();
            jpeg(&rgb, quality)?
        }
        ConvertTarget::Ico => ico(image, &options.ico_sizes)?,
        ConvertTarget::Webp | ConvertTarget::Avif => {
            let ffmpeg = env.ffmpeg()?;
            let temp =
                tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
            let source = temp.path().join("source.png");
            let prepared = if target == ConvertTarget::Avif {
                let (w, h) = (image.width() & !1, image.height() & !1);
                image::imageops::crop_imm(image, 0, 0, w.max(2), h.max(2)).to_image()
            } else {
                image.clone()
            };
            prepared.save(&source).map_err(fail)?;
            let saved = if target == ConvertTarget::Avif {
                Saved::Avif
            } else {
                Saved::Webp
            };
            let out = temp.path().join(format!("out.{}", saved.extension()));
            let mut args: Vec<String> = vec!["-i".into(), source.display().to_string()];
            if saved == Saved::Webp {
                args.extend([
                    "-c:v".into(),
                    "libwebp".into(),
                    "-quality".into(),
                    quality.to_string(),
                    "-f".into(),
                    "webp".into(),
                ]);
            } else {
                let encoder = crate::avif_encoder(env)?.to_owned();
                let crf = 63 - u32::from(quality) * 6 / 10;
                args.extend([
                    "-c:v".into(),
                    encoder.clone(),
                    "-crf".into(),
                    crf.to_string(),
                ]);
                if encoder == "libaom-av1" {
                    args.extend([
                        "-still-picture".into(),
                        "1".into(),
                        "-cpu-used".into(),
                        "6".into(),
                    ]);
                }
                args.extend([
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-f".into(),
                    "avif".into(),
                ]);
            }
            args.extend(output_args(&out));
            run_ffmpeg(ffmpeg, &args, None, control)?;
            std::fs::read(&out).map_err(|err| MediaError::io(&out, err))?
        }
        ConvertTarget::Png | ConvertTarget::Bmp | ConvertTarget::Tiff => {
            let format = match target {
                ConvertTarget::Png => image::ImageFormat::Png,
                ConvertTarget::Bmp => image::ImageFormat::Bmp,
                _ => image::ImageFormat::Tiff,
            };
            let mut out = std::io::Cursor::new(Vec::new());
            let opaque = image.pixels().all(|p| p[3] == 255);
            let dynamic = DynamicImage::ImageRgba8(image.clone());
            let dynamic = if opaque {
                DynamicImage::ImageRgb8(dynamic.to_rgb8())
            } else {
                dynamic
            };
            dynamic.write_to(&mut out, format).map_err(fail)?;
            out.into_inner()
        }
        _ => {
            return Err(MediaError::Unsupported(format!(
                "{} is not a picture format.",
                target.extension().to_uppercase()
            )));
        }
    };
    crate::write_atomic(output, &data)
}

/// an icon with every size in `sizes`, square, transparent around pictures that are not
pub fn ico(image: &RgbaImage, sizes: &[u32]) -> MediaResult<Vec<u8>> {
    let mut sizes: Vec<u32> = sizes
        .iter()
        .copied()
        .filter(|s| (1..=256).contains(s))
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    if sizes.is_empty() {
        sizes.push(256);
    }
    let side = image.width().max(image.height());
    let mut square = RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut square,
        image,
        i64::from((side - image.width()) / 2),
        i64::from((side - image.height()) / 2),
    );
    let fail = |err: image::ImageError| {
        MediaError::Unsupported(format!("The icon could not be made: {err}"))
    };
    let mut frames = Vec::new();
    for size in sizes {
        let resized = image::imageops::resize(&square, size, size, FilterType::Lanczos3);
        frames.push(
            IcoFrame::as_png(resized.as_raw(), size, size, ExtendedColorType::Rgba8)
                .map_err(fail)?,
        );
    }
    let mut out = Vec::new();
    IcoEncoder::new(&mut out)
        .encode_images(&frames)
        .map_err(fail)?;
    Ok(out)
}

/// pictures as a video, each shown for `slide_seconds`, letterboxed to the first one's shape
pub fn slideshow(
    env: &Env,
    inputs: &[PathBuf],
    target: ConvertTarget,
    options: &ConvertOptions,
    control: &dyn Control,
) -> MediaResult<ToolResult> {
    let ffmpeg = env.ffmpeg()?;
    let first = inputs.first().ok_or_else(|| {
        MediaError::Unsupported("A slideshow needs at least one picture.".to_owned())
    })?;
    let temp = tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
    let seconds = if options.slide_seconds.is_finite() {
        options.slide_seconds.clamp(0.1, 600.0)
    } else {
        3.0
    };
    let mut size: Option<(u32, u32)> = None;
    let mut list = String::from("ffconcat version 1.0\n");
    for (i, path) in inputs.iter().enumerate() {
        control.checkpoint()?;
        let picture = picture::decode(path, DecodeOptions::preview(1920), env.ffmpeg)?;
        let image = edit_still(picture.first().clone(), options);
        let (w, h) = *size.get_or_insert_with(|| {
            let (w, h) = image.dimensions();
            ((w.max(2)) & !1, (h.max(2)) & !1)
        });
        // letterbox onto black so every slide has the same size
        let fitted = DynamicImage::ImageRgba8(image)
            .resize(w, h, FilterType::Lanczos3)
            .to_rgba8();
        let mut frame = RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
        image::imageops::overlay(
            &mut frame,
            &fitted,
            i64::from((w - fitted.width()) / 2),
            i64::from((h - fitted.height()) / 2),
        );
        let file = temp.path().join(format!("slide{i}.png"));
        frame.save(&file).map_err(|err| {
            MediaError::Unsupported(format!("A slide could not be prepared: {err}"))
        })?;
        list.push_str(&format!("file 'slide{i}.png'\nduration {}\n", num(seconds)));
        control.progress(0.4 * (i + 1) as f32 / inputs.len() as f32);
    }
    // the last picture is listed twice, or the concat reader drops its duration
    list.push_str(&format!("file 'slide{}.png'\n", inputs.len() - 1));
    let list_path = temp.path().join("slides.txt");
    std::fs::write(&list_path, list).map_err(|err| MediaError::io(&list_path, err))?;

    let label = if inputs.len() > 1 {
        Some("slideshow")
    } else {
        None
    };
    let output = naming::output_for(first, env.output_folder, label, target.extension());
    let partial = encode::partial_path(output.path());
    let codec = if target == ConvertTarget::Webm {
        "libvpx-vp9"
    } else {
        env.hardware
            .software(options.codec)
            .map_or("libx264", |e| e.name.as_str())
    };
    let mut args: Vec<String> = vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list_path.display().to_string(),
        "-vf".into(),
        "fps=30,format=yuv420p".into(),
        "-c:v".into(),
        codec.into(),
    ];
    let crf = 40 - options.quality() * 24 / 100;
    if codec == "libvpx-vp9" {
        args.extend(["-crf".into(), crf.to_string(), "-b:v".into(), "0".into()]);
    } else {
        args.extend(["-crf".into(), crf.to_string()]);
    }
    if target == ConvertTarget::Mp4 {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.extend(output_args(&partial));
    let total = seconds * inputs.len() as f64;
    let span = vellum_media::control::Span {
        inner: control,
        start: 0.4,
        end: 1.0,
    };
    if let Err(err) = run_ffmpeg(ffmpeg, &args, Some(total), &span) {
        let _ = std::fs::remove_file(&partial);
        return Err(err);
    }
    crate::finish_partial(&partial, output.path())?;
    let mut result = ToolResult::file(first, output.path());
    result.changes.push(format!(
        "{} pictures, {} seconds each.",
        inputs.len(),
        num(seconds)
    ));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_chains_past_two_times() {
        assert_eq!(tempo_filter(1.0), None);
        assert_eq!(tempo_filter(1.5).unwrap(), "atempo=1.5");
        assert_eq!(tempo_filter(3.0).unwrap(), "atempo=2,atempo=1.5");
        assert_eq!(tempo_filter(0.25).unwrap(), "atempo=0.5,atempo=0.5");
    }

    #[test]
    fn filters_rotate_flip_and_shrink() {
        let options = ConvertOptions {
            rotate: 90,
            flip_horizontal: true,
            max_width: Some(1280),
            ..ConvertOptions::default()
        };
        let filters = picture_filters(&options, true).join(",");
        assert_eq!(
            filters,
            "transpose=clock,hflip,scale='min(iw,1280)':'min(ih,ih)':force_original_aspect_ratio=decrease,scale=trunc(iw/2)*2:trunc(ih/2)*2"
        );
    }

    #[test]
    fn icons_hold_every_size() {
        let image = RgbaImage::from_pixel(300, 200, image::Rgba([10, 200, 30, 255]));
        let data = ico(&image, &[16, 32, 32, 999, 256]).unwrap();
        // 6 header bytes, then 16 per size
        assert_eq!(u16::from_le_bytes([data[4], data[5]]), 3);
        let back = image::load_from_memory(&data).unwrap();
        assert_eq!(back.width(), 256);
        // padded to a square, so the top edge is see-through
        assert_eq!(back.to_rgba8().get_pixel(128, 2)[3], 0);
    }
}
