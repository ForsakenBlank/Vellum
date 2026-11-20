//! video to gif, and the other short looping formats: animated webp, apng, avif and a
//! silent mp4 loop.
//!
//! one ffmpeg run does the whole picture: trim, speed, frame rate, crop, scale, caption,
//! reverse or boomerang, then for gifs a palette made from this clip's own colours. the
//! palette pass counts colours across frames (`stats_mode=diff`) and only redraws what moved
//! (`diff_mode=rectangle`), which keeps gifs small without a separate optimiser. "lossy"
//! then runs vellum's own pass over the result ([`lossy`]).
//!
//! gifski would give slightly nicer gifs but is agpl, which would force the whole app to be
//! open source. see `ARCHITECTURE.md` section 23.

pub mod caption;
pub mod frames;
pub mod lossy;

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_media::ffmpeg::encode::partial_path;
use vellum_media::probe::{MediaInfo, MediaKind};
use vellum_media::{Control, MediaError, MediaResult};

use crate::{Env, num, output_args, run_ffmpeg};
pub use caption::{Caption, CaptionStyle};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AnimFormat {
    #[default]
    Gif,
    Webp,
    Apng,
    Avif,
    Mp4,
}

impl AnimFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Apng => "png",
            Self::Avif => "avif",
            Self::Mp4 => "mp4",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Gif => "GIF",
            Self::Webp => "WebP",
            Self::Apng => "APNG",
            Self::Avif => "AVIF",
            Self::Mp4 => "MP4",
        }
    }

    /// yuv encoders need even sides
    fn even_size(self) -> bool {
        matches!(self, Self::Avif | Self::Mp4)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Dither {
    None,
    /// an ordered pattern, crisp and compresses well
    Bayer,
    FloydSteinberg,
    /// smooth gradients, ffmpeg's default
    #[default]
    Sierra,
}

impl Dither {
    fn filter(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bayer => "bayer:bayer_scale=3",
            Self::FloydSteinberg => "floyd_steinberg",
            Self::Sierra => "sierra2_4a",
        }
    }
}

/// a rectangle in the source picture, upright, in pixels
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct AnimOptions {
    /// seconds into the source
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub crop: Option<Crop>,
    /// width of the result, none keeps the (cropped) source width. never upscales.
    pub width: Option<u32>,
    /// none keeps the source timing, which suits gifs that already have their own
    pub fps: Option<f64>,
    pub speed: f64,
    pub reverse: bool,
    /// plays forward then backward
    pub boomerang: bool,
    /// how many times it plays, 0 for forever
    pub loops: u32,
    pub caption: Caption,
    /// 2 to 256
    pub colors: u32,
    pub dither: Dither,
    /// 0 to 100, see [`lossy`]
    pub lossy: u32,
    pub format: AnimFormat,
    /// 0 to 100 for webp, avif and mp4
    pub quality: u32,
}

impl Default for AnimOptions {
    fn default() -> Self {
        Self {
            start: None,
            end: None,
            crop: None,
            width: Some(480),
            fps: Some(15.0),
            speed: 1.0,
            reverse: false,
            boomerang: false,
            loops: 0,
            caption: Caption::default(),
            colors: 256,
            dither: Dither::default(),
            lossy: 0,
            format: AnimFormat::Gif,
            quality: 75,
        }
    }
}

impl AnimOptions {
    fn speed(&self) -> f64 {
        if self.speed.is_finite() && self.speed > 0.0 {
            self.speed.clamp(0.1, 16.0)
        } else {
            1.0
        }
    }

    fn fps(&self) -> Option<f64> {
        self.fps
            .filter(|f| f.is_finite() && *f > 0.0)
            .map(|f| f.clamp(1.0, 60.0))
    }
}

/// what the renderer needs to know about its input
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimSource {
    pub width: u32,
    pub height: u32,
    pub duration: Option<f64>,
    pub fps: Option<f64>,
}

impl AnimSource {
    pub fn of(info: &MediaInfo) -> MediaResult<Self> {
        let (Some(width), Some(height)) = (info.width, info.height) else {
            return Err(MediaError::Unsupported(format!(
                "{} has no picture to make an animation from.",
                info.name
            )));
        };
        if !matches!(
            info.kind,
            MediaKind::Video | MediaKind::Animation | MediaKind::Image
        ) {
            return Err(MediaError::Unsupported(format!(
                "{} is not a video or a GIF.",
                info.name
            )));
        }
        Ok(Self {
            width,
            height,
            duration: info.duration_secs,
            fps: info.video.as_ref().and_then(|v| v.frame_rate),
        })
    }
}

/// the clip's length after trimming and speed, and the boomerang's way back
pub fn output_seconds(source: &AnimSource, options: &AnimOptions) -> Option<f64> {
    let start = options.start.unwrap_or(0.0).max(0.0);
    let end = options.end.or(source.duration)?;
    let length = (end - start).max(0.0) / options.speed();
    Some(if options.boomerang {
        length * 2.0
    } else {
        length
    })
}

