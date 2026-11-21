//! one way in for every still and animated image format.
//!
//! | format | decoded by |
//! |---|---|
//! | png, apng, jpeg, gif, webp, tiff, bmp, ico, qoi, tga, pnm, hdr, exr | the `image` crate |
//! | svg | resvg, at any size |
//! | psd | the `psd` crate, flattened |
//! | camera raw | rawler (full develop, or the embedded preview for speed) |
//! | avif, jpeg xl | ffmpeg |
//! | heic | libheif's `heif-dec`, then ffmpeg 7.1 or newer, then `sips` on macos |
//!
//! very large images are refused with a clear message before any pixels are allocated,
//! using the size from the file header. tiled editing of huge files is phase 5's job.

use std::fs;
use std::io::Cursor;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::Orientation;
use image::{
    AnimationDecoder, DynamicImage, ExtendedColorType, ImageDecoder, ImageReader, Limits, RgbaImage,
};

use crate::animation;
use crate::error::{MediaError, MediaResult};
use crate::exif_tags::{self, ExifSummary};
use crate::ffmpeg::{FfmpegTools, FrameRequest, extract_frame, find_program, probe_media};
use crate::info::{ImageInfo, MediaInfo, MediaKind, oriented_size};
use crate::sniff::Format;

/// the default ceiling on memory for one decoded image: a 23000 x 23000 rgba picture
pub const DEFAULT_MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecodeOptions {
    /// shrink to fit inside this box, keeping the shape. svgs are drawn at this size.
    pub max_size: Option<(u32, u32)>,
    /// every frame of an animation, or only the first
    pub all_frames: bool,
    /// turn photos upright using their exif orientation
    pub apply_orientation: bool,
    /// size multiplier for svgs drawn without `max_size`
    pub svg_scale: f32,
    /// use the small preview inside camera raw files instead of developing the raw data
    pub fast_preview: bool,
    /// refuse images that would need more memory than this
    pub max_bytes: u64,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_size: None,
            all_frames: false,
            apply_orientation: true,
            svg_scale: 1.0,
            fast_preview: false,
            max_bytes: DEFAULT_MAX_IMAGE_BYTES,
        }
    }
}

