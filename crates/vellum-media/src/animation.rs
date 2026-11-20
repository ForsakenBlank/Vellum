//! frame counts, durations and loop counts of gif, animated webp and apng, read from the
//! container without decoding a single pixel. a 200 frame gif probes in microseconds.

use crate::sniff::Format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Animation {
    pub frames: u32,
    /// one full play, in milliseconds
    pub duration_ms: u64,
    /// 0 plays forever, otherwise how many times it plays
    pub loop_count: u32,
}

/// none for stills and for anything that cannot be read
pub fn read(format: Format, bytes: &[u8]) -> Option<Animation> {
    let animation = match format {
        Format::Gif => gif(bytes),
        Format::WebP => webp(bytes),
        Format::Png => apng(bytes),
        _ => None,
    }?;
    (animation.frames > 1).then_some(animation)
}

/// browsers show gif delays under 2 centiseconds as 10, so vellum does too
fn gif_delay_ms(centis: u16) -> u64 {
    if centis < 2 {
        100
    } else {
        u64::from(centis) * 10
    }
}

fn gif(bytes: &[u8]) -> Option<Animation> {
    let mut pos = 6;
    let flags = *bytes.get(pos + 4)?;
    pos += 7;
    if flags & 0x80 != 0 {
        pos += 3 * (1 << ((flags & 0x07) + 1));
    }
    let mut frames = 0u32;
    let mut duration_ms = 0u64;
    let mut next_delay = 0u16;
    // no netscape block means the gif plays once
    let mut loop_count = 1u32;
    loop {
        match *bytes.get(pos)? {
            0x21 => {
                let label = *bytes.get(pos + 1)?;
                pos += 2;
                let first = bytes.get(pos + 1..)?;
                if label == 0xf9 {
                    next_delay = u16::from_le_bytes([*first.get(1)?, *first.get(2)?]);
                } else if label == 0xff && first.starts_with(b"NETSCAPE2.0") {
                    let sub = bytes.get(pos + 12..pos + 16)?;
                    if sub[0] == 3 && sub[1] == 1 {
                        let repeats = u16::from_le_bytes([sub[2], sub[3]]);
                        loop_count = if repeats == 0 {
                            0
                        } else {
                            u32::from(repeats) + 1
                        };
                    }
                }
                pos = skip_sub_blocks(bytes, pos)?;
            }
            0x2c => {
                frames += 1;
                duration_ms += gif_delay_ms(next_delay);
                next_delay = 0;
                let flags = *bytes.get(pos + 9)?;
                pos += 10;
                if flags & 0x80 != 0 {
                    pos += 3 * (1 << ((flags & 0x07) + 1));
                }
                // lzw minimum code size, then the image data
                pos = skip_sub_blocks(bytes, pos + 1)?;
            }
            0x3b => break,
            _ => return None,
        }
    }
    Some(Animation {
        frames,
        duration_ms,
        loop_count,
    })
}

/// gif data comes in blocks of up to 255 bytes, each led by its length, ended by a 0
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

fn webp(bytes: &[u8]) -> Option<Animation> {
    let mut pos = 12;
    let mut frames = 0u32;
    let mut duration_ms = 0u64;
    let mut loop_count = 0u32;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let data = bytes.get(pos + 8..(pos + 8 + size).min(bytes.len()))?;
        match id {
            b"ANIM" => loop_count = u32::from(u16::from_le_bytes([*data.get(4)?, *data.get(5)?])),
            b"ANMF" => {
                frames += 1;
                let d = data.get(12..15)?;
                duration_ms += u64::from(u32::from_le_bytes([d[0], d[1], d[2], 0]));
            }
            _ => {}
        }
        // chunks are padded to an even size
        pos += 8 + size + (size & 1);
    }
    Some(Animation {
        frames,
        duration_ms,
        loop_count,
    })
}

