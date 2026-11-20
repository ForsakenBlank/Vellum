//! ffprobe output turned into [`MediaInfo`].

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::{FfmpegTools, run_output};
use crate::error::{MediaError, MediaResult};
use crate::info::{
    AudioInfo, ColorInfo, FrameRateMode, HdrFormat, HdrInfo, MediaInfo, MediaKind, MetadataTag,
    Rational, StreamInfo, StreamKind, VideoInfo, rotated_size,
};

/// how many packets to look at when checking for a variable frame rate. enough to catch a
/// phone's frame rate changing in a few seconds, quick even for 4k files.
const VFR_SAMPLE_PACKETS: u32 = 600;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProbeOutput {
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawFormat {
    format_name: Option<String>,
    format_long_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
    tags: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawStream {
    index: u32,
    codec_name: Option<String>,
    codec_long_name: Option<String>,
    profile: Option<String>,
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    sample_aspect_ratio: Option<String>,
    pix_fmt: Option<String>,
    color_range: Option<String>,
    color_space: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    field_order: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
    bits_per_raw_sample: Option<String>,
    nb_frames: Option<String>,
    sample_fmt: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    channel_layout: Option<String>,
    bits_per_sample: Option<u32>,
    disposition: BTreeMap<String, i64>,
    tags: BTreeMap<String, Value>,
    side_data_list: Vec<Value>,
}

impl RawStream {
    fn kind(&self) -> StreamKind {
        match self.codec_type.as_deref() {
            Some("video") if self.flag("attached_pic") => StreamKind::Picture,
            Some("video") => StreamKind::Video,
            Some("audio") => StreamKind::Audio,
            Some("subtitle") => StreamKind::Subtitle,
            Some("attachment") => StreamKind::Attachment,
            _ => StreamKind::Data,
        }
    }

    fn flag(&self, name: &str) -> bool {
        self.disposition.get(name).copied().unwrap_or(0) != 0
    }

    fn tag(&self, name: &str) -> Option<String> {
        self.tags
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value_text(value))
    }
}

/// probes any file ffprobe can open
pub fn probe_media(tools: &FfmpegTools, path: &Path) -> MediaResult<MediaInfo> {
    let size_bytes = std::fs::metadata(path)
        .map_err(|err| MediaError::io(path, err))?
        .len();
    let mut cmd = tools.ffprobe();
    cmd.args(["-print_format", "json", "-show_format", "-show_streams"])
        .arg(path);
    let out = run_output(&mut cmd, "ffprobe").map_err(|err| match err {
        MediaError::Tool { message, .. } => MediaError::Unsupported(format!(
            "Vellum could not read this file. It may be damaged, or in a format FFmpeg does \
             not know. ({message})"
        )),
        other => other,
    })?;
    let parsed: ProbeOutput = serde_json::from_slice(&out)
        .map_err(|err| MediaError::tool("ffprobe", format!("its output made no sense: {err}")))?;
    let mut info = build_info(path, size_bytes, &parsed);

    // the extra checks only matter for video, and each one is a second quick ffprobe run
    if let Some(video) = info.video.as_mut() {
        let base_rate = parsed
            .streams
            .iter()
            .find(|s| s.index == video.stream_index)
            .and_then(|s| s.r_frame_rate.as_deref())
            .and_then(Rational::parse);
        match packet_times(tools, path, video.stream_index) {
            Ok(times) => {
                video.frame_rate_mode = frame_rate_mode(&times, video.frame_rate, base_rate);
            }
            Err(err) => info
                .warnings
                .push(format!("The frame rate could not be checked: {err}")),
        }
        if needs_hdr_details(video) {
            match frame_side_data(tools, path, video.stream_index) {
                Ok(side) => apply_hdr_side_data(video, &side),
                Err(err) => info
                    .warnings
                    .push(format!("HDR details could not be read: {err}")),
            }
        }
    }
    Ok(info)
}

fn build_info(path: &Path, size_bytes: u64, probe: &ProbeOutput) -> MediaInfo {
    let empty = RawFormat::default();
    let format = probe.format.as_ref().unwrap_or(&empty);
    let mut tags = Vec::new();
    for (key, value) in &format.tags {
        tags.push(MetadataTag::new("File", key, value_text(value)));
    }

    let mut streams = Vec::new();
    let mut video = None;
    let mut audio = None;
    for stream in &probe.streams {
        let kind = stream.kind();
        streams.push(StreamInfo {
            index: stream.index,
            kind,
            codec: stream.codec_name.clone(),
            language: stream.tag("language").filter(|l| l != "und"),
            title: stream.tag("title"),
            default: stream.flag("default"),
        });
        for (key, value) in &stream.tags {
            tags.push(MetadataTag::new(
                format!("Stream {}", stream.index),
                key,
                value_text(value),
            ));
        }
        match kind {
            StreamKind::Video if video.is_none() => video = video_info(stream),
            StreamKind::Audio if audio.is_none() => audio = Some(audio_info(stream)),
            _ => {}
        }
    }

    let format_name = format.format_name.clone().unwrap_or_default();
    let duration_secs = parse_f64(format.duration.as_deref())
        .or_else(|| {
            probe
                .streams
                .iter()
                .filter_map(|s| parse_f64(s.duration.as_deref()))
                .reduce(f64::max)
        })
        .filter(|d| *d > 0.0);
    let kind = media_kind(&format_name, video.as_ref(), audio.is_some(), duration_secs);
    MediaInfo {
        path: path.display().to_string(),
        kind,
        format: short_format(&format_name, path),
        format_label: format_label(&format_name, format.format_long_name.as_deref(), kind),
        size_bytes,
        duration_secs,
        bit_rate: parse_u64(format.bit_rate.as_deref()),
        video,
        audio,
        image: None,
        streams,
        tags,
        warnings: Vec::new(),
    }
}

fn media_kind(
    format_name: &str,
    video: Option<&VideoInfo>,
    has_audio: bool,
    duration: Option<f64>,
) -> MediaKind {
    let still_formats = ["image2", "_pipe", "png", "mjpeg"];
    if let Some(video) = video {
        let single_frame = video.frame_count == Some(1) || duration.is_none();
        if !has_audio && single_frame && still_formats.iter().any(|f| format_name.contains(f)) {
            return MediaKind::Image;
        }
        if format_name == "gif" || format_name == "webp_pipe" || format_name == "apng" {
            return MediaKind::Animation;
        }
        return MediaKind::Video;
    }
    if has_audio {
        MediaKind::Audio
    } else {
        MediaKind::Other
    }
}

fn video_info(stream: &RawStream) -> Option<VideoInfo> {
    let width = stream.width.filter(|w| *w > 0)?;
    let height = stream.height.filter(|h| *h > 0)?;
    let rotation = rotation_of(stream);
    // non square pixels (old dv, some broadcast files) stretch the picture sideways
    let sar = stream
        .sample_aspect_ratio
        .as_deref()
        .and_then(Rational::parse)
        .filter(|r| r.num > 0 && r.den > 0);
    let shown_width = match sar {
        Some(sar) if sar.num != sar.den => {
            ((f64::from(width) * sar.num as f64 / sar.den as f64).round() as u32).max(1)
        }
        _ => width,
    };
    let (display_width, display_height) = rotated_size(shown_width, height, rotation);

    let avg = stream.avg_frame_rate.as_deref().and_then(Rational::parse);
    let real = stream.r_frame_rate.as_deref().and_then(Rational::parse);
    let frame_rate = avg.filter(|r| r.as_f64().is_some()).or(real);
    let fps = frame_rate.and_then(Rational::as_f64);
    let bit_depth = parse_u64(stream.bits_per_raw_sample.as_deref())
        .and_then(|b| u32::try_from(b).ok())
        .or_else(|| stream.pix_fmt.as_deref().map(pixel_format_depth));
    let color = ColorInfo {
        space: known(stream.color_space.as_deref()),
        primaries: known(stream.color_primaries.as_deref()),
        transfer: known(stream.color_transfer.as_deref()),
        range: known(stream.color_range.as_deref()),
    };
    let hdr = hdr_from_stream(stream, &color);
    Some(VideoInfo {
        stream_index: stream.index,
        codec: stream
            .codec_name
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        codec_label: codec_label(stream),
        profile: stream.profile.clone(),
        width,
        height,
        display_width,
        display_height,
        rotation,
        frame_rate,
        fps,
        frame_rate_mode: FrameRateMode::Unknown,
        frame_count: parse_u64(stream.nb_frames.as_deref()).filter(|n| *n > 0),
        bit_rate: parse_u64(stream.bit_rate.as_deref()),
        pixel_format: stream.pix_fmt.clone(),
        bit_depth,
        color,
        hdr,
        interlaced: matches!(
            stream.field_order.as_deref(),
            Some("tt" | "bb" | "tb" | "bt")
        ),
    })
}

fn audio_info(stream: &RawStream) -> AudioInfo {
    AudioInfo {
        stream_index: stream.index,
        codec: stream
            .codec_name
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        codec_label: codec_label(stream),
        profile: stream.profile.clone(),
        channels: stream.channels.unwrap_or(0),
        channel_layout: stream.channel_layout.clone(),
        sample_rate: parse_u64(stream.sample_rate.as_deref())
            .and_then(|r| u32::try_from(r).ok())
            .unwrap_or(0),
        sample_format: stream.sample_fmt.clone(),
        bit_depth: stream.bits_per_sample.filter(|b| *b > 0).or_else(|| {
            parse_u64(stream.bits_per_raw_sample.as_deref()).and_then(|b| u32::try_from(b).ok())
        }),
        bit_rate: parse_u64(stream.bit_rate.as_deref()),
        duration_secs: parse_f64(stream.duration.as_deref()),
    }
}

/// clockwise degrees to turn the stored picture upright. ffmpeg 5 and later report a
/// display matrix whose `rotation` is counter-clockwise; older files carry a `rotate` tag
/// that is already clockwise.
fn rotation_of(stream: &RawStream) -> u32 {
    let from_matrix = stream.side_data_list.iter().find_map(|side| {
        let kind = side.get("side_data_type")?.as_str()?;
        if kind != "Display Matrix" {
            return None;
        }
        side.get("rotation")?.as_f64()
    });
    let clockwise = match from_matrix {
        Some(ccw) => -ccw,
        None => stream
            .tag("rotate")
            .and_then(|r| r.parse::<f64>().ok())
            .unwrap_or(0.0),
    };
    // snap to a quarter turn, files only ever use those
    let quarter = (clockwise / 90.0).round() as i64;
    (quarter.rem_euclid(4) * 90) as u32
}

fn hdr_from_stream(stream: &RawStream, color: &ColorInfo) -> Option<HdrInfo> {
    let side_types: Vec<&str> = stream
        .side_data_list
        .iter()
        .filter_map(|side| side.get("side_data_type")?.as_str())
        .collect();
    let format = if side_types.iter().any(|t| t.contains("DOVI")) {
        HdrFormat::DolbyVision
    } else {
        match color.transfer.as_deref() {
            Some("smpte2084") => HdrFormat::Hdr10,
            Some("arib-std-b67") => HdrFormat::Hlg,
            _ => return None,
        }
    };
    let mut hdr = HdrInfo {
        format,
        mastering_max_nits: None,
        mastering_min_nits: None,
        max_content_nits: None,
        max_frame_average_nits: None,
    };
    read_hdr_values(&mut hdr, &stream.side_data_list);
    Some(hdr)
}

fn needs_hdr_details(video: &VideoInfo) -> bool {
    video
        .hdr
        .as_ref()
        .is_some_and(|hdr| hdr.mastering_max_nits.is_none() || hdr.format == HdrFormat::Hdr10)
}

fn apply_hdr_side_data(video: &mut VideoInfo, side: &[Value]) {
    let Some(hdr) = video.hdr.as_mut() else {
        return;
    };
    let has_plus = side.iter().any(|s| {
        s.get("side_data_type")
            .and_then(Value::as_str)
            .is_some_and(|t| t.contains("2094-40") || t.contains("HDR10+"))
    });
    if has_plus && hdr.format == HdrFormat::Hdr10 {
        hdr.format = HdrFormat::Hdr10Plus;
    }
    read_hdr_values(hdr, side);
}

fn read_hdr_values(hdr: &mut HdrInfo, side: &[Value]) {
    for entry in side {
        let Some(kind) = entry.get("side_data_type").and_then(Value::as_str) else {
            continue;
        };
        if kind.contains("Mastering display") {
            let nits = |key: &str| {
                entry
                    .get(key)
                    .map(value_text)
                    .as_deref()
                    .and_then(Rational::parse)
                    .and_then(|r| (r.den != 0).then(|| r.num as f64 / r.den as f64))
            };
            hdr.mastering_max_nits = hdr.mastering_max_nits.or(nits("max_luminance"));
            hdr.mastering_min_nits = hdr.mastering_min_nits.or(nits("min_luminance"));
        }
        if kind.contains("Content light level") {
            let value = |key: &str| {
                entry
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
            };
            hdr.max_content_nits = hdr.max_content_nits.or(value("max_content"));
            hdr.max_frame_average_nits = hdr.max_frame_average_nits.or(value("max_average"));
        }
    }
}

/// presentation times of the first packets of a video stream, in stream time base units
fn packet_times(tools: &FfmpegTools, path: &Path, stream: u32) -> MediaResult<Vec<i64>> {
    let mut cmd = tools.ffprobe();
    cmd.args(["-select_streams", &stream.to_string()])
        .args(["-show_entries", "packet=pts"])
        .args(["-read_intervals", &format!("%+#{VFR_SAMPLE_PACKETS}")])
        .args(["-of", "csv=p=0"])
        .arg(path);
    let out = run_output(&mut cmd, "ffprobe")?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse().ok())
        .collect())
}