impl DecodeOptions {
    /// quick and small, for thumbnails and previews
    pub fn preview(max_side: u32) -> Self {
        Self {
            max_size: Some((max_side, max_side)),
            fast_preview: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub image: DynamicImage,
    pub delay_ms: u32,
}

#[derive(Debug, Clone)]
pub struct DecodedImage {
    /// at least one
    pub frames: Vec<DecodedFrame>,
    /// the size stored in the file, before turning upright and shrinking
    pub source_width: u32,
    pub source_height: u32,
    /// 0 plays forever
    pub loop_count: Option<u32>,
    pub icc_profile: Option<Vec<u8>>,
}

impl DecodedImage {
    fn still(image: DynamicImage) -> Self {
        Self {
            source_width: image.width(),
            source_height: image.height(),
            frames: vec![DecodedFrame { image, delay_ms: 0 }],
            loop_count: None,
            icc_profile: None,
        }
    }

    pub fn first(&self) -> &DynamicImage {
        // decoders never return zero frames, see `finish`
        &self.frames[0].image
    }

    pub fn width(&self) -> u32 {
        self.first().width()
    }

    pub fn height(&self) -> u32 {
        self.first().height()
    }
}

/// decodes any image vellum knows. `tools` is needed for avif, jpeg xl and some heic.
pub fn decode_image(
    path: &Path,
    format: Format,
    opts: &DecodeOptions,
    tools: Option<&FfmpegTools>,
) -> MediaResult<DecodedImage> {
    let decoded = match format {
        Format::Svg => decode_svg(path, opts)?,
        Format::Psd => guarded("Photoshop document", || decode_psd(path))?,
        Format::Raw => guarded("camera raw photo", || decode_raw(path, opts))?,
        Format::Avif | Format::Jxl => DecodedImage::still(decode_with_ffmpeg(path, tools)?),
        Format::Heif => DecodedImage::still(decode_heif(path, tools)?),
        Format::Media => {
            return Err(MediaError::Unsupported(
                "This is not an image Vellum can open.".into(),
            ));
        }
        raster => guarded("image", || decode_raster(path, raster, opts))?,
    };
    Ok(finish(decoded, opts))
}

/// shrinks to `max_size`. orientation was applied by each decoder.
fn finish(mut decoded: DecodedImage, opts: &DecodeOptions) -> DecodedImage {
    if let Some((max_w, max_h)) = opts.max_size {
        for frame in &mut decoded.frames {
            let image = &frame.image;
            if image.width() > max_w || image.height() > max_h {
                // area averaging, quick and good for large reductions
                frame.image = image.thumbnail(max_w.max(1), max_h.max(1));
            }
        }
    }
    decoded
}

/// third party decoders can panic on hostile files. that must never take the app down.
fn guarded<T>(what: &str, decode: impl FnOnce() -> MediaResult<T>) -> MediaResult<T> {
    panic::catch_unwind(AssertUnwindSafe(decode)).unwrap_or_else(|_| {
        Err(MediaError::decode(format!(
            "This {what} is damaged or uses a feature Vellum cannot read yet."
        )))
    })
}

fn read_file(path: &Path) -> MediaResult<Vec<u8>> {
    fs::read(path).map_err(|err| MediaError::io(path, err))
}

fn check_memory(width: u32, height: u32, bytes_per_pixel: u64, max: u64) -> MediaResult<()> {
    let needed = u64::from(width) * u64::from(height) * bytes_per_pixel;
    if needed <= max {
        return Ok(());
    }
    Err(MediaError::Unsupported(format!(
        "This image is {width} x {height} pixels and needs {:.1} GB of memory to open, more \
         than the {:.1} GB Vellum allows. Raise the limit in Settings > Media, or shrink the \
         image first.",
        needed as f64 / 1e9,
        max as f64 / 1e9
    )))
}

fn limits(opts: &DecodeOptions) -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(opts.max_bytes);
    limits
}

fn decode_raster(path: &Path, format: Format, opts: &DecodeOptions) -> MediaResult<DecodedImage> {
    let bytes = read_file(path)?;
    let image_format = format
        .image_crate_format()
        .ok_or_else(|| MediaError::decode("not a raster format"))?;
    let bad = |err: image::ImageError| match err {
        image::ImageError::Limits(_) => MediaError::Unsupported(
            "This image needs more memory than Vellum allows. Raise the limit in Settings > \
             Media."
                .into(),
        ),
        other => MediaError::decode(format!("This image could not be read: {other}")),
    };

    if opts.all_frames
        && let Some(decoded) = decode_animation(&bytes, format).map_err(bad)?
    {
        return Ok(decoded);
    }

    let mut reader = ImageReader::with_format(Cursor::new(&bytes), image_format);
    reader.limits(limits(opts));
    let mut decoder = reader.into_decoder().map_err(bad)?;
    let (width, height) = decoder.dimensions();
    let pixel_bytes = u64::from(decoder.color_type().bytes_per_pixel());
    check_memory(width, height, pixel_bytes, opts.max_bytes)?;
    let icc_profile = decoder.icc_profile().ok().flatten();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(bad)?;
    if opts.apply_orientation {
        image.apply_orientation(orientation);
    }
    let loop_count = animation::read(format, &bytes).map(|a| a.loop_count);
    Ok(DecodedImage {
        frames: vec![DecodedFrame { image, delay_ms: 0 }],
        source_width: width,
        source_height: height,
        loop_count,
        icc_profile,
    })
}

/// every frame of a gif, animated webp or apng. none for stills.
fn decode_animation(bytes: &[u8], format: Format) -> image::ImageResult<Option<DecodedImage>> {
    let Some(info) = animation::read(format, bytes) else {
        return Ok(None);
    };
    let frames = match format {
        Format::Gif => GifDecoder::new(Cursor::new(bytes))?.into_frames(),
        Format::WebP => WebPDecoder::new(Cursor::new(bytes))?.into_frames(),
        Format::Png => PngDecoder::new(Cursor::new(bytes))?.apng()?.into_frames(),
        _ => return Ok(None),
    };
    let frames = frames.collect_frames()?;
    let Some(first) = frames.first() else {
        return Ok(None);
    };
    let (source_width, source_height) = first.buffer().dimensions();
    let frames = frames
        .into_iter()
        .map(|frame| {
            let (num, den) = frame.delay().numer_denom_ms();
            DecodedFrame {
                delay_ms: num.checked_div(den).unwrap_or(0),
                image: DynamicImage::ImageRgba8(frame.into_buffer()),
            }
        })
        .collect();
    Ok(Some(DecodedImage {
        frames,
        source_width,
        source_height,
        loop_count: Some(info.loop_count),
        icc_profile: None,
    }))
}

/// system fonts are loaded once, it takes a moment
fn fonts() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

fn svg_tree(path: &Path) -> MediaResult<resvg::usvg::Tree> {
    let data = read_file(path)?;
    let options = resvg::usvg::Options {
        resources_dir: path.parent().map(Path::to_owned),
        fontdb: fonts(),
        ..Default::default()
    };
    resvg::usvg::Tree::from_data(&data, &options)
        .map_err(|err| MediaError::decode(format!("This SVG could not be read: {err}")))
}

fn decode_svg(path: &Path, opts: &DecodeOptions) -> MediaResult<DecodedImage> {
    let tree = svg_tree(path)?;
    let size = tree.size();
    let (natural_w, natural_h) = (size.width().max(1.0), size.height().max(1.0));
    // vector art draws sharp at any size, so it is scaled up to fill the box too
    let scale = match opts.max_size {
        Some((max_w, max_h)) => (max_w as f32 / natural_w).min(max_h as f32 / natural_h),
        None => opts.svg_scale.max(0.01),
    };
    let width = (natural_w * scale).round().max(1.0) as u32;
    let height = (natural_h * scale).round().max(1.0) as u32;
    check_memory(width, height, 4, opts.max_bytes)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(|| {
        MediaError::Unsupported(format!(
            "An SVG cannot be drawn at {width} x {height} pixels."
        ))
    })?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / natural_w,
        height as f32 / natural_h,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia keeps premultiplied alpha, images everywhere else are straight
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let image = RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| MediaError::decode("the SVG drew to the wrong size"))?;
    let mut decoded = DecodedImage::still(DynamicImage::ImageRgba8(image));
    decoded.source_width = natural_w.round() as u32;
    decoded.source_height = natural_h.round() as u32;
    Ok(decoded)
}

