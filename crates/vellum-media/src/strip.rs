//! "strip metadata": a copy of a file without location, camera, owner, software and
//! editing details. the original is never touched.
//!
//! jpeg, png, webp and gif are cleaned by rewriting their containers, so not one pixel
//! changes. the picture's orientation is kept (as a one-tag exif block) so it still shows
//! upright, and colour profiles are kept so colours do not shift. video and audio go
//! through ffmpeg with stream copy, which also drops data tracks such as gopro gps.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{MediaError, MediaResult};
use crate::exif_tags;
use crate::ffmpeg::{FfmpegTools, Reporter, run_lines};
use crate::sniff::Format;

/// writes a cleaned copy next to `path` and returns where
pub fn strip_metadata(
    path: &Path,
    format: Format,
    tools: Option<&FfmpegTools>,
    reporter: &dyn Reporter,
    duration_secs: Option<f64>,
) -> MediaResult<PathBuf> {
    let out = unique_output(path, "clean");
    let temp = out.with_file_name(format!(
        ".{}.part",
        out.file_name().and_then(|n| n.to_str()).unwrap_or("vellum")
    ));
    let result = match format {
        Format::Jpeg | Format::Png | Format::WebP | Format::Gif => {
            let bytes = fs::read(path).map_err(|err| MediaError::io(path, err))?;
            let cleaned = match format {
                Format::Jpeg => strip_jpeg(&bytes),
                Format::Png => strip_png(&bytes),
                Format::WebP => strip_webp(&bytes),
                _ => strip_gif(&bytes),
            }
            .ok_or_else(|| {
                MediaError::decode(
                    "This file is damaged, so its metadata could not be removed safely.",
                )
            })?;
            fs::write(&temp, cleaned).map_err(|err| MediaError::io(&temp, err))
        }
        Format::Media => strip_with_ffmpeg(
            tools.ok_or(MediaError::FfmpegMissing)?,
            path,
            &temp,
            &out,
            reporter,
            duration_secs,
        ),
        other => Err(MediaError::Unsupported(format!(
            "Vellum cannot remove metadata from {} files yet. Convert it to PNG or JPEG first.",
            other.label()
        ))),
    };
    if let Err(err) = result {
        let _ = fs::remove_file(&temp);
        return Err(err);
    }
    fs::rename(&temp, &out).map_err(|err| MediaError::io(&out, err))?;
    Ok(out)
}

fn strip_with_ffmpeg(
    tools: &FfmpegTools,
    input: &Path,
    temp: &Path,
    out: &Path,
    reporter: &dyn Reporter,
    duration_secs: Option<f64>,
) -> MediaResult<()> {
    let mut cmd = tools.ffmpeg();
    cmd.arg("-i")
        .arg(input)
        // every picture, sound and subtitle stream, but no data tracks (gps, telemetry)
        .args(["-map", "0", "-map", "-0:d?"])
        .args([
            "-map_metadata",
            "-1",
            "-map_metadata:s",
            "-1",
            "-map_chapters",
            "-1",
        ])
        .args(["-c", "copy"])
        // bitexact stops ffmpeg from adding its own "encoder" tags
        .args([
            "-fflags",
            "+bitexact",
            "-flags:v",
            "+bitexact",
            "-flags:a",
            "+bitexact",
        ])
        .args(["-progress", "pipe:1", "-nostats", "-y"]);
    // the temp name hides the real extension, so name the container explicitly
    if let Some(muxer) = out.extension().and_then(|e| e.to_str()).and_then(muxer_for) {
        cmd.args(["-f", muxer]);
    }
    cmd.arg(temp);
    run_lines(&mut cmd, "ffmpeg", reporter, |line| {
        if let (Some(value), Some(total)) = (line.strip_prefix("out_time_us="), duration_secs)
            && let Ok(us) = value.trim().parse::<f64>()
            && total > 0.0
        {
            reporter.progress((us / 1e6 / total).clamp(0.0, 1.0) as f32);
        }
    })
}

fn muxer_for(ext: &str) -> Option<&'static str> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" | "m4a" => "mp4",
        "mov" => "mov",
        "mkv" | "mka" => "matroska",
        "webm" => "webm",
        "avi" => "avi",
        "mp3" => "mp3",
        "flac" => "flac",
        "ogg" | "oga" => "ogg",
        "opus" => "opus",
        "wav" => "wav",
        "aiff" | "aif" => "aiff",
        "ts" | "m2ts" => "mpegts",
        "3gp" => "3gp",
        _ => return None,
    })
}

