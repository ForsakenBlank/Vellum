//! "strip metadata": writes a copy of a file without camera details, gps position,
//! editing software, comments and the like. the original is never touched.
//!
//! jpeg, png and webp are rewritten block by block, so the picture itself is copied byte for
//! byte with no quality loss. colour profiles stay (they change how the picture looks, not
//! who took it) and so does the orientation of a photo, rewritten as a tiny exif block on its
//! own, so a portrait photo does not come out sideways. video and audio are stream copied by
//! ffmpeg with every metadata tag and chapter dropped, again without re-encoding.

use std::path::{Path, PathBuf};

use crate::control::Control;
use crate::error::{MediaError, MediaResult};
use crate::ffmpeg::Ffmpeg;
use crate::ffmpeg::encode::partial_path;
use crate::ffmpeg::process::Running;
use crate::picture::PictureFormat;

/// `photo.jpg` becomes `photo (no metadata).jpg`, or `photo (no metadata 2).jpg` and so on
/// if that exists. never an existing file.
pub fn unique_sibling(path: &Path, label: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem} ({label}){ext}")
        } else {
            format!("{stem} ({label} {n}){ext}")
        };
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// writes the stripped copy next to the original and returns its path
pub fn strip_metadata(
    ffmpeg: Option<&Ffmpeg>,
    path: &Path,
    control: &dyn Control,
) -> MediaResult<PathBuf> {
    let output = unique_sibling(path, "no metadata");
    let format = PictureFormat::of_path(path)?;
    let cannot = |what: &str| {
        MediaError::Unsupported(format!(
            "Vellum cannot strip metadata from {what} files yet. Convert it to JPEG, PNG or WebP first."
        ))
    };
    let stripped = match format {
        Some(PictureFormat::Jpeg) => {
            Some(strip_jpeg(&read(path)?).map_err(|e| MediaError::decode(path, e))?)
        }
        Some(PictureFormat::Png) => {
            Some(strip_png(&read(path)?).map_err(|e| MediaError::decode(path, e))?)
        }
        Some(PictureFormat::Webp) => {
            Some(strip_webp(&read(path)?).map_err(|e| MediaError::decode(path, e))?)
        }
        Some(other) => return Err(cannot(other.label())),
        None => None,
    };
    control.checkpoint()?;
    let partial = partial_path(&output);
    let result = match stripped {
        Some(bytes) => std::fs::write(&partial, bytes).map_err(|err| MediaError::io(&partial, err)),
        None => strip_with_ffmpeg(
            ffmpeg.ok_or(MediaError::FfmpegMissing)?,
            path,
            &partial,
            control,
        ),
    };
    if let Err(err) = result {
        let _ = std::fs::remove_file(&partial);
        return Err(err);
    }
    std::fs::rename(&partial, &output).map_err(|err| MediaError::io(&output, err))?;
    control.progress(1.0);
    Ok(output)
}

fn read(path: &Path) -> MediaResult<Vec<u8>> {
    std::fs::read(path).map_err(|err| MediaError::io(path, err))
}

fn strip_with_ffmpeg(
    ffmpeg: &Ffmpeg,
    input: &Path,
    output: &Path,
    control: &dyn Control,
) -> MediaResult<()> {
    let mut cmd = ffmpeg.command();
    cmd.arg("-i")
        .arg(input)
        .args(["-map", "0", "-c", "copy"])
        .args(["-map_metadata", "-1", "-map_chapters", "-1"])
        // stream titles and languages are tags too, and encoder names say which app made it
        .args([
            "-metadata:s",
            "title=",
            "-metadata:s",
            "handler_name=",
            "-fflags",
            "+bitexact",
        ])
        .args(["-progress", "pipe:1", "-nostats", "-y"])
        .arg(output);
    let mut running = Running::spawn(cmd)?;
    running.lines(control, |_| {})?;
    running.finish()
}

/// jpeg markers that carry no personal information and change how the picture decodes
fn keep_jpeg_segment(marker: u8, body: &[u8]) -> bool {
    match marker {
        0xe0 => true,                               // app0, jfif
        0xe2 => body.starts_with(b"ICC_PROFILE\0"), // colour profile
        0xee => true,                               // app14, adobe colour transform
        0xe1 | 0xe3..=0xed | 0xef | 0xfe => false,  // exif, xmp, iptc, comments, vendor blocks
        _ => true,                                  // tables and frame headers
    }
}

