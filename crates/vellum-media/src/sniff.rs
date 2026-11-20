//! tells file formats apart by their first bytes, falling back to the extension. a photo
//! saved as `.png` that is really a jpeg (very common with downloads) still opens.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::{MediaError, MediaResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    WebP,
    Tiff,
    Bmp,
    Ico,
    Qoi,
    Tga,
    Pnm,
    Hdr,
    Exr,
    Psd,
    Svg,
    Avif,
    Heif,
    Jxl,
    /// camera raw, decoded by rawler
    Raw,
    /// anything else: video, audio and whatever ffprobe can open
    Media,
}

impl Format {
    /// short id used in `MediaInfo::format`
    pub fn id(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::WebP => "webp",
            Self::Tiff => "tiff",
            Self::Bmp => "bmp",
            Self::Ico => "ico",
            Self::Qoi => "qoi",
            Self::Tga => "tga",
            Self::Pnm => "pnm",
            Self::Hdr => "hdr",
            Self::Exr => "exr",
            Self::Psd => "psd",
            Self::Svg => "svg",
            Self::Avif => "avif",
            Self::Heif => "heic",
            Self::Jxl => "jxl",
            Self::Raw => "raw",
            Self::Media => "media",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG image",
            Self::Jpeg => "JPEG image",
            Self::Gif => "GIF",
            Self::WebP => "WebP image",
            Self::Tiff => "TIFF image",
            Self::Bmp => "BMP image",
            Self::Ico => "Icon",
            Self::Qoi => "QOI image",
            Self::Tga => "TGA image",
            Self::Pnm => "PNM image",
            Self::Hdr => "Radiance HDR image",
            Self::Exr => "OpenEXR image",
            Self::Psd => "Photoshop document",
            Self::Svg => "SVG image",
            Self::Avif => "AVIF image",
            Self::Heif => "HEIC image",
            Self::Jxl => "JPEG XL image",
            Self::Raw => "Camera raw photo",
            Self::Media => "Media file",
        }
    }

    /// formats the `image` crate decodes itself
    pub fn is_raster(self) -> bool {
        matches!(
            self,
            Self::Png
                | Self::Jpeg
                | Self::Gif
                | Self::WebP
                | Self::Tiff
                | Self::Bmp
                | Self::Ico
                | Self::Qoi
                | Self::Tga
                | Self::Pnm
                | Self::Hdr
                | Self::Exr
        )
    }

    pub fn is_image(self) -> bool {
        !matches!(self, Self::Media)
    }

    pub(crate) fn image_crate_format(self) -> Option<image::ImageFormat> {
        use image::ImageFormat as F;
        Some(match self {
            Self::Png => F::Png,
            Self::Jpeg => F::Jpeg,
            Self::Gif => F::Gif,
            Self::WebP => F::WebP,
            Self::Tiff => F::Tiff,
            Self::Bmp => F::Bmp,
            Self::Ico => F::Ico,
            Self::Qoi => F::Qoi,
            Self::Tga => F::Tga,
            Self::Pnm => F::Pnm,
            Self::Hdr => F::Hdr,
            Self::Exr => F::OpenExr,
            _ => return None,
        })
    }
}

const RAW_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "orf", "rw2", "raf", "srw",
    "pef", "erf", "kdc", "dcr", "mos", "mrw", "3fr", "iiq", "x3f", "rwl",
];

/// works out a file's format from its first bytes and its name
pub fn sniff(path: &Path) -> MediaResult<Format> {
    let mut head = [0u8; 64];
    let mut file = File::open(path).map_err(|err| MediaError::io(path, err))?;
    let mut filled = 0;
    // read can return short counts, so keep going until the buffer is full or the file ends
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(MediaError::io(path, err)),
        }
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    Ok(sniff_bytes(&head[..filled], &ext))
}

/// the same as [`sniff`] for bytes already in memory. `ext` is lowercase without the dot.
pub fn sniff_bytes(head: &[u8], ext: &str) -> Format {
    // raw files are often tiff inside, so the extension decides before the tiff magic does
    if RAW_EXTENSIONS.contains(&ext) {
        return Format::Raw;
    }
    if let Some(format) = from_magic(head) {
        return format;
    }
    match ext {
        "svg" | "svgz" => Format::Svg,
        "tga" => Format::Tga,
        _ => Format::Media,
    }
}