fn apng(bytes: &[u8]) -> Option<Animation> {
    let mut pos = 8;
    let mut declared = None;
    let mut loop_count = 0u32;
    let mut frames = 0u32;
    let mut duration_ms = 0u64;
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let id = &bytes[pos + 4..pos + 8];
        let data = bytes.get(pos + 8..pos + 8 + len)?;
        match id {
            b"acTL" => {
                declared = Some(u32::from_be_bytes(data.get(0..4)?.try_into().ok()?));
                loop_count = u32::from_be_bytes(data.get(4..8)?.try_into().ok()?);
            }
            b"fcTL" => {
                frames += 1;
                let num = u16::from_be_bytes(data.get(20..22)?.try_into().ok()?);
                let den = u16::from_be_bytes(data.get(22..24)?.try_into().ok()?);
                // a zero denominator means hundredths of a second
                let den = if den == 0 { 100 } else { den };
                duration_ms += u64::from(num) * 1000 / u64::from(den);
            }
            b"IDAT" if declared.is_none() => return None,
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    declared?;
    Some(Animation {
        frames,
        duration_ms,
        loop_count,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, RgbaImage};

    use super::*;

    fn gif_bytes(frames: u32, delay_ms: u32, repeat: Repeat) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            encoder.set_repeat(repeat).unwrap();
            for i in 0..frames {
                let image = RgbaImage::from_pixel(8, 8, image::Rgba([i as u8 * 40, 0, 0, 255]));
                let delay = Delay::from_numer_denom_ms(delay_ms, 1);
                encoder
                    .encode_frame(Frame::from_parts(image, 0, 0, delay))
                    .unwrap();
            }
        }
        out
    }

    #[test]
    fn gifs_report_frames_duration_and_loops() {
        let bytes = gif_bytes(5, 80, Repeat::Infinite);
        let anim = read(Format::Gif, &bytes).unwrap();
        assert_eq!(anim.frames, 5);
        assert_eq!(anim.duration_ms, 400);
        assert_eq!(anim.loop_count, 0);

        let anim = read(Format::Gif, &gif_bytes(3, 100, Repeat::Finite(2))).unwrap();
        assert_eq!(anim.loop_count, 3);
        assert!(read(Format::Gif, &gif_bytes(1, 100, Repeat::Infinite)).is_none());
    }

    #[test]
    fn tiny_gif_delays_play_like_browsers_do() {
        let anim = read(Format::Gif, &gif_bytes(4, 0, Repeat::Infinite)).unwrap();
        assert_eq!(anim.duration_ms, 400);
    }

    #[test]
    fn truncated_files_give_nothing_instead_of_panicking() {
        let bytes = gif_bytes(5, 80, Repeat::Infinite);
        for cut in [0, 5, 13, 40, bytes.len() / 2] {
            let _ = read(Format::Gif, &bytes[..cut]);
        }
        assert!(read(Format::WebP, b"RIFF").is_none());
        assert!(read(Format::Png, b"\x89PNG\r\n\x1a\n").is_none());
    }

    #[test]
    fn apng_frames_are_counted_and_plain_pngs_are_stills() {
        use image::ImageEncoder;
        use image::codecs::png::PngEncoder;
        let mut plain = Vec::new();
        PngEncoder::new(Cursor::new(&mut plain))
            .write_image(&[0u8; 4 * 4 * 4], 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert!(read(Format::Png, &plain).is_none());

        // a hand made apng: signature, ihdr, actl (2 frames, forever), two fctl, iend
        fn chunk(out: &mut Vec<u8>, id: &[u8], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(id);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0, 0, 0, 0]);
        }
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut bytes, b"IHDR", &[0; 13]);
        chunk(&mut bytes, b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]);
        let mut fctl = [0u8; 26];
        fctl[20..22].copy_from_slice(&1u16.to_be_bytes());
        fctl[22..24].copy_from_slice(&4u16.to_be_bytes());
        chunk(&mut bytes, b"fcTL", &fctl);
        chunk(&mut bytes, b"IDAT", &[]);
        chunk(&mut bytes, b"fcTL", &fctl);
        chunk(&mut bytes, b"fdAT", &[]);
        chunk(&mut bytes, b"IEND", &[]);
        let anim = read(Format::Png, &bytes).unwrap();
        assert_eq!(
            (anim.frames, anim.duration_ms, anim.loop_count),
            (2, 500, 0)
        );
    }
}
