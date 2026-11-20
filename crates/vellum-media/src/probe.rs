//! one [`MediaInfo`] for any file: what it is, how big, how long, which codecs, colour,
//! hdr, rotation and every metadata tag.
//!
//! images are read by vellum's own decoders (exif through `kamadak-exif`), everything else
//! by ffprobe. both paths fill the same struct so the info panel, the export engine and the
//! cli never need to care where a file came from.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use ::image::metadata::Orientation;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::Ffmpeg;
use crate::ffmpeg::process;
use crate::picture::{self, PictureFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum MediaKind {
    Image,
    /// gif and other images with frames over time
    Animation,
    Video,
    Audio,
    Vector,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaInfo {
    pub path: String,
    pub name: String,
    pub kind: MediaKind,
    /// the container or image format, eg "MP4", "Matroska", "JPEG"
    pub format: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    #[ts(type = "number | null")]
    pub modified_ms: Option<u64>,
    /// upright size, after rotation
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_secs: Option<f64>,
    #[ts(type = "number | null")]
    pub bit_rate: Option<u64>,
    pub video: Option<VideoStream>,
    pub audio: Vec<AudioStream>,
    pub subtitles: Vec<SubtitleStream>,
    pub image: Option<ImageDetails>,
    pub chapters: u32,
    pub tags: Vec<MetadataTag>,
    /// gps coordinates are in the metadata, worth knowing before sharing the file
    pub has_location: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VideoStream {
    /// short ffmpeg name, eg "h264"
    pub codec: String,
    /// eg "H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10"
    pub codec_name: String,
    pub profile: Option<String>,
    pub pixel_format: Option<String>,
    pub bit_depth: Option<u32>,
    /// size as stored, before rotation
    pub coded_width: u32,
    pub coded_height: u32,
    pub frame_rate: Option<f64>,
    /// the exact rate, eg "30000/1001"
    pub frame_rate_text: Option<String>,
    pub variable_frame_rate: bool,
    #[ts(type = "number | null")]
    pub frame_count: Option<u64>,
    #[ts(type = "number | null")]
    pub bit_rate: Option<u64>,
    /// clockwise degrees to turn the picture for display: 0, 90, 180 or 270
    pub rotation: u32,
    pub color: ColorInfo,
    pub hdr: Option<HdrInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColorInfo {
    /// matrix, eg "bt709"
    pub space: Option<String>,
    pub primaries: Option<String>,
    pub transfer: Option<String>,
    /// "tv" (limited) or "pc" (full)
    pub range: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HdrFormat {
    /// pq with static mastering metadata
    Hdr10,
    Hdr10Plus,
    Hlg,
    DolbyVision,
    /// pq without mastering metadata
    Pq,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HdrInfo {
    pub format: HdrFormat,
    /// eg "max 1000 nits, min 0.0050 nits"
    pub mastering_display: Option<String>,
    /// brightest pixel, in nits
    pub max_cll: Option<u32>,
    /// brightest frame average, in nits
    pub max_fall: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioStream {
    pub codec: String,
    pub codec_name: String,
    pub channels: u32,
    pub channel_layout: Option<String>,
    pub sample_rate: Option<u32>,
    #[ts(type = "number | null")]
    pub bit_rate: Option<u64>,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubtitleStream {
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageDetails {
    pub format: PictureFormat,
    /// eg "RGBA, 8 bits per channel"
    pub color: Option<String>,
    pub has_alpha: Option<bool>,
    pub frames: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MetadataTag {
    /// where it came from: "File", "Video", "Audio 1", "EXIF", "GPS", "Camera"
    pub group: String,
    pub key: String,
    pub value: String,
}

/// longest tag value kept, longer ones (embedded thumbnails, xml) are cut short
const MAX_TAG_LEN: usize = 300;

/// reads everything vellum can tell about a file. `ffmpeg` is needed for video, audio and
/// a few image formats; without it those return [`MediaError::FfmpegMissing`].
pub fn probe(path: &Path, ffmpeg: Option<&Ffmpeg>) -> MediaResult<MediaInfo> {
    let meta = std::fs::metadata(path).map_err(|err| MediaError::io(path, err))?;
    let mut info = MediaInfo {
        path: path.display().to_string(),
        name: vellum_core::files::display_name(path),
        kind: MediaKind::Unknown,
        format: String::new(),
        size_bytes: meta.len(),
        modified_ms: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| u64::try_from(d.as_millis()).ok()),
        width: None,
        height: None,
        duration_secs: None,
        bit_rate: None,
        video: None,
        audio: Vec::new(),
        subtitles: Vec::new(),
        image: None,
        chapters: 0,
        tags: Vec::new(),
        has_location: false,
    };
    match PictureFormat::of_path(path)? {
        Some(format) => probe_picture(path, format, ffmpeg, &mut info)?,
        None => {
            let ffmpeg = ffmpeg.ok_or(MediaError::FfmpegMissing)?;
            probe_av(path, ffmpeg, &mut info)?;
        }
    }
    Ok(info)
}

fn probe_picture(
    path: &Path,
    format: PictureFormat,
    ffmpeg: Option<&Ffmpeg>,
    info: &mut MediaInfo,
) -> MediaResult<()> {
    let (_, width, height) = picture::dimensions(path, ffmpeg)?;
    info.width = Some(width);
    info.height = Some(height);
    info.format = format.label().to_owned();
    info.kind = match format {
        PictureFormat::Svg => MediaKind::Vector,
        PictureFormat::Gif => MediaKind::Animation,
        _ => MediaKind::Image,
    };
    let (color, has_alpha) = picture_color(path, format);
    let mut details = ImageDetails {
        format,
        color,
        has_alpha,
        frames: None,
    };
    // gifs have a duration and a frame count, ffprobe knows both without decoding pixels
    if format == PictureFormat::Gif
        && let Some(ffmpeg) = ffmpeg
        && let Ok(json) = ffprobe_json(ffmpeg, path, &["-count_packets"])
    {
        info.duration_secs = json["format"]["duration"].as_str().and_then(parse_f64);
        details.frames = json["streams"][0]["nb_read_packets"]
            .as_str()
            .and_then(|n| n.parse().ok());
    }
    info.image = Some(details);
    let (tags, has_location) = exif_tags(path);
    info.tags = tags;
    info.has_location = has_location;
    Ok(())
}

/// the colour layout of raster formats the image crate reads
fn picture_color(path: &Path, format: PictureFormat) -> (Option<String>, Option<bool>) {
    use ::image::{ColorType, ImageDecoder};
    let Some(image_format) = format_for_color(format) else {
        return (None, None);
    };
    let decoder = File::open(path).ok().and_then(|file| {
        ::image::ImageReader::with_format(BufReader::new(file), image_format)
            .into_decoder()
            .ok()
    });
    let Some(decoder) = decoder else {
        return (None, None);
    };
    let colour = decoder.color_type();
    let (layout, bits) = match colour {
        ColorType::L8 => ("Greyscale", 8),
        ColorType::La8 => ("Greyscale with alpha", 8),
        ColorType::Rgb8 => ("RGB", 8),
        ColorType::Rgba8 => ("RGBA", 8),
        ColorType::L16 => ("Greyscale", 16),
        ColorType::La16 => ("Greyscale with alpha", 16),
        ColorType::Rgb16 => ("RGB", 16),
        ColorType::Rgba16 => ("RGBA", 16),
        ColorType::Rgb32F => ("RGB float", 32),
        ColorType::Rgba32F => ("RGBA float", 32),
        _ => return (None, Some(colour.has_alpha())),
    };
    (
        Some(format!("{layout}, {bits} bits per channel")),
        Some(colour.has_alpha()),
    )
}

fn format_for_color(format: PictureFormat) -> Option<::image::ImageFormat> {
    use ::image::ImageFormat as F;
    Some(match format {
        PictureFormat::Png => F::Png,
        PictureFormat::Jpeg => F::Jpeg,
        PictureFormat::Webp => F::WebP,
        PictureFormat::Gif => F::Gif,
        PictureFormat::Tiff => F::Tiff,
        PictureFormat::Bmp => F::Bmp,
        PictureFormat::Tga => F::Tga,
        PictureFormat::Qoi => F::Qoi,
        PictureFormat::Hdr => F::Hdr,
        PictureFormat::Exr => F::OpenExr,
        _ => return None,
    })
}

/// the exif orientation of a photo, if it has one
pub fn exif_orientation(path: &Path) -> Option<Orientation> {
    let exif = read_exif(path)?;
    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    let value = field.value.get_uint(0)?;
    Orientation::from_exif(u8::try_from(value).ok()?)
}

fn read_exif(path: &Path) -> Option<exif::Exif> {
    let file = File::open(path).ok()?;
    exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .ok()
}

/// every readable exif field, grouped, and whether gps coordinates are among them
pub fn exif_tags(path: &Path) -> (Vec<MetadataTag>, bool) {
    let Some(exif) = read_exif(path) else {
        return (Vec::new(), false);
    };
    let mut tags = Vec::new();
    let mut has_location = false;
    for field in exif.fields() {
        if field.ifd_num != exif::In::PRIMARY {
            continue;
        }
        // maker notes are opaque vendor blobs, and the interop block is plumbing
        if field.tag == exif::Tag::MakerNote || field.tag.context() == exif::Context::Interop {
            continue;
        }
        let group = match field.tag.context() {
            exif::Context::Gps => {
                if matches!(field.tag, exif::Tag::GPSLatitude | exif::Tag::GPSLongitude) {
                    has_location = true;
                }
                "GPS"
            }
            exif::Context::Tiff => "Camera",
            _ => "EXIF",
        };
        let value = field.display_value().with_unit(&exif).to_string();
        tags.push(MetadataTag {
            group: group.to_owned(),
            key: field.tag.to_string(),
            value: clip(value),
        });
    }
    (tags, has_location)
}

fn clip(mut value: String) -> String {
    value = value.trim().to_owned();
    if value.len() > MAX_TAG_LEN {
        let mut cut = MAX_TAG_LEN;
        while !value.is_char_boundary(cut) {
            cut -= 1;
        }
        value.truncate(cut);
        value.push_str("...");
    }
    value
}

fn ffprobe_json(ffmpeg: &Ffmpeg, path: &Path, extra: &[&str]) -> MediaResult<Value> {
    let mut cmd = ffmpeg.probe_command();
    cmd.args([
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        "-show_chapters",
    ])
    .args(extra)
    .arg(path);
    let out = process::capture(cmd, Duration::from_secs(30))?;
    serde_json::from_slice(&out).map_err(|err| MediaError::decode(path, err))
}

fn probe_av(path: &Path, ffmpeg: &Ffmpeg, info: &mut MediaInfo) -> MediaResult<()> {
    let json = ffprobe_json(ffmpeg, path, &[]).map_err(|err| match err {
        MediaError::Ffmpeg(reason) => MediaError::decode(path, reason),
        other => other,
    })?;
    fill_from_ffprobe(&json, info);
    if let Some(video) = &mut info.video {
        if !video.variable_frame_rate {
            video.variable_frame_rate = packets_look_variable(ffmpeg, path);
        }
        if video.color.transfer.as_deref() == Some("smpte2084") {
            enrich_hdr10(ffmpeg, path, video);
        }
    }
    Ok(())
}

/// fills `info` from ffprobe's json. split out so it can be tested on canned output.
pub fn fill_from_ffprobe(json: &Value, info: &mut MediaInfo) {
    let format = &json["format"];
    info.format = container_label(format["format_name"].as_str().unwrap_or_default());
    info.duration_secs = format["duration"].as_str().and_then(parse_f64);
    info.bit_rate = format["bit_rate"].as_str().and_then(|b| b.parse().ok());
    info.chapters = json["chapters"]
        .as_array()
        .map_or(0, |c| u32::try_from(c.len()).unwrap_or(u32::MAX));
    push_tags(&mut info.tags, "File", &format["tags"]);

    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    let mut audio_number = 0;
    for stream in &streams {
        match stream["codec_type"].as_str() {
            Some("video") if stream["disposition"]["attached_pic"].as_i64() != Some(1) => {
                if info.video.is_none() {
                    let video = video_stream(stream);
                    push_tags(&mut info.tags, "Video", &stream["tags"]);
                    let (w, h) = (video.coded_width, video.coded_height);
                    let (w, h) = if video.rotation % 180 == 90 {
                        (h, w)
                    } else {
                        (w, h)
                    };
                    info.width = Some(w);
                    info.height = Some(h);
                    info.video = Some(video);
                }
            }
            Some("audio") => {
                audio_number += 1;
                push_tags(
                    &mut info.tags,
                    &format!("Audio {audio_number}"),
                    &stream["tags"],
                );
                info.audio.push(audio_stream(stream));
            }
            Some("subtitle") => info.subtitles.push(SubtitleStream {
                codec: text(&stream["codec_name"]).unwrap_or_default(),
                language: text(&stream["tags"]["language"]),
                title: text(&stream["tags"]["title"]),
            }),
            _ => {}
        }
    }
    info.kind = if info.video.is_some() {
        MediaKind::Video
    } else if !info.audio.is_empty() {
        MediaKind::Audio
    } else {
        MediaKind::Unknown
    };
    info.has_location = info.tags.iter().any(|tag| {
        let key = tag.key.to_ascii_lowercase();
        key == "location" || key.ends_with("location.iso6709") || key == "location-eng"
    });
}

fn video_stream(stream: &Value) -> VideoStream {
    let pixel_format = text(&stream["pix_fmt"]);
    let avg = text(&stream["avg_frame_rate"]).filter(|r| rate(r).is_some());
    let real = text(&stream["r_frame_rate"]).filter(|r| rate(r).is_some());
    let frame_rate_text = avg.clone().or_else(|| real.clone());
    // when the average and the base rate disagree by more than rounding the file is vfr
    let variable_frame_rate = match (
        avg.as_deref().and_then(rate),
        real.as_deref().and_then(rate),
    ) {
        (Some(a), Some(r)) => (a - r).abs() / r > 0.01,
        _ => false,
    };
    let rotation = stream["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|side| side["rotation"].as_f64())
        .or_else(|| {
            stream["tags"]["rotate"]
                .as_str()
                .and_then(parse_f64)
                .map(|r| -r)
        })
        .map_or(0, clockwise);
    VideoStream {
        codec: text(&stream["codec_name"]).unwrap_or_default(),
        codec_name: text(&stream["codec_long_name"]).unwrap_or_default(),
        profile: text(&stream["profile"]),
        bit_depth: stream["bits_per_raw_sample"]
            .as_str()
            .and_then(|b| b.parse().ok())
            .or_else(|| pixel_format.as_deref().map(bit_depth_of)),
        pixel_format,
        coded_width: stream["width"]
            .as_u64()
            .and_then(|w| u32::try_from(w).ok())
            .unwrap_or(0),
        coded_height: stream["height"]
            .as_u64()
            .and_then(|h| u32::try_from(h).ok())
            .unwrap_or(0),
        frame_rate: frame_rate_text.as_deref().and_then(rate),
        frame_rate_text,
        variable_frame_rate,
        frame_count: stream["nb_frames"].as_str().and_then(|n| n.parse().ok()),
        bit_rate: stream["bit_rate"].as_str().and_then(|b| b.parse().ok()),
        rotation,
        color: ColorInfo {
            space: known(&stream["color_space"]),
            primaries: known(&stream["color_primaries"]),
            transfer: known(&stream["color_transfer"]),
            range: known(&stream["color_range"]),
        },
        hdr: hdr_from(stream),
    }
}

fn audio_stream(stream: &Value) -> AudioStream {
    AudioStream {
        codec: text(&stream["codec_name"]).unwrap_or_default(),
        codec_name: text(&stream["codec_long_name"]).unwrap_or_default(),
        channels: stream["channels"]
            .as_u64()
            .and_then(|c| u32::try_from(c).ok())
            .unwrap_or(0),
        channel_layout: text(&stream["channel_layout"]),
        sample_rate: stream["sample_rate"].as_str().and_then(|s| s.parse().ok()),
        bit_rate: stream["bit_rate"].as_str().and_then(|b| b.parse().ok()),
        language: text(&stream["tags"]["language"]).filter(|l| l != "und"),
        title: text(&stream["tags"]["title"]),
    }
}

fn hdr_from(stream: &Value) -> Option<HdrInfo> {
    let side: Vec<&Value> = stream["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let has_side = |name: &str| {
        side.iter().any(|s| {
            s["side_data_type"]
                .as_str()
                .is_some_and(|t| t.contains(name))
        })
    };
    let mut hdr = HdrInfo {
        format: HdrFormat::Pq,
        mastering_display: None,
        max_cll: None,
        max_fall: None,
    };
    if has_side("DOVI") {
        hdr.format = HdrFormat::DolbyVision;
    } else {
        match stream["color_transfer"].as_str() {
            Some("smpte2084") => {}
            Some("arib-std-b67") => hdr.format = HdrFormat::Hlg,
            _ => return None,
        }
    }
    apply_hdr_side_data(&mut hdr, &side);
    Some(hdr)
}

fn apply_hdr_side_data(hdr: &mut HdrInfo, side: &[&Value]) {
    for data in side {
        let kind = data["side_data_type"].as_str().unwrap_or_default();
        if kind.starts_with("Mastering display") {
            let max = data["max_luminance"].as_str().and_then(fraction);
            let min = data["min_luminance"].as_str().and_then(fraction);
            if let (Some(max), Some(min)) = (max, min) {
                hdr.mastering_display = Some(format!("max {max:.0} nits, min {min:.4} nits"));
            }
            if hdr.format == HdrFormat::Pq {
                hdr.format = HdrFormat::Hdr10;
            }
        } else if kind.starts_with("Content light level") {
            hdr.max_cll = data["max_content"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok());
            hdr.max_fall = data["max_average"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok());
        } else if kind.contains("HDR10+") || kind.contains("2094-40") {
            hdr.format = HdrFormat::Hdr10Plus;
        }
    }
}

/// hevc keeps hdr10 metadata in the first frame rather than the container, so look there
fn enrich_hdr10(ffmpeg: &Ffmpeg, path: &Path, video: &mut VideoStream) {
    let mut cmd = ffmpeg.probe_command();
    cmd.args(["-select_streams", "v:0", "-read_intervals", "%+#1"])
        .args(["-show_frames", "-print_format", "json"])
        .arg(path);
    let Ok(out) = process::capture(cmd, Duration::from_secs(20)) else {
        return;
    };
    let Ok(json) = serde_json::from_slice::<Value>(&out) else {
        return;
    };
    let side: Vec<&Value> = json["frames"][0]["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    if let Some(hdr) = &mut video.hdr {
        apply_hdr_side_data(hdr, &side);
    }
}

/// looks at the gaps between the first few hundred frames. some phones write a constant
/// rate in the header and then vary the timing anyway, which breaks frame accurate editing
/// if nobody notices.
fn packets_look_variable(ffmpeg: &Ffmpeg, path: &Path) -> bool {
    let mut cmd = ffmpeg.probe_command();
    cmd.args(["-select_streams", "v:0", "-read_intervals", "%+#300"])
        .args(["-show_entries", "packet=pts_time", "-of", "csv=p=0"])
        .arg(path);
    let Ok(out) = process::capture(cmd, Duration::from_secs(20)) else {
        return false;
    };
    let mut times: Vec<f64> = String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| parse_f64(line.trim().trim_end_matches(',')))
        .collect();
    times.sort_by(f64::total_cmp);
    gaps_are_variable(&times)
}

pub fn gaps_are_variable(sorted_times: &[f64]) -> bool {
    let mut gaps: Vec<f64> = sorted_times
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|gap| *gap > 0.0)
        .collect();
    if gaps.len() < 10 {
        return false;
    }
    gaps.sort_by(f64::total_cmp);
    let median = gaps[gaps.len() / 2];
    // millisecond timestamps in mkv round 33.3 ms to 33 or 34, so allow some slack
    let odd = gaps
        .iter()
        .filter(|gap| (*gap - median).abs() > median * 0.2 + 0.0015)
        .count();
    odd * 50 > gaps.len()
}

fn push_tags(tags: &mut Vec<MetadataTag>, group: &str, source: &Value) {
    let Some(map) = source.as_object() else {
        return;
    };
    for (key, value) in map {
        let value = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        tags.push(MetadataTag {
            group: group.to_owned(),
            key: key.clone(),
            value: clip(value),
        });
    }
}

/// "mov,mp4,m4a,3gp,3g2,mj2" is what ffmpeg calls the mp4 family
fn container_label(name: &str) -> String {
    let first = name.split(',').next().unwrap_or(name);
    match first {
        "mov" => "MP4 / QuickTime",
        "matroska" => {
            if name.contains("webm") {
                "Matroska / WebM"
            } else {
                "Matroska"
            }
        }
        "mp3" => "MP3",
        "wav" => "WAV",
        "flac" => "FLAC",
        "ogg" => "Ogg",
        "avi" => "AVI",
        "mpegts" => "MPEG transport stream",
        "aiff" => "AIFF",
        "gif" => "GIF",
        "image2" => "Image",
        "" => "Unknown",
        other => return other.to_uppercase(),
    }
    .to_owned()
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned).filter(|s| !s.is_empty())
}

/// drops ffprobe's placeholders for missing colour fields
fn known(value: &Value) -> Option<String> {
    text(value).filter(|s| s != "unknown" && s != "reserved")
}

fn parse_f64(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// "30000/1001" as a number, none for "0/0"
fn rate(text: &str) -> Option<f64> {
    fraction(text).filter(|r| *r > 0.0)
}

fn fraction(text: &str) -> Option<f64> {
    match text.split_once('/') {
        Some((n, d)) => {
            let n: f64 = n.trim().parse().ok()?;
            let d: f64 = d.trim().parse().ok()?;
            (d != 0.0).then(|| n / d)
        }
        None => parse_f64(text.trim()),
    }
}

fn bit_depth_of(pixel_format: &str) -> u32 {
    if pixel_format.contains("12") {
        12
    } else if pixel_format.contains("10") {
        10
    } else if pixel_format.contains("16") {
        16
    } else {
        8
    }
}

/// ffprobe gives the display matrix rotation counterclockwise, eg -90 for a phone held
/// upright. people think of it clockwise.
fn clockwise(degrees: f64) -> u32 {
    let turned = (-degrees).round() as i64;
    let normalised = turned.rem_euclid(360);
    // snap to quarter turns, the only ones players honour
    ((normalised + 45) / 90 % 4 * 90) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_info() -> MediaInfo {
        MediaInfo {
            path: String::new(),
            name: String::new(),
            kind: MediaKind::Unknown,
            format: String::new(),
            size_bytes: 0,
            modified_ms: None,
            width: None,
            height: None,
            duration_secs: None,
            bit_rate: None,
            video: None,
            audio: Vec::new(),
            subtitles: Vec::new(),
            image: None,
            chapters: 0,
            tags: Vec::new(),
            has_location: false,
        }
    }

    #[test]
    fn a_phone_video_is_read_upright_with_its_hdr_and_location() {
        let probe = json!({
            "format": {
                "format_name": "mov,mp4,m4a,3gp,3g2,mj2",
                "duration": "12.500000",
                "bit_rate": "8000000",
                "tags": { "com.apple.quicktime.location.ISO6709": "+51.5+000.1/" }
            },
            "streams": [
                {
                    "codec_type": "video", "codec_name": "hevc",
                    "codec_long_name": "H.265 / HEVC", "profile": "Main 10",
                    "width": 3840, "height": 2160, "pix_fmt": "yuv420p10le",
                    "avg_frame_rate": "30000/1001", "r_frame_rate": "30000/1001",
                    "nb_frames": "375", "color_transfer": "arib-std-b67",
                    "color_primaries": "bt2020", "color_space": "bt2020nc", "color_range": "tv",
                    "side_data_list": [ { "side_data_type": "Display Matrix", "rotation": -90 } ],
                    "disposition": { "attached_pic": 0 }
                },
                {
                    "codec_type": "audio", "codec_name": "aac", "codec_long_name": "AAC",
                    "channels": 2, "channel_layout": "stereo", "sample_rate": "48000",
                    "tags": { "language": "und" }
                }
            ],
            "chapters": []
        });
        let mut info = empty_info();
        fill_from_ffprobe(&probe, &mut info);
        assert_eq!(info.kind, MediaKind::Video);
        assert_eq!(info.format, "MP4 / QuickTime");
        assert_eq!((info.width, info.height), (Some(2160), Some(3840)));
        let video = info.video.unwrap();
        assert_eq!(video.rotation, 90);
        assert_eq!(video.bit_depth, Some(10));
        assert!((video.frame_rate.unwrap() - 29.97).abs() < 0.01);
        assert!(!video.variable_frame_rate);
        assert_eq!(video.hdr.unwrap().format, HdrFormat::Hlg);
        assert_eq!(info.audio[0].language, None);
        assert!(info.has_location);
    }

    #[test]
    fn album_art_does_not_make_audio_a_video() {
        let probe = json!({
            "format": { "format_name": "mp3", "duration": "180.0" },
            "streams": [
                { "codec_type": "audio", "codec_name": "mp3", "channels": 2, "sample_rate": "44100" },
                { "codec_type": "video", "codec_name": "mjpeg", "width": 500, "height": 500,
                  "disposition": { "attached_pic": 1 } }
            ]
        });
        let mut info = empty_info();
        fill_from_ffprobe(&probe, &mut info);
        assert_eq!(info.kind, MediaKind::Audio);
        assert!(info.video.is_none());
        assert_eq!(info.format, "MP3");
    }

    #[test]
    fn hdr10_needs_mastering_metadata() {
        let stream = json!({
            "color_transfer": "smpte2084",
            "side_data_list": [
                { "side_data_type": "Mastering display metadata",
                  "max_luminance": "10000000/10000", "min_luminance": "50/10000" },
                { "side_data_type": "Content light level metadata", "max_content": 1000, "max_average": 400 }
            ]
        });
        let hdr = hdr_from(&stream).unwrap();
        assert_eq!(hdr.format, HdrFormat::Hdr10);
        assert_eq!(hdr.max_cll, Some(1000));
        assert_eq!(
            hdr.mastering_display.as_deref(),
            Some("max 1000 nits, min 0.0050 nits")
        );
        assert_eq!(
            hdr_from(&json!({ "color_transfer": "smpte2084" }))
                .unwrap()
                .format,
            HdrFormat::Pq
        );
        assert!(hdr_from(&json!({ "color_transfer": "bt709" })).is_none());
    }

    #[test]
    fn rotation_is_clockwise_quarter_turns() {
        assert_eq!(clockwise(-90.0), 90);
        assert_eq!(clockwise(90.0), 270);
        assert_eq!(clockwise(180.0), 180);
        assert_eq!(clockwise(-180.0), 180);
        assert_eq!(clockwise(0.0), 0);
        assert_eq!(clockwise(-89.6), 90);
    }

    #[test]
    fn steady_gaps_are_constant_and_jumpy_ones_are_variable() {
        let steady: Vec<f64> = (0..120)
            .map(|i| f64::from(i) * 0.033 + if i % 3 == 0 { 0.001 } else { 0.0 })
            .collect();
        assert!(!gaps_are_variable(&steady));
        let mut jumpy = Vec::new();
        let mut t = 0.0;
        for i in 0..120 {
            t += if i % 4 == 0 { 1.0 / 60.0 } else { 1.0 / 30.0 };
            jumpy.push(t);
        }
        assert!(gaps_are_variable(&jumpy));
        assert!(!gaps_are_variable(&[0.0, 1.0]));
    }

    #[test]
    fn long_tags_are_cut_on_a_character_boundary() {
        let long = "é".repeat(400);
        let clipped = clip(long);
        assert!(clipped.ends_with("..."));
        assert!(clipped.len() <= MAX_TAG_LEN + 3);
    }
}