/// `photo.jpg` becomes `photo-clean.jpg`, or `photo-clean 2.jpg` if that exists already.
/// nothing is ever overwritten.
pub fn unique_output(path: &Path, suffix: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|e| e.to_str());
    let name = |n: u32| {
        let counter = if n > 1 {
            format!(" {n}")
        } else {
            String::new()
        };
        match ext {
            Some(ext) => format!("{stem}-{suffix}{counter}.{ext}"),
            None => format!("{stem}-{suffix}{counter}"),
        }
    };
    (1..)
        .map(|n| path.with_file_name(name(n)))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| path.with_file_name(name(0)))
}

/// a minimal big endian exif block holding only the orientation
fn orientation_exif(orientation: u32) -> Vec<u8> {
    let mut block = b"MM\0*".to_vec();
    block.extend_from_slice(&8u32.to_be_bytes());
    block.extend_from_slice(&1u16.to_be_bytes());
    block.extend_from_slice(&0x0112u16.to_be_bytes());
    block.extend_from_slice(&3u16.to_be_bytes());
    block.extend_from_slice(&1u32.to_be_bytes());
    // a short value sits in the first two of the four value bytes
    block.extend_from_slice(&(orientation as u16).to_be_bytes());
    block.extend_from_slice(&[0, 0]);
    block.extend_from_slice(&0u32.to_be_bytes());
    block
}

fn orientation_of(bytes: &[u8]) -> u32 {
    exif_tags::from_file_bytes(bytes).map_or(1, |e| e.orientation)
}

/// rewrites a jpeg keeping only what is needed to show it: quantisation and huffman
/// tables, frame headers, the jfif and adobe markers and the icc profile. exif, xmp,
/// iptc, comments and anything after the end of the image (extra "multi picture" images
/// with their own exif) are dropped.
pub fn strip_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let orientation = orientation_of(bytes);
    let mut out = vec![0xff, 0xd8];
    if orientation != 1 {
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(orientation_exif(orientation));
        out.extend_from_slice(&[0xff, 0xe1]);
        out.extend_from_slice(&u16::try_from(app1.len() + 2).ok()?.to_be_bytes());
        out.extend(app1);
    }
    let mut pos = 2;
    loop {
        // markers may be padded with extra 0xff bytes
        while bytes.get(pos) == Some(&0xff) && bytes.get(pos + 1) == Some(&0xff) {
            pos += 1;
        }
        if *bytes.get(pos)? != 0xff {
            return None;
        }
        let marker = *bytes.get(pos + 1)?;
        match marker {
            0xd9 => {
                out.extend_from_slice(&[0xff, 0xd9]);
                return Some(out);
            }
            0xd0..=0xd7 | 0x01 => {
                out.extend_from_slice(&[0xff, marker]);
                pos += 2;
                continue;
            }
            _ => {}
        }
        let len = usize::from(u16::from_be_bytes([
            *bytes.get(pos + 2)?,
            *bytes.get(pos + 3)?,
        ]));
        let segment = bytes.get(pos..pos + 2 + len)?;
        let payload = &segment[4..];
        let keep = match marker {
            // jfif header and adobe colour transform: needed to decode colours right
            0xe0 => payload.starts_with(b"JFIF\0") || payload.starts_with(b"JFXX\0"),
            0xee => payload.starts_with(b"Adobe"),
            0xe2 => payload.starts_with(b"ICC_PROFILE\0"),
            // every other app marker and comments are metadata
            0xe1 | 0xe3..=0xed | 0xef | 0xfe => false,
            _ => true,
        };
        if keep {
            out.extend_from_slice(segment);
        }
        pos += 2 + len;
        if marker == 0xda {
            // entropy coded data runs until the next marker that is not a restart or a
            // stuffed byte
            let start = pos;
            loop {
                let b = *bytes.get(pos)?;
                if b == 0xff {
                    let next = *bytes.get(pos + 1)?;
                    if next != 0x00 && !(0xd0..=0xd7).contains(&next) {
                        break;
                    }
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            out.extend_from_slice(&bytes[start..pos]);
        }
    }
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// rewrites a png keeping image data, palette, transparency, colour information and
/// animation chunks. text, time, exif and unknown extra chunks are dropped.
pub fn strip_png(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return None;
    }
    let orientation = orientation_of(bytes);
    let mut out = PNG_SIGNATURE.to_vec();
    let mut pos = PNG_SIGNATURE.len();
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let id: [u8; 4] = bytes[pos + 4..pos + 8].try_into().ok()?;
        let chunk = bytes.get(pos..pos + 12 + len)?;
        // an uppercase first letter marks a chunk the image cannot be shown without
        let critical = id[0].is_ascii_uppercase();
        let keep = critical
            || matches!(
                &id,
                b"tRNS"
                    | b"gAMA"
                    | b"cHRM"
                    | b"sRGB"
                    | b"iCCP"
                    | b"sBIT"
                    | b"pHYs"
                    | b"bKGD"
                    | b"acTL"
                    | b"fcTL"
                    | b"fdAT"
                    | b"cICP"
                    | b"mDCv"
                    | b"cLLi"
            );
        if keep {
            out.extend_from_slice(chunk);
        }
        if &id == b"IHDR" && orientation != 1 {
            push_png_chunk(&mut out, b"eXIf", &orientation_exif(orientation));
        }
        pos += 12 + len;
        if &id == b"IEND" {
            return Some(out);
        }
    }
    None
}

