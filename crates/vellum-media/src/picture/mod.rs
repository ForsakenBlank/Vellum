//! one decoder api for every still image format vellum opens.
//!
//! - png, jpeg, webp, gif, tiff, bmp, ico, tga, qoi, hdr and exr go through the `image`
//!   crate, with all frames of animated gif, apng and webp when asked for
//! - camera raw goes through `rawler` (pure rust, no libraw to ship)
//! - svg is rendered by `resvg` at any size
//! - psd shows the flattened image photoshop stores next to the layers
//! - avif, heic and jpeg xl are decoded by ffmpeg, so no extra native libraries are needed
//!
//! big files stay cheap when only a preview is wanted: jpegs are scaled down while they are
//! decoded and pngs are shrunk row by row, so a 200 megapixel scan never sits in memory at
//! full size just to make a thumbnail. see `ARCHITECTURE.md` section 22.

mod psd;
mod raw;
mod scaled;
mod vector;
mod via_ffmpeg;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use ::image::codecs::gif::GifDecoder;
use ::image::codecs::png::PngDecoder;
use ::image::codecs::webp::WebPDecoder;
use ::image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageReader, Limits, RgbaImage};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::Ffmpeg;

pub use scaled::{fit, shrink};

/// the most memory one decoded image may use, about a 30000 by 30000 photo
const MAX_DECODE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// all frames of an animation together, decoded
const MAX_ANIMATION_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum PictureFormat {
    Png,
    Jpeg,
    Webp,
    Gif,
    Tiff,
    Bmp,
    Ico,
    Tga,
    Qoi,
    Hdr,
    Exr,
    Avif,
    Heif,
    Jxl,
    Raw,
    Svg,
    Psd,
}

const RAW_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "raf", "srw", "pef",
];

impl PictureFormat {
    /// works out the format from the first bytes of the file, then the extension. raw
    /// extensions win over the bytes, because most raw files are tiff inside.
    pub fn of_path(path: &Path) -> MediaResult<Option<Self>> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if RAW_EXTENSIONS.contains(&ext.as_str()) {
            return Ok(Some(Self::Raw));
        }
        let mut head = [0_u8; 1024];
        let mut file = File::open(path).map_err(|err| MediaError::io(path, err))?;
        let mut len = 0;
        while len < head.len() {
            match file.read(&mut head[len..]) {
                Ok(0) => break,
                Ok(n) => len += n,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                Err(err) => return Err(MediaError::io(path, err)),
            }
        }
        Ok(Self::sniff(&head[..len]).or_else(|| Self::from_extension(&ext)))
    }

    pub fn sniff(head: &[u8]) -> Option<Self> {
        let starts = |magic: &[u8]| head.starts_with(magic);
        if starts(b"\x89PNG\r\n\x1a\n") {
            return Some(Self::Png);
        }
        if starts(&[0xff, 0xd8, 0xff]) {
            return Some(Self::Jpeg);
        }
        if starts(b"GIF87a") || starts(b"GIF89a") {
            return Some(Self::Gif);
        }
        if head.len() >= 12 && starts(b"RIFF") && &head[8..12] == b"WEBP" {
            return Some(Self::Webp);
        }
        if starts(b"II*\0") || starts(b"MM\0*") {
            return Some(Self::Tiff);
        }
        if starts(b"8BPS") {
            return Some(Self::Psd);
        }
        if starts(b"qoif") {
            return Some(Self::Qoi);
        }
        if starts(b"#?RADIANCE") || starts(b"#?RGBE") {
            return Some(Self::Hdr);
        }
        if starts(&[0x76, 0x2f, 0x31, 0x01]) {
            return Some(Self::Exr);
        }
        if starts(&[0xff, 0x0a]) || starts(b"\0\0\0\x0cJXL ") {
            return Some(Self::Jxl);
        }
        if head.len() >= 12 && &head[4..8] == b"ftyp" {
            return iso_brand(head);
        }
        if starts(b"BM") && head.len() >= 14 {
            return Some(Self::Bmp);
        }
        if starts(&[0, 0, 1, 0]) {
            return Some(Self::Ico);
        }
        let text = String::from_utf8_lossy(head);
        let trimmed = text.trim_start_matches('\u{feff}').trim_start();
        if trimmed.starts_with('<') && text.contains("<svg") {
            return Some(Self::Svg);
        }
        None
    }

    fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext {
            "png" | "apng" => Self::Png,
            "jpg" | "jpeg" | "jfif" => Self::Jpeg,
            "webp" => Self::Webp,
            "gif" => Self::Gif,
            "tif" | "tiff" => Self::Tiff,
            "bmp" => Self::Bmp,
            "ico" => Self::Ico,
            "tga" => Self::Tga,
            "qoi" => Self::Qoi,
            "hdr" => Self::Hdr,
            "exr" => Self::Exr,
            "avif" => Self::Avif,
            "heic" | "heif" => Self::Heif,
            "jxl" => Self::Jxl,
            "svg" | "svgz" => Self::Svg,
            "psd" => Self::Psd,
            _ => return None,
        })
    }

    /// for "JPEG photo" style labels in the info panel
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Webp => "WebP",
            Self::Gif => "GIF",
            Self::Tiff => "TIFF",
            Self::Bmp => "BMP",
            Self::Ico => "ICO",
            Self::Tga => "TGA",
            Self::Qoi => "QOI",
            Self::Hdr => "Radiance HDR",
            Self::Exr => "OpenEXR",
            Self::Avif => "AVIF",
            Self::Heif => "HEIC",
            Self::Jxl => "JPEG XL",
            Self::Raw => "Camera raw",
            Self::Svg => "SVG",
            Self::Psd => "Photoshop",
        }
    }

    fn image_format(self) -> Option<::image::ImageFormat> {
        use ::image::ImageFormat as F;
        Some(match self {
            Self::Png => F::Png,
            Self::Jpeg => F::Jpeg,
            Self::Webp => F::WebP,
            Self::Gif => F::Gif,
            Self::Tiff => F::Tiff,
            Self::Bmp => F::Bmp,
            Self::Ico => F::Ico,
            Self::Tga => F::Tga,
            Self::Qoi => F::Qoi,
            Self::Hdr => F::Hdr,
            Self::Exr => F::OpenExr,
            _ => return None,
        })
    }

    /// formats that need ffmpeg to decode
    pub fn needs_ffmpeg(self) -> bool {
        matches!(self, Self::Avif | Self::Heif | Self::Jxl)
    }
}