fn frame_side_data(tools: &FfmpegTools, path: &Path, stream: u32) -> MediaResult<Vec<Value>> {
    let mut cmd = tools.ffprobe();
    cmd.args(["-select_streams", &stream.to_string()])
        .args(["-read_intervals", "%+#1"])
        .args(["-show_frames", "-show_entries", "frame=side_data_list"])
        .args(["-print_format", "json"])
        .arg(path);
    let out = run_output(&mut cmd, "ffprobe")?;
    let parsed: Value =
        serde_json::from_slice(&out).map_err(|err| MediaError::tool("ffprobe", err.to_string()))?;
    let frames = parsed.get("frames").and_then(Value::as_array);
    Ok(frames
        .into_iter()
        .flatten()
        .filter_map(|frame| frame.get("side_data_list")?.as_array())
        .flatten()
        .cloned()
        .collect())
}

/// decides whether frames arrive at a steady pace. `pts` are presentation times in decode
/// order; `average` and `real` are ffprobe's avg_frame_rate and r_frame_rate.
pub fn frame_rate_mode(
    pts: &[i64],
    average: Option<Rational>,
    real: Option<Rational>,
) -> FrameRateMode {
    let mut sorted = pts.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut gaps: Vec<i64> = sorted.windows(2).map(|w| w[1] - w[0]).collect();
    if gaps.len() >= 2 {
        let all = gaps.clone();
        gaps.sort_unstable();
        let median = gaps[gaps.len() / 2];
        // two ticks covers rounding in coarse time bases (33 and 34 ms at 29.97 fps)
        let tolerance = (median / 10).max(2);
        if all.iter().any(|gap| (gap - median).abs() > tolerance) {
            return FrameRateMode::Variable;
        }
    }
    let rates = (
        average.and_then(Rational::as_f64),
        real.and_then(Rational::as_f64),
    );
    match rates {
        // a frame rate that changes later in the file shows up as an average that does
        // not match the base rate. ignore absurd base rates some muxers write.
        (Some(avg), Some(base)) if base < 1000.0 && (avg - base).abs() / base > 0.02 => {
            FrameRateMode::Variable
        }
        _ if gaps.len() >= 2 => FrameRateMode::Constant,
        (Some(_), _) => FrameRateMode::Constant,
        _ => FrameRateMode::Unknown,
    }
}