fn decode_psd(path: &Path) -> MediaResult<DecodedImage> {
    let bytes = read_file(path)?;
    let psd = psd::Psd::from_bytes(&bytes).map_err(|err| {
        MediaError::decode(format!("This Photoshop document could not be read: {err}"))
    })?;
    let (width, height) = (psd.width(), psd.height());
    let mut rgba = psd.rgba();
    // files saved without "maximise compatibility" have an empty composite, so build one
    // from the layers instead
    if rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 0) && !psd.layers().is_empty() {
        rgba = psd
            .flatten_layers_rgba(&|(_, layer)| layer.visible())
            .map_err(|err| {
                MediaError::decode(format!("The layers could not be combined: {err}"))
            })?;
    }
    let image = RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| MediaError::decode("The Photoshop document has the wrong size."))?;
    Ok(DecodedImage::still(DynamicImage::ImageRgba8(image)))
}

fn decode_raw(path: &Path, opts: &DecodeOptions) -> MediaResult<DecodedImage> {
    let params = rawler::decoders::RawDecodeParams::default();
    let developed = if opts.fast_preview {
        rawler::analyze::extract_preview_pixels(path, &params)
    } else {
        rawler::analyze::raw_to_srgb(path, &params)
    };
    let mut image = developed.map_err(|err| {
        MediaError::Unsupported(format!(
            "This camera raw photo could not be read ({err}). The camera may be too new for \
             Vellum, try converting it to DNG first."
        ))
    })?;
    let (source_width, source_height) = (image.width(), image.height());
    if opts.apply_orientation {
        let orientation = read_file(path)
            .ok()
            .and_then(|bytes| exif_tags::from_file_bytes(&bytes))
            .map_or(1, |summary| summary.orientation);
        if let Some(o) = Orientation::from_exif(orientation as u8) {
            image.apply_orientation(o);
        }
    }
    let mut decoded = DecodedImage::still(image);
    decoded.source_width = source_width;
    decoded.source_height = source_height;
    Ok(decoded)
}

fn decode_with_ffmpeg(path: &Path, tools: Option<&FfmpegTools>) -> MediaResult<DynamicImage> {
    let tools = tools.ok_or(MediaError::FfmpegMissing)?;
    let info = probe_media(tools, path)?;
    let video = info
        .video
        .as_ref()
        .ok_or_else(|| MediaError::Unsupported("FFmpeg found no picture in this file.".into()))?;
    let req = FrameRequest {
        stream: video.stream_index,
        seek: 0.0,
        accurate: true,
        width: video.display_width,
        height: video.display_height,
    };
    Ok(DynamicImage::ImageRgba8(extract_frame(tools, path, &req)?))
}