/// heif, avif and canon cr3 all share the iso media file layout, the brands tell them apart
fn iso_brand(head: &[u8]) -> Option<PictureFormat> {
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let end = size.clamp(12, head.len());
    let brands: Vec<&[u8]> = std::iter::once(&head[8..12])
        .chain(head[16.min(end)..end].chunks_exact(4))
        .collect();
    let has = |brand: &[u8]| brands.contains(&brand);
    if has(b"crx ") {
        Some(PictureFormat::Raw)
    } else if has(b"avif") || has(b"avis") {
        Some(PictureFormat::Avif)
    } else if [
        "heic", "heix", "heim", "heis", "hevc", "hevx", "mif1", "msf1",
    ]
    .iter()
    .any(|b| has(b.as_bytes()))
    {
        Some(PictureFormat::Heif)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodeOptions {
    /// the longest edge wanted. raster images are only ever shrunk to it, svg is rendered at
    /// exactly this size. none decodes at full size.
    pub size: Option<u32>,
    /// every frame of an animated gif, png or webp, not just the first
    pub all_frames: bool,
}

impl DecodeOptions {
    pub fn preview(size: u32) -> Self {
        Self {
            size: Some(size),
            all_frames: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub image: RgbaImage,
    pub delay_ms: u32,
}

#[derive(Debug, Clone)]
pub struct Picture {
    pub format: PictureFormat,
    /// the full size of the source, upright, whatever size was decoded
    pub width: u32,
    pub height: u32,
    /// at least one
    pub frames: Vec<Frame>,
    /// the colour profile embedded in the file, if the decoder found one. none means srgb.
    pub icc: Option<Vec<u8>>,
}

impl Picture {
    pub fn first(&self) -> &RgbaImage {
        // decoders never return a picture without frames
        &self.frames[0].image
    }

    fn still(format: PictureFormat, width: u32, height: u32, image: RgbaImage) -> Self {
        Self {
            format,
            width,
            height,
            frames: vec![Frame { image, delay_ms: 0 }],
            icc: None,
        }
    }
}

/// decodes any supported image. `ffmpeg` is only needed for avif, heic and jpeg xl.
pub fn decode(
    path: &Path,
    options: DecodeOptions,
    ffmpeg: Option<&Ffmpeg>,
) -> MediaResult<Picture> {
    let format = PictureFormat::of_path(path)?.ok_or_else(|| unsupported(path))?;
    let mut picture = match format {
        PictureFormat::Raw => raw::decode(path, options)?,
        PictureFormat::Svg => vector::decode(path, options)?,
        PictureFormat::Psd => psd::decode(path)?,
        f if f.needs_ffmpeg() => {
            let ffmpeg = ffmpeg.ok_or(MediaError::FfmpegMissing)?;
            via_ffmpeg::decode(ffmpeg, path, f, options)?
        }
        f => match (decode_raster(path, f, options), ffmpeg) {
            (Ok(picture), _) => picture,
            // odd variants the pure rust decoders do not cover (ycbcr tiff, cmyk jpeg,
            // 16 bit bmp) are usually fine in ffmpeg
            (Err(err @ MediaError::Decode(_)), Some(ffmpeg)) => {
                via_ffmpeg::decode(ffmpeg, path, f, options).map_err(|_| err)?
            }
            (Err(err), _) => return Err(err),
        },
    };
    if format != PictureFormat::Svg
        && let Some(size) = options.size
    {
        for frame in &mut picture.frames {
            frame.image = scaled::shrink(std::mem::take(&mut frame.image), size);
        }
    }
    Ok(picture)
}

/// the format and upright size without decoding the pixels where the format allows it
pub fn dimensions(path: &Path, ffmpeg: Option<&Ffmpeg>) -> MediaResult<(PictureFormat, u32, u32)> {
    let format = PictureFormat::of_path(path)?.ok_or_else(|| unsupported(path))?;
    let (width, height) = match format {
        PictureFormat::Raw => raw::dimensions(path)?,
        PictureFormat::Svg => vector::dimensions(path)?,
        PictureFormat::Psd => psd::dimensions(path)?,
        f if f.needs_ffmpeg() => {
            let ffmpeg = ffmpeg.ok_or(MediaError::FfmpegMissing)?;
            via_ffmpeg::dimensions(ffmpeg, path, f)?
        }
        f => match (raster_dimensions(path, f), ffmpeg) {
            (Ok(size), _) => size,
            (Err(err @ MediaError::Decode(_)), Some(ffmpeg)) => {
                via_ffmpeg::dimensions(ffmpeg, path, f).map_err(|_| err)?
            }
            (Err(err), _) => return Err(err),
        },
    };
    Ok((format, width, height))
}

fn unsupported(path: &Path) -> MediaError {
    MediaError::Unsupported(format!(
        "{} is not an image format Vellum can read.",
        vellum_core::files::display_name(path)
    ))
}

fn upright(w: u32, h: u32, orientation: ::image::metadata::Orientation) -> (u32, u32) {
    use ::image::metadata::Orientation as O;
    match orientation {
        O::Rotate90 | O::Rotate270 | O::Rotate90FlipH | O::Rotate270FlipH => (h, w),
        _ => (w, h),
    }
}

fn raster_dimensions(path: &Path, format: PictureFormat) -> MediaResult<(u32, u32)> {
    let mut decoder = raster_decoder(path, format)?;
    let (w, h) = decoder.dimensions();
    let orientation = decoder
        .orientation()
        .map_err(|err| MediaError::decode(path, err))?;
    Ok(upright(w, h, orientation))
}

fn raster_decoder(path: &Path, format: PictureFormat) -> MediaResult<Box<dyn ImageDecoder>> {
    let file = File::open(path).map_err(|err| MediaError::io(path, err))?;
    let image_format = format.image_format().ok_or_else(|| unsupported(path))?;
    let mut reader = ImageReader::with_format(BufReader::new(file), image_format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|err| MediaError::decode(path, err))?;
    Ok(Box::new(decoder))
}

fn decode_raster(
    path: &Path,
    format: PictureFormat,
    options: DecodeOptions,
) -> MediaResult<Picture> {
    if options.all_frames
        && matches!(
            format,
            PictureFormat::Gif | PictureFormat::Png | PictureFormat::Webp
        )
        && let Some(picture) = decode_animation(path, format)?
    {
        return Ok(picture);
    }
    if let Some(size) = options.size {
        let fast = match format {
            PictureFormat::Jpeg => scaled::jpeg(path, size),
            PictureFormat::Png => scaled::png(path, size),
            _ => None,
        };
        if let Some((image, width, height)) = fast {
            return Ok(Picture::still(format, width, height, image));
        }
    }
    let mut decoder = raster_decoder(path, format)?;
    let orientation = decoder
        .orientation()
        .map_err(|err| MediaError::decode(path, err))?;
    // a profile that cannot be read is not worth failing the picture over
    let icc = decoder.icc_profile().ok().flatten();
    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|err| MediaError::decode(path, err))?;
    image.apply_orientation(orientation);
    let (width, height) = (image.width(), image.height());
    let mut picture = Picture::still(format, width, height, image.into_rgba8());
    picture.icc = icc;
    Ok(picture)
}

/// none when the file is not animated after all, so the caller decodes it as a still
fn decode_animation(path: &Path, format: PictureFormat) -> MediaResult<Option<Picture>> {
    let open = || -> MediaResult<BufReader<File>> {
        Ok(BufReader::new(
            File::open(path).map_err(|err| MediaError::io(path, err))?,
        ))
    };
    let bad = |err: ::image::ImageError| MediaError::decode(path, err);
    let frames: Box<dyn Iterator<Item = ::image::ImageResult<::image::Frame>>> = match format {
        PictureFormat::Gif => Box::new(GifDecoder::new(open()?).map_err(bad)?.into_frames()),
        PictureFormat::Png => {
            let decoder = PngDecoder::new(open()?).map_err(bad)?;
            if !decoder.is_apng().map_err(bad)? {
                return Ok(None);
            }
            Box::new(decoder.apng().map_err(bad)?.into_frames())
        }
        PictureFormat::Webp => {
            let decoder = WebPDecoder::new(open()?).map_err(bad)?;
            if !decoder.has_animation() {
                return Ok(None);
            }
            Box::new(decoder.into_frames())
        }
        _ => return Ok(None),
    };
    let mut out = Vec::new();
    let mut bytes = 0_u64;
    for frame in frames {
        let frame = frame.map_err(bad)?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = numer.checked_div(denom).unwrap_or(0);
        let image = frame.into_buffer();
        bytes += u64::from(image.width()) * u64::from(image.height()) * 4;
        if bytes > MAX_ANIMATION_BYTES {
            return Err(MediaError::Unsupported(format!(
                "{} has too many frames to open at once.",
                vellum_core::files::display_name(path)
            )));
        }
        out.push(Frame { image, delay_ms });
    }
    let Some(first) = out.first() else {
        return Ok(None);
    };
    let (width, height) = first.image.dimensions();
    Ok(Some(Picture {
        format,
        width,
        height,
        frames: out,
        icc: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_bytes_win_over_wrong_extensions() {
        assert_eq!(
            PictureFormat::sniff(b"\x89PNG\r\n\x1a\nxxxx"),
            Some(PictureFormat::Png)
        );
        assert_eq!(
            PictureFormat::sniff(&[0xff, 0xd8, 0xff, 0xe0]),
            Some(PictureFormat::Jpeg)
        );
        assert_eq!(
            PictureFormat::sniff(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(PictureFormat::Webp)
        );
        assert_eq!(
            PictureFormat::sniff(b"8BPS\0\x01"),
            Some(PictureFormat::Psd)
        );
        assert_eq!(
            PictureFormat::sniff(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"x\"/>"),
            Some(PictureFormat::Svg)
        );
        assert_eq!(PictureFormat::sniff(b"hello"), None);
    }

    #[test]
    fn iso_brands_tell_heic_avif_and_cr3_apart() {
        let boxed = |major: &[u8], compatible: &[&[u8]]| {
            let mut b = Vec::new();
            let size = 16 + 4 * compatible.len();
            b.extend_from_slice(&(size as u32).to_be_bytes());
            b.extend_from_slice(b"ftyp");
            b.extend_from_slice(major);
            b.extend_from_slice(&[0, 0, 0, 0]);
            for c in compatible {
                b.extend_from_slice(c);
            }
            b
        };
        assert_eq!(
            PictureFormat::sniff(&boxed(b"heic", &[b"mif1"])),
            Some(PictureFormat::Heif)
        );
        assert_eq!(
            PictureFormat::sniff(&boxed(b"mif1", &[b"avif", b"miaf"])),
            Some(PictureFormat::Avif)
        );
        assert_eq!(
            PictureFormat::sniff(&boxed(b"crx ", &[b"isom"])),
            Some(PictureFormat::Raw)
        );
        assert_eq!(PictureFormat::sniff(&boxed(b"isom", &[b"mp41"])), None);
    }

    #[test]
    fn rotated_photos_report_upright_sizes() {
        use ::image::metadata::Orientation as O;
        assert_eq!(upright(400, 300, O::Rotate90), (300, 400));
        assert_eq!(upright(400, 300, O::FlipHorizontal), (400, 300));
    }
}