/// the picture size of the result, before caption bars
pub fn frame_size(source: &AnimSource, options: &AnimOptions) -> (u32, u32) {
    let crop = clamp_crop(source, options.crop);
    let (cw, ch) = (crop.width, crop.height);
    let width = options.width.filter(|w| *w >= 2).unwrap_or(cw).min(cw);
    let height = (f64::from(ch) * f64::from(width) / f64::from(cw)).round();
    let height = (height as u32).max(1);
    if options.format.even_size() {
        (width.max(2) & !1, height.max(2) & !1)
    } else {
        (width, height)
    }
}

fn clamp_crop(source: &AnimSource, crop: Option<Crop>) -> Crop {
    let full = Crop {
        x: 0,
        y: 0,
        width: source.width,
        height: source.height,
    };
    let Some(crop) = crop else { return full };
    let x = crop.x.min(source.width.saturating_sub(2));
    let y = crop.y.min(source.height.saturating_sub(2));
    let width = crop.width.clamp(2, source.width - x);
    let height = crop.height.clamp(2, source.height - y);
    Crop {
        x,
        y,
        width,
        height,
    }
}

/// the most memory reverse and boomerang may hold, since they keep every frame
const REVERSE_BYTES: f64 = 1.5e9;

/// makes the animation at `output` (via a partial file, so a failed run leaves nothing)
pub fn render(
    env: &Env,
    input: &Path,
    source: &AnimSource,
    options: &AnimOptions,
    output: &Path,
    control: &dyn Control,
) -> MediaResult<()> {
    let ffmpeg = env.ffmpeg()?;
    let temp = tempfile::tempdir().map_err(|err| MediaError::io(std::env::temp_dir(), err))?;
    let (width, height) = frame_size(source, options);
    let seconds = output_seconds(source, options);

    if options.reverse || options.boomerang {
        let fps = options.fps().or(source.fps).unwrap_or(30.0);
        let frames = seconds.unwrap_or(60.0) * fps;
        let bytes = frames * f64::from(width) * f64::from(height) * 4.0;
        if bytes > REVERSE_BYTES {
            let fits = REVERSE_BYTES / (fps * f64::from(width) * f64::from(height) * 4.0);
            return Err(MediaError::Unsupported(format!(
                "Playing backwards keeps every frame in memory, which is too much for this clip. Trim it to under {:.0} seconds or make it smaller.",
                fits.floor().max(1.0)
            )));
        }
    }

    let mut args: Vec<String> = Vec::new();
    if let Some(start) = options.start.filter(|s| *s > 0.0) {
        args.extend(["-ss".into(), num(start)]);
    }
    if let Some(end) = options.end {
        args.extend(["-to".into(), num(end)]);
    }
    args.extend(["-i".into(), input.display().to_string()]);

    let mut chain: Vec<String> = Vec::new();
    let speed = options.speed();
    if (speed - 1.0).abs() > 1e-6 {
        chain.push(format!("setpts=(PTS-STARTPTS)/{}", num(speed)));
    }
    if let Some(fps) = options.fps() {
        chain.push(format!("fps={}", num(fps)));
    }
    if options.crop.is_some() {
        let c = clamp_crop(source, options.crop);
        chain.push(format!("crop={}:{}:{}:{}", c.width, c.height, c.x, c.y));
    }
    chain.push(format!("scale={width}:{height}:flags=lanczos"));
    let mut graph = format!("[0:v]{}[base]", chain.join(","));
    let mut last = "base".to_owned();

    if !options.caption.is_empty() {
        let layer = caption::render(&options.caption, width, height)?;
        let file = temp.path().join("caption.png");
        layer.image.save(&file).map_err(|err| {
            MediaError::Unsupported(format!("The caption could not be saved: {err}"))
        })?;
        args.extend(["-i".into(), file.display().to_string()]);
        if layer.pad_top + layer.pad_bottom > 0 {
            graph.push_str(&format!(
                ";[{last}]pad={width}:{}:0:{}:white[padded]",
                height + layer.pad_top + layer.pad_bottom,
                layer.pad_top
            ));
            last = "padded".into();
        }
        graph.push_str(&format!(";[{last}][1:v]overlay=0:0[captioned]"));
        last = "captioned".into();
    }

    if options.boomerang {
        // the way back skips the frame at the turn so it does not show twice
        graph.push_str(&format!(
            ";[{last}]split[fwd][back];[back]reverse,trim=start_frame=1,setpts=PTS-STARTPTS[rev];[fwd][rev]concat=n=2:v=1:a=0[boom]"
        ));
        last = "boom".into();
    } else if options.reverse {
        graph.push_str(&format!(";[{last}]reverse[rev]"));
        last = "rev".into();
    }

    let quality = options.quality.min(100);
    let mut codec: Vec<String> = Vec::new();
    match options.format {
        AnimFormat::Gif => {
            let colors = options.colors.clamp(2, 256);
            graph.push_str(&format!(
                ";[{last}]split[s0][s1];[s0]palettegen=max_colors={colors}:stats_mode=diff[pal];[s1][pal]paletteuse=dither={}:diff_mode=rectangle[out]",
                options.dither.filter()
            ));
            // gif counts repeats after the first play: -1 plays once, 0 forever
            let repeats = match options.loops {
                0 => 0,
                1 => -1,
                n => i64::from(n) - 1,
            };
            codec.extend([
                "-loop".into(),
                repeats.to_string(),
                "-f".into(),
                "gif".into(),
            ]);
        }
        AnimFormat::Webp => {
            graph.push_str(&format!(";[{last}]null[out]"));
            codec.extend([
                "-c:v".into(),
                "libwebp_anim".into(),
                "-lossless".into(),
                "0".into(),
                "-quality".into(),
                quality.to_string(),
                "-loop".into(),
                options.loops.to_string(),
                "-f".into(),
                "webp".into(),
            ]);
        }
        AnimFormat::Apng => {
            graph.push_str(&format!(";[{last}]format=rgba[out]"));
            codec.extend([
                "-c:v".into(),
                "apng".into(),
                "-pred".into(),
                "mixed".into(),
                "-plays".into(),
                options.loops.to_string(),
                "-f".into(),
                "apng".into(),
            ]);
        }
        AnimFormat::Avif => {
            graph.push_str(&format!(";[{last}]format=yuv420p[out]"));
            let encoder = crate::avif_encoder(env)?;
            // quality 100 is crf 10, quality 0 is crf 60
            let crf = 60 - quality / 2;
            codec.extend([
                "-c:v".into(),
                encoder.into(),
                "-crf".into(),
                crf.to_string(),
            ]);
            if encoder == "libaom-av1" {
                codec.extend(["-cpu-used".into(), "6".into(), "-row-mt".into(), "1".into()]);
            } else {
                codec.extend(["-preset".into(), "8".into()]);
            }
            codec.extend(["-f".into(), "avif".into()]);
        }
        AnimFormat::Mp4 => {
            graph.push_str(&format!(";[{last}]format=yuv420p[out]"));
            // quality 100 is crf 16, quality 0 is crf 40
            let crf = 40 - quality * 24 / 100;
            let h264 = env
                .hardware
                .software(vellum_media::ffmpeg::hw::VideoCodec::H264)
                .map_or("libx264", |e| e.name.as_str());
            codec.extend(["-c:v".into(), h264.into()]);
            if h264 == "libx264" {
                codec.extend([
                    "-crf".into(),
                    crf.to_string(),
                    "-preset".into(),
                    "medium".into(),
                ]);
            } else {
                codec.extend(["-b:v".into(), format!("{}k", 200 + quality * 20)]);
            }
            codec.extend([
                "-movflags".into(),
                "+faststart".into(),
                "-f".into(),
                "mp4".into(),
            ]);
        }
    }
    args.extend([
        "-filter_complex".into(),
        graph,
        "-map".into(),
        "[out]".into(),
        "-an".into(),
        "-map_metadata".into(),
        "-1".into(),
    ]);
    args.extend(codec);

    let partial = partial_path(output);
    args.extend(output_args(&partial));
    let lossy = options.format == AnimFormat::Gif && options.lossy > 0;
    let encode_share = if lossy { 0.85 } else { 1.0 };
    let span = vellum_media::control::Span {
        inner: control,
        start: 0.0,
        end: encode_share,
    };
    if let Err(err) = run_ffmpeg(ffmpeg, &args, seconds, &span) {
        let _ = std::fs::remove_file(&partial);
        return Err(err);
    }
    if lossy {
        control.message("Compressing the GIF");
        let data = std::fs::read(&partial).map_err(|err| MediaError::io(&partial, err))?;
        let squeezed = lossy::compress(&data, options.lossy, control);
        let _ = std::fs::remove_file(&partial);
        crate::write_atomic(output, &squeezed?)?;
    } else {
        crate::finish_partial(&partial, output)?;
    }
    control.progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> AnimSource {
        AnimSource {
            width: 1920,
            height: 1080,
            duration: Some(10.0),
            fps: Some(30.0),
        }
    }

    #[test]
    fn sizes_follow_crop_and_width_and_never_upscale() {
        let mut options = AnimOptions::default();
        assert_eq!(frame_size(&source(), &options), (480, 270));
        options.crop = Some(Crop {
            x: 100,
            y: 100,
            width: 400,
            height: 400,
        });
        assert_eq!(frame_size(&source(), &options), (400, 400));
        options.width = Some(201);
        options.format = AnimFormat::Mp4;
        assert_eq!(frame_size(&source(), &options), (200, 200));
        // a crop hanging off the edge is pulled back inside
        options.crop = Some(Crop {
            x: 1900,
            y: 0,
            width: 400,
            height: 100,
        });
        options.width = None;
        let (w, _) = frame_size(&source(), &options);
        assert!(w <= 20);
    }

    #[test]
    fn length_counts_trim_speed_and_boomerang() {
        let mut options = AnimOptions {
            start: Some(2.0),
            end: Some(6.0),
            speed: 2.0,
            ..AnimOptions::default()
        };
        assert_eq!(output_seconds(&source(), &options), Some(2.0));
        options.boomerang = true;
        assert_eq!(output_seconds(&source(), &options), Some(4.0));
        options.speed = f64::NAN;
        options.end = None;
        assert_eq!(output_seconds(&source(), &options), Some(16.0));
    }
}