fn decode_heif(path: &Path, tools: Option<&FfmpegTools>) -> MediaResult<DynamicImage> {
    let mut reasons = Vec::new();
    for name in ["heif-dec", "heif-convert"] {
        let Some(program) = find_program(name) else {
            continue;
        };
        match convert_with(&program, path, |cmd, out| {
            cmd.arg(path).arg(out);
        }) {
            Ok(image) => return Ok(image),
            Err(err) => reasons.push(err.to_string()),
        }
    }
    if let Some(tools) = tools {
        match decode_with_ffmpeg(path, Some(tools)) {
            Ok(image) => return Ok(image),
            Err(err) => reasons.push(err.to_string()),
        }
    }
    if cfg!(target_os = "macos") {
        match convert_with(Path::new("/usr/bin/sips"), path, |cmd, out| {
            cmd.args(["-s", "format", "png"])
                .arg(path)
                .arg("--out")
                .arg(out);
        }) {
            Ok(image) => return Ok(image),
            Err(err) => reasons.push(err.to_string()),
        }
    }
    tracing::debug!(?reasons, "heic decoders failed");
    Err(MediaError::Unsupported(
        "HEIC photos need libheif or FFmpeg 7.1 or newer, and neither could open this one. \
         Install one of them, or convert the photo to JPEG first."
            .into(),
    ))
}

/// runs a converter that writes a png, then reads the png back
fn convert_with(
    program: &Path,
    input: &Path,
    args: impl FnOnce(&mut std::process::Command, &Path),
) -> MediaResult<DynamicImage> {
    let out = TempPath::new("png");
    let mut cmd = crate::ffmpeg::command(program);
    args(&mut cmd, &out.0);
    let name = program
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("converter")
        .to_owned();
    crate::ffmpeg::run_output(&mut cmd, &name)?;
    let bytes = read_file(&out.0)?;
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).map_err(|err| {
        MediaError::decode(format!(
            "{name} wrote an unreadable file: {err} ({})",
            input.display()
        ))
    })
}

/// a temp file path that is deleted when dropped
struct TempPath(PathBuf);