pub fn strip_jpeg(data: &[u8]) -> Result<Vec<u8>, String> {
    if !data.starts_with(&[0xff, 0xd8]) {
        return Err("it is not a JPEG".into());
    }
    let orientation = jpeg_orientation(data).filter(|o| *o != 1);
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&[0xff, 0xd8]);
    if let Some(orientation) = orientation {
        out.extend(minimal_exif_segment(orientation));
    }
    let mut at = 2;
    loop {
        // skip fill bytes between segments
        while data.get(at) == Some(&0xff) && data.get(at + 1) == Some(&0xff) {
            at += 1;
        }
        let (Some(&0xff), Some(&marker)) = (data.get(at), data.get(at + 1)) else {
            return Err("a block in the file is damaged".into());
        };
        if marker == 0xd9 {
            out.extend_from_slice(&[0xff, 0xd9]);
            return Ok(out);
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            out.extend_from_slice(&data[at..at + 2]);
            at += 2;
            continue;
        }
        let len = data
            .get(at + 2..at + 4)
            .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
            .ok_or("the file ends in the middle of a block")?;
        let end = at + 2 + len;
        let segment = data
            .get(at..end)
            .ok_or("the file ends in the middle of a block")?;
        if marker == 0xda {
            // start of scan: the compressed picture runs to the end, copy it all as is
            out.extend_from_slice(&data[at..]);
            return Ok(out);
        }
        if keep_jpeg_segment(marker, &segment[4..]) {
            out.extend_from_slice(segment);
        }
        at = end;
    }
}

fn jpeg_orientation(data: &[u8]) -> Option<u16> {
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(data))
        .ok()?;
    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    u16::try_from(field.value.get_uint(0)?).ok()
}

/// an app1 segment holding a tiff with one tag: the orientation
fn minimal_exif_segment(orientation: u16) -> Vec<u8> {
    let mut tiff: Vec<u8> = b"MM\0*".to_vec();
    tiff.extend(8_u32.to_be_bytes());
    tiff.extend(1_u16.to_be_bytes());
    tiff.extend(0x0112_u16.to_be_bytes());
    tiff.extend(3_u16.to_be_bytes());
    tiff.extend(1_u32.to_be_bytes());
    tiff.extend(orientation.to_be_bytes());
    tiff.extend([0, 0]);
    tiff.extend(0_u32.to_be_bytes());
    let mut segment = vec![0xff, 0xe1];
    let len = u16::try_from(2 + 6 + tiff.len()).unwrap_or(u16::MAX);
    segment.extend(len.to_be_bytes());
    segment.extend(b"Exif\0\0");
    segment.extend(tiff);
    segment
}

/// png chunks that affect how the picture looks or animates. everything else (text,
/// exif, timestamps, private chunks) goes.
const PNG_KEEP: &[&[u8; 4]] = &[
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT",
    b"pHYs", b"bKGD", b"cICP", b"mDCv", b"cLLi", b"acTL", b"fcTL", b"fdAT",
];

pub fn strip_png(data: &[u8]) -> Result<Vec<u8>, String> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIGNATURE) {
        return Err("it is not a PNG".into());
    }
    let mut out = SIGNATURE.to_vec();
    let mut at = SIGNATURE.len();
    while at < data.len() {
        let header = data
            .get(at..at + 8)
            .ok_or("the file ends in the middle of a chunk")?;
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind = &header[4..8];
        let end = at + 12 + len;
        let chunk = data
            .get(at..end)
            .ok_or("the file ends in the middle of a chunk")?;
        if PNG_KEEP.iter().any(|keep| keep.as_slice() == kind) {
            // each chunk carries its own checksum, so a copied chunk stays valid
            out.extend_from_slice(chunk);
        }
        at = end;
        if kind == b"IEND" {
            break;
        }
    }
    Ok(out)
}