fn codec_label(stream: &RawStream) -> String {
    let name = stream.codec_name.as_deref().unwrap_or("unknown");
    let friendly = match name {
        "h264" => "H.264",
        "hevc" => "H.265 / HEVC",
        "av1" => "AV1",
        "vp9" => "VP9",
        "vp8" => "VP8",
        "prores" => "Apple ProRes",
        "dnxhd" => "Avid DNxHD / DNxHR",
        "mpeg4" => "MPEG-4 Part 2",
        "mpeg2video" => "MPEG-2",
        "mjpeg" => "Motion JPEG",
        "gif" => "GIF",
        "aac" => "AAC",
        "mp3" | "mp3float" => "MP3",
        "opus" => "Opus",
        "vorbis" => "Vorbis",
        "flac" => "FLAC",
        "alac" => "Apple Lossless",
        "ac3" => "Dolby Digital (AC-3)",
        "eac3" => "Dolby Digital Plus (E-AC-3)",
        "truehd" => "Dolby TrueHD",
        "dts" => "DTS",
        name if name.starts_with("pcm_") => "PCM (uncompressed)",
        _ => {
            return stream
                .codec_long_name
                .clone()
                .unwrap_or_else(|| name.to_owned());
        }
    };
    friendly.to_owned()
}

fn short_format(format_name: &str, path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let names: Vec<&str> = format_name.split(',').collect();
    // ffprobe names whole families ("mov,mp4,m4a,..."), so the extension picks the member
    if let Some(ext) = ext.as_deref()
        && names.contains(&ext)
    {
        return ext.to_owned();
    }
    match names.first().copied().unwrap_or_default() {
        "matroska" if ext.as_deref() == Some("webm") => "webm".to_owned(),
        "matroska" => "mkv".to_owned(),
        "mov" => "mov".to_owned(),
        "" => ext.unwrap_or_else(|| "unknown".into()),
        first => first.trim_end_matches("_pipe").to_owned(),
    }
}