impl TempPath {
    fn new(ext: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("vellum-{}-{n}.{ext}", std::process::id());
        Self(std::env::temp_dir().join(name))
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// files larger than this are not read whole just to find their metadata
const PROBE_READ_LIMIT: u64 = 256 * 1024 * 1024;

/// describes an image without decoding its pixels where the format allows
pub fn probe_image(
    path: &Path,
    format: Format,
    tools: Option<&FfmpegTools>,
) -> MediaResult<MediaInfo> {
    let size_bytes = fs::metadata(path)
        .map_err(|err| MediaError::io(path, err))?
        .len();
    let mut info = MediaInfo {
        path: path.display().to_string(),
        kind: MediaKind::Image,
        format: format.id().to_owned(),
        format_label: format.label().to_owned(),
        size_bytes,
        duration_secs: None,
        bit_rate: None,
        video: None,
        audio: None,
        image: None,
        streams: Vec::new(),
        tags: Vec::new(),
        warnings: Vec::new(),
    };
    let image = match format {
        Format::Svg => probe_svg(path)?,
        Format::Psd => probe_psd(path)?,
        Format::Raw => guarded("camera raw photo", || probe_raw(path, &mut info))?,
        Format::Avif | Format::Jxl | Format::Heif => {
            probe_external(path, format, tools, &mut info)?
        }
        Format::Media => {
            return Err(MediaError::Unsupported("This is not an image.".into()));
        }
        raster => guarded("image", || {
            probe_raster(path, raster, size_bytes, &mut info)
        })?,
    };
    info.image = Some(image);
    Ok(info)
}

fn image_info(width: u32, height: u32, orientation: u32) -> ImageInfo {
    let (display_width, display_height) = oriented_size(width, height, orientation);
    ImageInfo {
        width,
        height,
        display_width,
        display_height,
        frames: 1,
        loop_count: None,
        has_alpha: false,
        bit_depth: Some(8),
        color_model: Some("RGB".into()),
        has_icc_profile: false,
        orientation,
        dpi: None,
    }
}

fn probe_raster(
    path: &Path,
    format: Format,
    size_bytes: u64,
    info: &mut MediaInfo,
) -> MediaResult<ImageInfo> {
    let bytes = if size_bytes <= PROBE_READ_LIMIT {
        read_file(path)?
    } else {
        let mut head = vec![0u8; 16 * 1024 * 1024];
        let mut file = fs::File::open(path).map_err(|err| MediaError::io(path, err))?;
        let n = crate::ffmpeg::read_full(&mut file, &mut head)
            .map_err(|err| MediaError::io(path, err))?;
        head.truncate(n);
        head
    };
    let image_format = format
        .image_crate_format()
        .ok_or_else(|| MediaError::decode("not a raster format"))?;
    let mut decoder = ImageReader::with_format(Cursor::new(&bytes), image_format)
        .into_decoder()
        .map_err(|err| MediaError::decode(format!("This image could not be read: {err}")))?;
    let (width, height) = decoder.dimensions();
    let color = decoder.original_color_type();
    let has_icc = decoder.icc_profile().ok().flatten().is_some();
    let exif = decoder
        .exif_metadata()
        .ok()
        .flatten()
        .and_then(|block| exif_tags::from_raw_block(&block))
        .or_else(|| exif_tags::from_file_bytes(&bytes))
        .unwrap_or_default();
    let mut image = image_info(width, height, exif.orientation);
    image.has_alpha = decoder.color_type().has_alpha();
    image.bit_depth = color_depth(color);
    image.color_model = Some(color_model(color).to_owned());
    image.has_icc_profile = has_icc;
    image.dpi = exif.dpi;
    if let Some(anim) = animation::read(format, &bytes) {
        info.kind = MediaKind::Animation;
        info.duration_secs = Some(anim.duration_ms as f64 / 1000.0);
        image.frames = anim.frames;
        image.loop_count = Some(anim.loop_count);
    }
    info.tags = exif.tags;
    Ok(image)
}

fn color_depth(color: ExtendedColorType) -> Option<u32> {
    let channels = u32::from(color.channel_count());
    (channels > 0).then(|| u32::from(color.bits_per_pixel()) / channels)
}

fn color_model(color: ExtendedColorType) -> &'static str {
    use ExtendedColorType as C;
    match color {
        C::A8 => "Alpha",
        C::L1 | C::L2 | C::L4 | C::L8 | C::L16 | C::La1 | C::La2 | C::La4 | C::La8 | C::La16 => {
            "Grayscale"
        }
        C::Cmyk8 | C::Cmyk16 => "CMYK",
        C::Unknown(_) => "Indexed",
        _ => "RGB",
    }
}

fn probe_svg(path: &Path) -> MediaResult<ImageInfo> {
    let tree = svg_tree(path)?;
    let size = tree.size();
    let mut image = image_info(size.width().round() as u32, size.height().round() as u32, 1);
    image.has_alpha = true;
    image.bit_depth = None;
    image.color_model = Some("Vector".into());
    Ok(image)
}

/// the psd header is 26 bytes: signature, version, reserved, channels, height, width,
/// depth and colour mode
fn probe_psd(path: &Path) -> MediaResult<ImageInfo> {
    let mut head = [0u8; 26];
    let mut file = fs::File::open(path).map_err(|err| MediaError::io(path, err))?;
    let n =
        crate::ffmpeg::read_full(&mut file, &mut head).map_err(|err| MediaError::io(path, err))?;
    if n < 26 || &head[0..4] != b"8BPS" {
        return Err(MediaError::decode("This Photoshop document is damaged."));
    }
    let u16_at = |at: usize| u16::from_be_bytes([head[at], head[at + 1]]);
    let u32_at =
        |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    let channels = u16_at(12);
    let mut image = image_info(u32_at(18), u32_at(14), 1);
    image.bit_depth = Some(u32::from(u16_at(22)));
    image.color_model = Some(
        match u16_at(24) {
            0 => "Bitmap",
            1 => "Grayscale",
            2 => "Indexed",
            3 => "RGB",
            4 => "CMYK",
            7 => "Multichannel",
            8 => "Duotone",
            9 => "Lab",
            _ => "Unknown",
        }
        .to_owned(),
    );
    image.has_alpha = channels > 3;
    Ok(image)
}

fn probe_raw(path: &Path, info: &mut MediaInfo) -> MediaResult<ImageInfo> {
    let source = rawler::rawsource::RawSource::new(path).map_err(|err| {
        MediaError::decode(format!("This camera raw photo could not be read: {err}"))
    })?;
    let decoder = rawler::get_decoder(&source).map_err(|err| {
        MediaError::Unsupported(format!(
            "This camera raw photo is not supported ({err}). Try converting it to DNG first."
        ))
    })?;
    let params = rawler::decoders::RawDecodeParams::default();
    // a dummy decode reads the headers but no pixels
    let raw = decoder.raw_image(&source, &params, true).map_err(|err| {
        MediaError::decode(format!("This camera raw photo could not be read: {err}"))
    })?;
    let (width, height) = raw
        .crop_area
        .map_or((raw.width, raw.height), |crop| (crop.d.w, crop.d.h));
    let exif: ExifSummary = read_file(path)
        .ok()
        .and_then(|bytes| exif_tags::from_file_bytes(&bytes))
        .unwrap_or_default();
    let orientation = match raw.orientation.to_u16() {
        o @ 1..=8 => u32::from(o),
        _ => exif.orientation,
    };
    let mut image = image_info(
        u32::try_from(width).unwrap_or(0),
        u32::try_from(height).unwrap_or(0),
        orientation,
    );
    image.bit_depth = u32::try_from(raw.bps).ok();
    image.color_model = Some(if raw.cpp == 1 { "Bayer sensor" } else { "RGB" }.to_owned());
    image.dpi = exif.dpi;
    info.format_label =
        format!("Camera raw photo ({} {})", raw.clean_make, raw.clean_model).replace(" ()", "");
    info.tags = exif.tags;
    Ok(image)
}

fn probe_external(
    path: &Path,
    format: Format,
    tools: Option<&FfmpegTools>,
    info: &mut MediaInfo,
) -> MediaResult<ImageInfo> {
    let exif = read_file(path)
        .ok()
        .and_then(|bytes| exif_tags::from_file_bytes(&bytes))
        .unwrap_or_default();
    info.tags = exif.tags.clone();
    if let Some(tools) = tools
        && let Ok(probed) = probe_media(tools, path)
        && let Some(video) = probed.video
    {
        // heif turns pictures upright with its own boxes, which ffmpeg applies as rotation
        let mut image = image_info(video.width, video.height, 1);
        image.display_width = video.display_width;
        image.display_height = video.display_height;
        image.bit_depth = video.bit_depth;
        image.has_alpha = video
            .pixel_format
            .as_deref()
            .is_some_and(|p| p.contains('a'));
        return Ok(image);
    }
    if format != Format::Heif {
        return Err(tools.map_or(MediaError::FfmpegMissing, |_| {
            MediaError::Unsupported("FFmpeg could not read this image.".into())
        }));
    }
    // older ffmpeg cannot read heic at all, so decode it to learn its size
    let decoded = decode_heif(path, tools)?;
    let mut image = image_info(decoded.width(), decoded.height(), 1);
    image.has_alpha = decoded.color().has_alpha();
    Ok(image)
}

#[cfg(test)]
mod tests {
    use image::{ImageEncoder, Rgb, RgbImage, Rgba};