fn push_png_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(id);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// rewrites a webp without its exif and xmp chunks, fixing the header flags and size
pub fn strip_webp(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return None;
    }
    let orientation = orientation_of(bytes);
    let mut body = Vec::new();
    let mut pos = 12;
    let mut vp8x_at = None;
    while pos + 8 <= bytes.len() {
        let id: [u8; 4] = bytes[pos..pos + 4].try_into().ok()?;
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let padded = size + (size & 1);
        let chunk = bytes.get(pos..(pos + 8 + padded).min(bytes.len()))?;
        match &id {
            b"EXIF" | b"XMP " => {}
            b"VP8X" => {
                vp8x_at = Some(body.len());
                body.extend_from_slice(chunk);
            }
            _ => body.extend_from_slice(chunk),
        }
        pos += 8 + padded;
    }
    const EXIF_FLAG: u8 = 0x08;
    const XMP_FLAG: u8 = 0x04;
    if let Some(at) = vp8x_at {
        let flags = body.get_mut(at + 8)?;
        *flags &= !(EXIF_FLAG | XMP_FLAG);
        if orientation != 1 {
            *flags |= EXIF_FLAG;
            let exif = orientation_exif(orientation);
            body.extend_from_slice(b"EXIF");
            body.extend_from_slice(&(exif.len() as u32).to_le_bytes());
            body.extend_from_slice(&exif);
            if exif.len() % 2 == 1 {
                body.push(0);
            }
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(body.len() + 4).ok()?.to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend(body);
    Some(out)
}

/// rewrites a gif without comment blocks and application blocks other than the loop count
pub fn strip_gif(bytes: &[u8]) -> Option<Vec<u8>> {
    if !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let flags = *bytes.get(10)?;
    let mut pos = 13;
    if flags & 0x80 != 0 {
        pos += 3 * (1 << ((flags & 0x07) + 1));
    }
    let mut out = bytes.get(..pos)?.to_vec();
    loop {
        match *bytes.get(pos)? {
            0x21 => {
                let label = *bytes.get(pos + 1)?;
                let end = skip_sub_blocks(bytes, pos + 2)?;
                let first = bytes.get(pos + 3..)?;
                let keep = match label {
                    0xfe => false,
                    0xff => first.starts_with(b"NETSCAPE2.0") || first.starts_with(b"ANIMEXTS1.0"),
                    _ => true,
                };
                if keep {
                    out.extend_from_slice(&bytes[pos..end]);
                }
                pos = end;
            }
            0x2c => {
                let flags = *bytes.get(pos + 9)?;
                let mut header_end = pos + 10;
                if flags & 0x80 != 0 {
                    header_end += 3 * (1 << ((flags & 0x07) + 1));
                }
                let end = skip_sub_blocks(bytes, header_end + 1)?;
                out.extend_from_slice(bytes.get(pos..end)?);
                pos = end;
            }
            0x3b => {
                out.push(0x3b);
                return Some(out);
            }
            _ => return None,
        }
    }
}

fn skip_sub_blocks(bytes: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = usize::from(*bytes.get(pos)?);
        pos += 1;
        if len == 0 {
            return Some(pos);
        }
        pos += len;
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};

    use super::*;
    use crate::exif_tags::tests::exif_block;
    use crate::ffmpeg::{NoReporter, probe_media, testing};
    use crate::images::probe_image;

    fn photo() -> RgbImage {
        RgbImage::from_fn(48, 32, |x, y| Rgb([(x * 5) as u8, (y * 7) as u8, 100]))
    }

    /// a jpeg with exif (orientation 6, a camera make, gps), a comment and xmp
    fn tagged_jpeg() -> Vec<u8> {
        let mut plain = Vec::new();
        DynamicImage::ImageRgb8(photo())
            .write_to(&mut Cursor::new(&mut plain), ImageFormat::Jpeg)
            .unwrap();
        let make = u32::from_le_bytes([b'X', b'Y', 0, 0]);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(exif_block(&[(0x010f, 2, 3, make), (0x0112, 3, 1, 6)]));
        let segment = |marker: u8, payload: &[u8]| {
            let mut s = vec![0xff, marker];
            s.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
            s.extend_from_slice(payload);
            s
        };
        let mut out = plain[..2].to_vec();
        out.extend(segment(0xe1, &app1));
        out.extend(segment(
            0xe1,
            b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>secret</x:xmpmeta>",
        ));
        out.extend(segment(0xfe, b"made on my secret laptop"));
        out.extend_from_slice(&plain[2..]);
        // a trailing "multi picture" image some phones append
        out.extend_from_slice(b"\xff\xd8 another picture with more exif \xff\xd9");
        out
    }

    fn pixels(bytes: &[u8]) -> Vec<u8> {
        image::load_from_memory(bytes)
            .unwrap()
            .to_rgba8()
            .into_raw()
    }

    #[test]
    fn jpeg_loses_its_metadata_but_not_a_single_pixel_or_its_orientation() {
        let original = tagged_jpeg();
        let cleaned = strip_jpeg(&original).unwrap();
        let text = String::from_utf8_lossy(&cleaned);
        assert!(!text.contains("secret"));
        assert!(!text.contains("another picture"));
        assert!(cleaned.ends_with(&[0xff, 0xd9]));
        assert_eq!(pixels(&cleaned), pixels(&original));
        let exif = exif_tags::from_file_bytes(&cleaned).unwrap();
        assert_eq!(exif.orientation, 6);
        assert!(exif.tags.iter().all(|t| t.key != "Make"));
    }

    #[test]
    fn upright_jpegs_get_no_exif_at_all() {
        let mut plain = Vec::new();
        DynamicImage::ImageRgb8(photo())
            .write_to(&mut Cursor::new(&mut plain), ImageFormat::Jpeg)
            .unwrap();
        let cleaned = strip_jpeg(&plain).unwrap();
        assert!(exif_tags::from_file_bytes(&cleaned).is_none());
        assert_eq!(pixels(&cleaned), pixels(&plain));
    }

    #[test]
    fn png_text_and_exif_go_and_transparency_stays() {
        let image = RgbaImage::from_fn(16, 16, |x, _| Rgba([200, 10, 10, (x * 16) as u8]));
        let mut plain = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut plain), ImageFormat::Png)
            .unwrap();
        // insert text and exif chunks after ihdr (8 + 25 bytes in)
        let mut tagged = plain[..33].to_vec();
        push_png_chunk(&mut tagged, b"tEXt", b"Author\0Someone Private");
        let exif = exif_block(&[(0x0112, 3, 1, 8)]);
        push_png_chunk(&mut tagged, b"eXIf", &exif);
        push_png_chunk(&mut tagged, b"tIME", &[7, 0xe9, 1, 1, 0, 0, 0]);
        tagged.extend_from_slice(&plain[33..]);
        assert_eq!(pixels(&tagged), pixels(&plain));

        let cleaned = strip_png(&tagged).unwrap();
        assert!(!String::from_utf8_lossy(&cleaned).contains("Private"));
        assert!(!cleaned.windows(4).any(|w| w == b"tIME"));
        assert_eq!(pixels(&cleaned), pixels(&plain));
        assert_eq!(exif_tags::from_file_bytes(&cleaned).unwrap().orientation, 8);
    }

    #[test]
    fn crc_matches_the_png_reference() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn webp_exif_goes_and_the_header_is_fixed() {
        let mut plain = Vec::new();
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 128])))
            .write_to(&mut Cursor::new(&mut plain), ImageFormat::WebP)
            .unwrap();
        // build an extended webp: vp8x (exif + alpha flags), the image chunk, then exif
        let image_chunk = &plain[12..];
        let mut body = b"VP8X".to_vec();
        body.extend_from_slice(&10u32.to_le_bytes());
        body.extend_from_slice(&[0x08 | 0x10, 0, 0, 0, 7, 0, 0, 7, 0, 0]);
        body.extend_from_slice(image_chunk);
        let exif = exif_block(&[
            (0x0112, 3, 1, 1),
            (0x8298, 2, 4, u32::from_le_bytes(*b"Me!\0")),
        ]);
        body.extend_from_slice(b"EXIF");
        body.extend_from_slice(&(exif.len() as u32).to_le_bytes());
        body.extend_from_slice(&exif);
        let mut tagged = b"RIFF".to_vec();
        tagged.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        tagged.extend_from_slice(b"WEBP");
        tagged.extend(body);

        let cleaned = strip_webp(&tagged).unwrap();
        assert!(!cleaned.windows(4).any(|w| w == b"EXIF"));
        assert_eq!(cleaned[20] & 0x08, 0);
        assert_eq!(cleaned[20] & 0x10, 0x10);
        let riff = u32::from_le_bytes(cleaned[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, cleaned.len());
        assert_eq!(pixels(&cleaned), pixels(&plain));
    }

    #[test]
    fn gif_comments_go_and_the_loop_count_stays() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let mut plain = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut plain);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for i in 0..3u8 {
                let frame = RgbaImage::from_pixel(6, 6, Rgba([i * 80, 0, 0, 255]));
                encoder.encode_frame(image::Frame::new(frame)).unwrap();
            }
        }
        // a comment block straight after the header and colour table
        let flags = plain[10];
        let header = 13
            + if flags & 0x80 != 0 {
                3 * (1 << ((flags & 7) + 1))
            } else {
                0
            };
        let mut tagged = plain[..header].to_vec();
        tagged.extend_from_slice(&[0x21, 0xfe, 7]);
        tagged.extend_from_slice(b"private");
        tagged.push(0);
        tagged.extend_from_slice(&plain[header..]);

        let cleaned = strip_gif(&tagged).unwrap();
        assert!(!String::from_utf8_lossy(&cleaned).contains("private"));
        assert_eq!(cleaned, plain);
        let anim = crate::animation::read(Format::Gif, &cleaned).unwrap();
        assert_eq!((anim.frames, anim.loop_count), (3, 0));
    }

    #[test]
    fn damaged_files_are_refused_rather_than_half_written() {
        let jpeg = tagged_jpeg();
        assert!(strip_jpeg(&jpeg[..jpeg.len() / 3]).is_none());
        assert!(strip_png(b"\x89PNG\r\n\x1a\n\0\0").is_none());
        assert!(strip_gif(b"GIF89a\0\0").is_none());
        assert!(strip_webp(b"RIFF").is_none());
    }

    #[test]
    fn outputs_never_overwrite_anything() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        fs::write(&path, tagged_jpeg()).unwrap();
        let first = strip_metadata(&path, Format::Jpeg, None, &NoReporter, None).unwrap();
        assert_eq!(first.file_name().unwrap(), "photo-clean.jpg");
        let second = strip_metadata(&path, Format::Jpeg, None, &NoReporter, None).unwrap();
        assert_eq!(second.file_name().unwrap(), "photo-clean 2.jpg");
        assert_eq!(fs::read(&path).unwrap(), tagged_jpeg());
        let info = probe_image(&second, Format::Jpeg, None).unwrap();
        assert!(info.tags.iter().all(|t| t.key == "Orientation"));
        // no temp files left
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
    }

    #[test]
    fn video_metadata_and_gps_tracks_are_removed_by_ffmpeg() {
        let Some(tools) = testing::tools() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("holiday.mp4");
        testing::make(
            &tools,
            "-f lavfi -i testsrc2=s=64x48:d=1 -f lavfi -i sine=d=1 -c:v libx264 -c:a aac \
             -metadata title=Secret -metadata location=+51.5-000.1/ \
             -metadata:s:a:0 title=MyVoice",
            &path,
        );
        let before = probe_media(&tools, &path).unwrap();
        assert!(before.tags.iter().any(|t| t.value == "Secret"));
        let out =
            strip_metadata(&path, Format::Media, Some(&tools), &NoReporter, Some(1.0)).unwrap();
        assert_eq!(out.file_name().unwrap(), "holiday-clean.mp4");
        let after = probe_media(&tools, &out).unwrap();
        assert!(
            after
                .tags
                .iter()
                .all(|t| t.value != "Secret" && t.value != "MyVoice")
        );
        assert!(
            after
                .tags
                .iter()
                .all(|t| t.key != "location" && t.key != "encoder")
        );
        assert!(after.video.is_some() && after.audio.is_some());
    }

    #[test]
    fn formats_without_a_cleaner_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.psd");
        fs::write(&path, b"8BPS").unwrap();
        let err = strip_metadata(&path, Format::Psd, None, &NoReporter, None).unwrap_err();
        assert!(err.to_string().contains("Photoshop document"), "{err}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