fn from_magic(head: &[u8]) -> Option<Format> {
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Some(Format::Png);
    }
    if starts(&[0xff, 0xd8, 0xff]) {
        return Some(Format::Jpeg);
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(Format::Gif);
    }
    if starts(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Some(Format::WebP);
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return Some(Format::Tiff);
    }
    if starts(b"8BPS") {
        return Some(Format::Psd);
    }
    if starts(b"BM") && head.len() >= 14 {
        return Some(Format::Bmp);
    }
    if starts(&[0, 0, 1, 0]) || starts(&[0, 0, 2, 0]) {
        return Some(Format::Ico);
    }
    if starts(b"qoif") {
        return Some(Format::Qoi);
    }
    if starts(b"#?RADIANCE") || starts(b"#?RGBE") {
        return Some(Format::Hdr);
    }
    if starts(&[0x76, 0x2f, 0x31, 0x01]) {
        return Some(Format::Exr);
    }
    if starts(&[0xff, 0x0a]) || starts(b"\0\0\0\x0cJXL \r\n\x87\n") {
        return Some(Format::Jxl);
    }
    if head.len() >= 2 && head[0] == b'P' && (b'1'..=b'7').contains(&head[1]) {
        return Some(Format::Pnm);
    }
    if let Some(format) = from_ftyp(head) {
        return Some(format);
    }
    if looks_like_svg(head) {
        return Some(Format::Svg);
    }
    None
}

/// iso media files (mp4, mov, heic, avif) start with an `ftyp` box naming their brands
fn from_ftyp(head: &[u8]) -> Option<Format> {
    if head.get(4..8) != Some(b"ftyp") {
        return None;
    }
    let size = u32::from_be_bytes(head.get(0..4)?.try_into().ok()?) as usize;
    let end = size.min(head.len());
    let major = head.get(8..12)?;
    let compatible = head.get(16..end).unwrap_or_default();
    let brands = std::iter::once(major).chain(compatible.chunks_exact(4));
    let mut found = None;
    for brand in brands {
        match brand {
            // a still avif. "avis" is an avif sequence, which ffprobe treats as video.
            b"avif" => return Some(Format::Avif),
            b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" => {
                found.get_or_insert(Format::Heif);
            }
            b"avis" => return None,
            _ => {}
        }
    }
    found
}

fn looks_like_svg(head: &[u8]) -> bool {
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_bytes_win_over_the_extension() {
        assert_eq!(sniff_bytes(&[0xff, 0xd8, 0xff, 0xe0], "png"), Format::Jpeg);
        assert_eq!(sniff_bytes(b"\x89PNG\r\n\x1a\n....", "jpg"), Format::Png);
        assert_eq!(sniff_bytes(b"RIFF\0\0\0\0WEBPVP8 ", "webp"), Format::WebP);
        assert_eq!(sniff_bytes(b"RIFF\0\0\0\0WAVEfmt ", "wav"), Format::Media);
        assert_eq!(sniff_bytes(b"8BPS\0\x01", ""), Format::Psd);
        assert_eq!(sniff_bytes(b"GIF89a", ""), Format::Gif);
    }

    #[test]
    fn raw_extensions_beat_the_tiff_magic() {
        assert_eq!(sniff_bytes(b"II*\0\x08\0\0\0", "nef"), Format::Raw);
        assert_eq!(sniff_bytes(b"II*\0\x08\0\0\0", "tif"), Format::Tiff);
        assert_eq!(
            sniff_bytes(b"", "CR3".to_ascii_lowercase().as_str()),
            Format::Raw
        );
    }

    #[test]
    fn iso_brands_split_stills_from_video() {
        let ftyp = |major: &[u8], compatible: &[u8]| {
            let mut bytes = Vec::new();
            let size = 16 + compatible.len();
            bytes.extend_from_slice(&(size as u32).to_be_bytes());
            bytes.extend_from_slice(b"ftyp");
            bytes.extend_from_slice(major);
            bytes.extend_from_slice(&[0, 0, 0, 0]);
            bytes.extend_from_slice(compatible);
            bytes
        };
        assert_eq!(sniff_bytes(&ftyp(b"avif", b"mif1miaf"), ""), Format::Avif);
        assert_eq!(sniff_bytes(&ftyp(b"mif1", b"heic"), ""), Format::Heif);
        assert_eq!(sniff_bytes(&ftyp(b"heic", b"mif1"), ""), Format::Heif);
        assert_eq!(
            sniff_bytes(&ftyp(b"isom", b"iso2avc1mp41"), ""),
            Format::Media
        );
        assert_eq!(sniff_bytes(&ftyp(b"avis", b"avifmsf1"), ""), Format::Media);
        assert_eq!(sniff_bytes(&ftyp(b"qt  ", b"qt  "), "mov"), Format::Media);
    }

    #[test]
    fn svg_is_found_by_content_or_name() {
        assert_eq!(sniff_bytes(b"<?xml version=\"1.0\"?><svg", ""), Format::Svg);
        assert_eq!(sniff_bytes(b"  <svg xmlns=", "txt"), Format::Svg);
        assert_eq!(sniff_bytes(b"\x1f\x8b\x08", "svgz"), Format::Svg);
        assert_eq!(sniff_bytes(b"hello", "txt"), Format::Media);
    }
}