fn format_label(format_name: &str, long_name: Option<&str>, kind: MediaKind) -> String {
    let first = format_name.split(',').next().unwrap_or_default();
    let label = match first {
        "mov" => "QuickTime / MPEG-4",
        "matroska" => "Matroska / WebM",
        "avi" => "AVI",
        "mpegts" => "MPEG transport stream",
        "flv" => "Flash video",
        "wav" => "WAV audio",
        "mp3" => "MP3 audio",
        "flac" => "FLAC audio",
        "ogg" => "Ogg",
        "aiff" => "AIFF audio",
        "gif" => "GIF",
        _ => {
            return long_name.map(str::to_owned).unwrap_or_else(|| match kind {
                MediaKind::Audio => "Audio file".to_owned(),
                MediaKind::Image => "Image".to_owned(),
                _ => "Media file".to_owned(),
            });
        }
    };
    label.to_owned()
}

/// ffprobe writes "unknown" for colour fields it could not tell
fn known(value: Option<&str>) -> Option<String> {
    value
        .filter(|v| !v.is_empty() && *v != "unknown" && *v != "reserved")
        .map(str::to_owned)
}

fn pixel_format_depth(pix_fmt: &str) -> u32 {
    for (marker, depth) in [("16", 16), ("14", 14), ("12", 12), ("10", 10), ("9", 9)] {
        if pix_fmt.contains(&format!("p{marker}")) || pix_fmt.ends_with(&format!("{marker}le")) {
            return depth;
        }
    }
    8
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn parse_u64(text: Option<&str>) -> Option<u64> {
    text?.trim().parse().ok()
}

fn parse_f64(text: Option<&str>) -> Option<f64> {
    text?.trim().parse().ok().filter(|v: &f64| v.is_finite())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ffmpeg::testing;

    fn stream(value: Value) -> RawStream {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn display_matrix_rotation_is_turned_clockwise() {
        let side = |rotation: i64| {
            stream(json!({
                "side_data_list": [{ "side_data_type": "Display Matrix", "rotation": rotation }]
            }))
        };
        // phones write -90 for portrait video, which needs a quarter turn clockwise
        assert_eq!(rotation_of(&side(-90)), 90);
        assert_eq!(rotation_of(&side(90)), 270);
        assert_eq!(rotation_of(&side(180)), 180);
        assert_eq!(rotation_of(&side(-180)), 180);
        assert_eq!(
            rotation_of(&stream(json!({ "tags": { "rotate": "90" } }))),
            90
        );
        assert_eq!(rotation_of(&stream(json!({}))), 0);
    }

    #[test]
    fn portrait_phone_video_reports_its_shown_size() {
        let raw = stream(json!({
            "index": 0, "codec_type": "video", "codec_name": "hevc", "width": 1920, "height": 1080,
            "avg_frame_rate": "30000/1001", "r_frame_rate": "30/1", "pix_fmt": "yuv420p10le",
            "color_transfer": "arib-std-b67", "color_primaries": "bt2020", "color_space": "bt2020nc",
            "side_data_list": [{ "side_data_type": "Display Matrix", "rotation": -90 }]
        }));
        let video = video_info(&raw).unwrap();
        assert_eq!((video.display_width, video.display_height), (1080, 1920));
        assert_eq!(video.rotation, 90);
        assert_eq!(video.bit_depth, Some(10));
        assert_eq!(video.hdr.as_ref().map(|h| h.format), Some(HdrFormat::Hlg));
        assert_eq!(video.codec_label, "H.265 / HEVC");
        assert!((video.fps.unwrap() - 29.97).abs() < 0.01);
    }

    #[test]
    fn non_square_pixels_widen_the_shown_size() {
        let raw = stream(json!({
            "codec_type": "video", "width": 720, "height": 576, "sample_aspect_ratio": "16:11"
        }));
        let video = video_info(&raw).unwrap();
        assert_eq!((video.display_width, video.display_height), (1047, 576));
    }

    #[test]
    fn hdr10_mastering_values_are_read() {
        let raw = stream(json!({
            "codec_type": "video", "width": 3840, "height": 2160, "color_transfer": "smpte2084",
        }));
        let mut video = video_info(&raw).unwrap();
        apply_hdr_side_data(
            &mut video,
            &[
                json!({ "side_data_type": "Mastering display metadata",
                        "max_luminance": "10000000/10000", "min_luminance": "50/10000" }),
                json!({ "side_data_type": "Content light level metadata",
                        "max_content": 1000, "max_average": 400 }),
            ],
        );
        let hdr = video.hdr.unwrap();
        assert_eq!(hdr.format, HdrFormat::Hdr10);
        assert_eq!(hdr.mastering_max_nits, Some(1000.0));
        assert_eq!(hdr.mastering_min_nits, Some(0.005));
        assert_eq!(hdr.max_content_nits, Some(1000));
        assert_eq!(hdr.max_frame_average_nits, Some(400));
    }

    #[test]
    fn steady_and_uneven_frame_timing_are_told_apart() {
        let r = |n, d| Some(Rational::new(n, d));
        // 29.97 fps in a millisecond time base: 33 and 34 ms gaps are still constant
        let ntsc: Vec<i64> = (0..100)
            .map(|i| (i as f64 * 1001.0 / 30.0).round() as i64)
            .collect();
        assert_eq!(
            frame_rate_mode(&ntsc, r(30000, 1001), r(30000, 1001)),
            FrameRateMode::Constant
        );
        // b-frames arrive out of order but the presentation times are steady
        let reordered = [0, 3000, 1000, 2000, 6000, 4000, 5000];
        assert_eq!(
            frame_rate_mode(&reordered, r(30, 1), r(30, 1)),
            FrameRateMode::Constant
        );
        // a phone dropping to 20 fps in a dark scene
        let mut phone: Vec<i64> = (0..30).map(|i| i * 33).collect();
        phone.extend((0..30).map(|i| 990 + i * 50));
        assert_eq!(
            frame_rate_mode(&phone, r(30, 1), r(30, 1)),
            FrameRateMode::Variable
        );
        // steady at the start but the averages disagree
        assert_eq!(
            frame_rate_mode(&ntsc, r(24, 1), r(30, 1)),
            FrameRateMode::Variable
        );
        assert_eq!(frame_rate_mode(&[], None, None), FrameRateMode::Unknown);
    }

    #[test]
    fn short_formats_follow_the_extension_within_a_family() {
        let family = "mov,mp4,m4a,3gp,3g2,mj2";
        assert_eq!(short_format(family, Path::new("a.mp4")), "mp4");
        assert_eq!(short_format(family, Path::new("a.MOV")), "mov");
        assert_eq!(short_format(family, Path::new("a.bin")), "mov");
        assert_eq!(short_format("matroska,webm", Path::new("a.webm")), "webm");
        assert_eq!(short_format("matroska,webm", Path::new("a")), "mkv");
        assert_eq!(short_format("png_pipe", Path::new("a")), "png");
    }

    #[test]
    fn probing_a_real_file_fills_every_section() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mp4");
        testing::make(
            &tools,
            "-f lavfi -i testsrc2=s=320x240:r=25:d=2 -f lavfi -i sine=f=440:d=2:sample_rate=48000 \
             -c:v libx264 -pix_fmt yuv420p -c:a aac -ac 2 \
             -metadata title=Holiday -metadata location=+51.5074-000.1278/",
            &path,
        );
        let info = probe_media(&tools, &path).unwrap();
        assert_eq!(info.kind, MediaKind::Video);
        assert_eq!(info.format, "mp4");
        assert!((info.duration_secs.unwrap() - 2.0).abs() < 0.1);
        let video = info.video.as_ref().unwrap();
        assert_eq!((video.width, video.height), (320, 240));
        assert_eq!(video.codec, "h264");
        assert_eq!(video.fps, Some(25.0));
        assert_eq!(video.frame_rate_mode, FrameRateMode::Constant);
        assert_eq!(video.frame_count, Some(50));
        let audio = info.audio.as_ref().unwrap();
        assert_eq!(audio.sample_rate, 48000);
        assert_eq!(audio.channels, 2);
        assert!(
            info.tags
                .iter()
                .any(|t| t.key == "title" && t.value == "Holiday")
        );
        assert!(info.tags.iter().any(|t| t.key == "location" && t.sensitive));
    }

    #[test]
    fn variable_frame_rate_video_is_detected() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vfr.mkv");
        // the first second runs at 30 fps, the next at 10
        testing::make(
            &tools,
            "-f lavfi -i testsrc2=s=160x120:r=30:d=2 \
             -vf setpts='if(lt(N,30),N/30,1+(N-30)/10)/TB' -fps_mode vfr -c:v libx264",
            &path,
        );
        let info = probe_media(&tools, &path).unwrap();
        assert_eq!(info.video.unwrap().frame_rate_mode, FrameRateMode::Variable);
    }

    #[test]
    fn audio_files_are_audio() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.flac");
        testing::make(&tools, "-f lavfi -i sine=d=1:sample_rate=44100", &path);
        let info = probe_media(&tools, &path).unwrap();
        assert_eq!(info.kind, MediaKind::Audio);
        assert_eq!(info.format, "flac");
        assert_eq!(info.audio.unwrap().codec_label, "FLAC");
    }

    #[test]
    fn garbage_is_reported_plainly() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.mp4");
        std::fs::write(&path, b"this is not a video at all").unwrap();
        let err = probe_media(&tools, &path).unwrap_err();
        assert!(matches!(err, MediaError::Unsupported(_)), "{err}");
        assert!(
            err.to_string()
                .starts_with("Vellum could not read this file")
        );
    }
}