    use super::*;
    use crate::exif_tags::tests::exif_block;
    use crate::ffmpeg::testing;
    use crate::sniff::sniff;

    fn gradient(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            Rgba([
                (x * 255 / width.max(1)) as u8,
                (y * 255 / height.max(1)) as u8,
                90,
                255,
            ])
        })
    }

    fn save(dir: &Path, name: &str, image: &DynamicImage) -> PathBuf {
        let path = dir.join(name);
        image.save(&path).unwrap();
        path
    }

    #[test]
    fn every_raster_format_decodes_through_one_call() {
        let dir = tempfile::tempdir().unwrap();
        let rgba = DynamicImage::ImageRgba8(gradient(40, 30));
        let rgb = DynamicImage::ImageRgb8(rgba.to_rgb8());
        let cases = [
            ("a.png", &rgba, Format::Png),
            ("a.jpg", &rgb, Format::Jpeg),
            ("a.webp", &rgba, Format::WebP),
            ("a.gif", &rgba, Format::Gif),
            ("a.tiff", &rgba, Format::Tiff),
            ("a.bmp", &rgba, Format::Bmp),
            ("a.qoi", &rgba, Format::Qoi),
            ("a.tga", &rgba, Format::Tga),
            ("a.ico", &rgba, Format::Ico),
        ];
        for (name, image, format) in cases {
            let path = save(dir.path(), name, image);
            assert_eq!(sniff(&path).unwrap(), format, "{name}");
            let decoded = decode_image(&path, format, &DecodeOptions::default(), None).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (40, 30), "{name}");
            let info = probe_image(&path, format, None).unwrap();
            let image = info.image.unwrap();
            assert_eq!((image.width, image.height), (40, 30), "{name}");
        }
    }

    #[test]
    fn previews_shrink_to_fit_and_keep_the_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = save(
            dir.path(),
            "big.png",
            &DynamicImage::ImageRgba8(gradient(400, 100)),
        );
        let decoded = decode_image(&path, Format::Png, &DecodeOptions::preview(64), None).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (64, 16));
        assert_eq!((decoded.source_width, decoded.source_height), (400, 100));
    }

    #[test]
    fn too_big_images_are_refused_before_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = save(
            dir.path(),
            "a.png",
            &DynamicImage::ImageRgba8(gradient(100, 100)),
        );
        let opts = DecodeOptions {
            max_bytes: 1000,
            ..DecodeOptions::default()
        };
        let err = decode_image(&path, Format::Png, &opts, None).unwrap_err();
        assert!(err.to_string().contains("100 x 100 pixels"), "{err}");
    }

    #[test]
    fn jpeg_orientation_turns_the_photo_upright() {
        let dir = tempfile::tempdir().unwrap();
        // a 40 x 20 photo stored sideways with orientation 6 (turn 90 degrees clockwise)
        let mut jpeg = Vec::new();
        let rgb = DynamicImage::ImageRgba8(gradient(40, 20)).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .write_image(rgb.as_raw(), 40, 20, ExtendedColorType::Rgb8)
            .unwrap();
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(exif_block(&[(0x0112, 3, 1, 6)]));
        let mut with_exif = jpeg[..2].to_vec();
        with_exif.extend_from_slice(&[0xff, 0xe1]);
        with_exif.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        with_exif.extend_from_slice(&app1);
        with_exif.extend_from_slice(&jpeg[2..]);
        let path = dir.path().join("sideways.jpg");
        fs::write(&path, &with_exif).unwrap();

        let decoded = decode_image(&path, Format::Jpeg, &DecodeOptions::default(), None).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (20, 40));
        let image = probe_image(&path, Format::Jpeg, None)
            .unwrap()
            .image
            .unwrap();
        assert_eq!(image.orientation, 6);
        assert_eq!((image.width, image.height), (40, 20));
        assert_eq!((image.display_width, image.display_height), (20, 40));
    }

    #[test]
    fn animated_gifs_keep_every_frame_when_asked() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("anim.gif");
        {
            let file = fs::File::create(&path).unwrap();
            let mut encoder = GifEncoder::new(file);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for i in 0..4u8 {
                let frame = RgbaImage::from_pixel(10, 10, Rgba([i * 60, 0, 0, 255]));
                let delay = image::Delay::from_numer_denom_ms(50, 1);
                encoder
                    .encode_frame(image::Frame::from_parts(frame, 0, 0, delay))
                    .unwrap();
            }
        }
        let opts = DecodeOptions {
            all_frames: true,
            ..DecodeOptions::default()
        };
        let decoded = decode_image(&path, Format::Gif, &opts, None).unwrap();
        assert_eq!(decoded.frames.len(), 4);
        assert_eq!(decoded.frames[1].delay_ms, 50);
        assert_eq!(decoded.loop_count, Some(0));
        let still = decode_image(&path, Format::Gif, &DecodeOptions::default(), None).unwrap();
        assert_eq!(still.frames.len(), 1);
        let info = probe_image(&path, Format::Gif, None).unwrap();
        assert_eq!(info.kind, MediaKind::Animation);
        assert_eq!(info.image.unwrap().frames, 4);
        assert_eq!(info.duration_secs, Some(0.2));
    }

    #[test]
    fn svgs_draw_at_any_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logo.svg");
        fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
                 <rect width="20" height="10" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        assert_eq!(sniff(&path).unwrap(), Format::Svg);
        let natural = decode_image(&path, Format::Svg, &DecodeOptions::default(), None).unwrap();
        assert_eq!((natural.width(), natural.height()), (20, 10));
        let opts = DecodeOptions {
            svg_scale: 50.0,
            ..DecodeOptions::default()
        };
        let huge = decode_image(&path, Format::Svg, &opts, None).unwrap();
        assert_eq!((huge.width(), huge.height()), (1000, 500));
        assert_eq!(
            huge.first().to_rgba8().get_pixel(500, 250),
            &Rgba([255, 0, 0, 255])
        );
        let fitted = decode_image(&path, Format::Svg, &DecodeOptions::preview(256), None).unwrap();
        assert_eq!((fitted.width(), fitted.height()), (256, 128));
        let info = probe_image(&path, Format::Svg, None)
            .unwrap()
            .image
            .unwrap();
        assert_eq!((info.width, info.height), (20, 10));
    }

    /// a flattened 8-bit rgb psd with raw (uncompressed) image data
    fn tiny_psd(width: u32, height: u32, colour: [u8; 3]) -> Vec<u8> {
        let mut psd = b"8BPS".to_vec();
        psd.extend_from_slice(&1u16.to_be_bytes());
        psd.extend_from_slice(&[0; 6]);
        psd.extend_from_slice(&3u16.to_be_bytes());
        psd.extend_from_slice(&height.to_be_bytes());
        psd.extend_from_slice(&width.to_be_bytes());
        psd.extend_from_slice(&8u16.to_be_bytes());
        psd.extend_from_slice(&3u16.to_be_bytes());
        // colour mode data, image resources, layer and mask info: all empty
        psd.extend_from_slice(&0u32.to_be_bytes());
        psd.extend_from_slice(&0u32.to_be_bytes());
        psd.extend_from_slice(&0u32.to_be_bytes());
        psd.extend_from_slice(&0u16.to_be_bytes());
        for channel in colour {
            psd.extend(std::iter::repeat_n(channel, (width * height) as usize));
        }
        psd
    }

    #[test]
    fn photoshop_documents_flatten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.psd");
        fs::write(&path, tiny_psd(6, 4, [10, 200, 30])).unwrap();
        assert_eq!(sniff(&path).unwrap(), Format::Psd);
        let decoded = decode_image(&path, Format::Psd, &DecodeOptions::default(), None).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (6, 4));
        let px = decoded.first().to_rgb8().get_pixel(3, 2).0;
        assert_eq!(px, [10, 200, 30]);
        let info = probe_image(&path, Format::Psd, None)
            .unwrap()
            .image
            .unwrap();
        assert_eq!(
            (info.width, info.height, info.color_model.as_deref()),
            (6, 4, Some("RGB"))
        );
    }

    #[test]
    fn damaged_files_fail_politely() {
        let dir = tempfile::tempdir().unwrap();
        for (name, format) in [
            ("a.png", Format::Png),
            ("a.psd", Format::Psd),
            ("a.nef", Format::Raw),
        ] {
            let path = dir.path().join(name);
            let mut bytes = match format {
                Format::Png => b"\x89PNG\r\n\x1a\n".to_vec(),
                Format::Psd => b"8BPS\0\x01".to_vec(),
                _ => b"II*\0".to_vec(),
            };
            bytes.extend_from_slice(&[0x55; 64]);
            fs::write(&path, &bytes).unwrap();
            assert!(
                decode_image(&path, format, &DecodeOptions::default(), None).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn avif_decodes_through_ffmpeg() {
        let Some(tools) = testing::tools() else {
            return;
        };
        if !testing::has_encoder(&tools, "libaom-av1") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let png = save(
            dir.path(),
            "src.png",
            &DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 24, Rgb([200, 40, 40]))),
        );
        let path = dir.path().join("a.avif");
        testing::make(
            &tools,
            &format!("-i {} -c:v libaom-av1 -still-picture 1", png.display()),
            &path,
        );
        assert_eq!(sniff(&path).unwrap(), Format::Avif);
        let decoded =
            decode_image(&path, Format::Avif, &DecodeOptions::default(), Some(&tools)).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (32, 24));
        let px = decoded.first().to_rgb8().get_pixel(16, 12).0;
        assert!(px[0] > 170 && px[1] < 80, "{px:?}");
        let info = probe_image(&path, Format::Avif, Some(&tools)).unwrap();
        assert_eq!(info.kind, MediaKind::Image);
        assert_eq!(info.image.unwrap().width, 32);
    }

    #[test]
    fn heic_decodes_when_a_heif_decoder_is_installed() {
        let Some(encoder) = find_program("heif-enc") else {
            eprintln!("skipping: heif-enc is not installed");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let png = save(
            dir.path(),
            "src.png",
            &DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 48, Rgb([30, 60, 220]))),
        );
        let path = dir.path().join("photo.heic");
        let mut cmd = crate::ffmpeg::command(&encoder);
        cmd.arg("-o").arg(&path).arg(&png);
        if crate::ffmpeg::run_output(&mut cmd, "heif-enc").is_err() {
            eprintln!("skipping: heif-enc has no hevc encoder");
            return;
        }
        assert_eq!(sniff(&path).unwrap(), Format::Heif);
        let tools = FfmpegTools::locate(None).ok();
        let decoded = decode_image(
            &path,
            Format::Heif,
            &DecodeOptions::default(),
            tools.as_ref(),
        )
        .unwrap();
        assert_eq!((decoded.width(), decoded.height()), (64, 48));
        let px = decoded.first().to_rgb8().get_pixel(32, 24).0;
        assert!(px[2] > 180 && px[0] < 80, "{px:?}");
        let info = probe_image(&path, Format::Heif, tools.as_ref())
            .unwrap()
            .image
            .unwrap();
        assert_eq!((info.width, info.height), (64, 48));
    }
}