pub fn strip_webp(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err("it is not a WebP".into());
    }
    let mut body = Vec::with_capacity(data.len());
    let mut at = 12;
    while at + 8 <= data.len() {
        let kind = &data[at..at + 4];
        let len =
            u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]) as usize;
        // chunks are padded to an even length
        let end = at + 8 + len + (len & 1);
        let chunk = data
            .get(at..end.min(data.len()))
            .ok_or("the file ends in the middle of a chunk")?;
        match kind {
            b"EXIF" | b"XMP " => {}
            b"VP8X" => {
                let mut chunk = chunk.to_vec();
                // flags in the first payload byte: 0x08 exif, 0x04 xmp
                if let Some(flags) = chunk.get_mut(8) {
                    *flags &= !(0x08 | 0x04);
                }
                body.extend(chunk);
            }
            _ => body.extend_from_slice(chunk),
        }
        at = end;
    }
    let mut out = b"RIFF".to_vec();
    let size = u32::try_from(body.len() + 4).map_err(|_| "the file is too big")?;
    out.extend(size.to_le_bytes());
    out.extend(b"WEBP");
    out.extend(body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_names_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("photo.jpg");
        let first = unique_sibling(&photo, "no metadata");
        assert_eq!(first.file_name().unwrap(), "photo (no metadata).jpg");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_sibling(&photo, "no metadata");
        assert_eq!(second.file_name().unwrap(), "photo (no metadata 2).jpg");
    }

    #[test]
    fn png_text_chunks_go_and_pixels_stay() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk = |kind: &[u8], body: &[u8]| {
            let mut c = (body.len() as u32).to_be_bytes().to_vec();
            c.extend(kind);
            c.extend(body);
            c.extend([0, 0, 0, 0]); // checksum, not checked here
            c
        };
        png.extend(chunk(b"IHDR", &[0; 13]));
        png.extend(chunk(b"tEXt", b"Author\0Someone"));
        png.extend(chunk(b"iCCP", b"profile"));
        png.extend(chunk(b"IDAT", b"pixels"));
        png.extend(chunk(b"eXIf", b"exif"));
        png.extend(chunk(b"IEND", b""));
        let out = strip_png(&png).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("tEXt") && !text.contains("eXIf") && !text.contains("Someone"));
        assert!(text.contains("iCCP") && text.contains("IDAT") && text.contains("IEND"));
    }

    #[test]
    fn webp_exif_goes_and_flags_follow() {
        let mut body = Vec::new();
        body.extend(b"VP8X");
        body.extend(10_u32.to_le_bytes());
        body.extend([0x0c, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        body.extend(b"VP8 ");
        body.extend(3_u32.to_le_bytes());
        body.extend([1, 2, 3, 0]); // odd length, padded
        body.extend(b"EXIF");
        body.extend(4_u32.to_le_bytes());
        body.extend(b"gps!");
        let mut webp = b"RIFF".to_vec();
        webp.extend(((body.len() + 4) as u32).to_le_bytes());
        webp.extend(b"WEBP");
        webp.extend(body);
        let out = strip_webp(&webp).unwrap();
        assert!(!out.windows(4).any(|w| w == b"EXIF"));
        assert_eq!(out[20] & 0x0c, 0);
        let size = u32::from_le_bytes([out[4], out[5], out[6], out[7]]) as usize;
        assert_eq!(size + 8, out.len());
    }

    #[test]
    fn jpeg_keeps_tables_and_orientation_but_not_exif_or_comments() {
        let mut jpeg = vec![0xff, 0xd8];
        let segment = |marker: u8, body: &[u8]| {
            let mut s = vec![0xff, marker];
            s.extend(((body.len() + 2) as u16).to_be_bytes());
            s.extend(body);
            s
        };
        jpeg.extend(segment(0xe0, b"JFIF\0\x01\x01"));
        jpeg.extend(segment(0xfe, b"shot by someone"));
        jpeg.extend(segment(0xe2, b"ICC_PROFILE\0\x01\x01data"));
        jpeg.extend(segment(0xdb, &[0; 65]));
        jpeg.extend(segment(0xda, &[0; 10]));
        jpeg.extend([1, 2, 3, 0xff, 0xd9]);
        let out = strip_jpeg(&jpeg).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("someone"));
        assert!(text.contains("JFIF") && text.contains("ICC_PROFILE"));
        assert!(out.ends_with(&[1, 2, 3, 0xff, 0xd9]));
        assert!(strip_jpeg(b"nope").is_err());
    }

    #[test]
    fn orientation_survives_on_its_own() {
        let photo = crate::testmedia::jpeg_with_exif(&minimal_jpeg(), 6, Some((51.5, -0.1)));
        assert_eq!(jpeg_orientation(&photo), Some(6));
        let out = strip_jpeg(&photo).unwrap();
        assert_eq!(jpeg_orientation(&out), Some(6));
        let exif = exif::Reader::new()
            .read_from_container(&mut std::io::Cursor::new(&out))
            .unwrap();
        assert!(
            exif.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY)
                .is_none()
        );
        assert_eq!(exif.fields().count(), 1);
    }

    fn minimal_jpeg() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        ::image::RgbImage::new(4, 4)
            .write_to(&mut out, ::image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }
}
