//! working on a gif frame by frame: the frame editor (delete, duplicate, reorder, change a
//! frame's delay) and the splitter (every frame as a png).
//!
//! the editor writes gifs itself rather than through ffmpeg, so every delay lands exactly
//! as the user typed it. the frames came from a gif, so their colours usually fit one
//! palette exactly and nothing is lost; only when frames used different palettes that add
//! up to more than 255 colours is a shared palette worked out.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use color_quant::NeuQuant;
use gif::{DisposalMethod, Encoder, Frame, Repeat};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_media::picture::{self, DecodeOptions};
use vellum_media::{Control, MediaError, MediaResult};

use super::lossy;

/// one frame of the edited gif: which source frame it shows and for how long
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditedFrame {
    pub source: u32,
    pub delay_ms: u32,
}

/// the frames of a gif, for the editor to start from
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GifFrames {
    pub width: u32,
    pub height: u32,
    pub delays_ms: Vec<u32>,
    /// how many times it plays, 0 for forever
    pub loops: u32,
}

/// browsers show anything quicker than this at 100 ms, so the editor warns below it
pub const MIN_SAFE_DELAY_MS: u32 = 20;

pub fn read_frames(path: &Path) -> MediaResult<(GifFrames, Vec<RgbaImage>)> {
    let picture = picture::decode(
        path,
        DecodeOptions {
            size: None,
            all_frames: true,
        },
        None,
    )?;
    let loops = read_loops(path).unwrap_or(0);
    let delays_ms = picture.frames.iter().map(|f| f.delay_ms).collect();
    let frames = picture.frames.into_iter().map(|f| f.image).collect();
    Ok((
        GifFrames {
            width: picture.width,
            height: picture.height,
            delays_ms,
            loops,
        },
        frames,
    ))
}

/// the loop count stored in a gif. none for files that are not gifs.
fn read_loops(path: &Path) -> Option<u32> {
    let data = std::fs::read(path).ok()?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut decoder = options.read_info(Cursor::new(&data)).ok()?;
    // the loop count comes after the first frame in most files
    while decoder.read_next_frame().ok()?.is_some() {}
    Some(match decoder.repeat() {
        Repeat::Infinite => 0,
        Repeat::Finite(n) => u32::from(n) + 1,
    })
}

/// writes `edit` of `input` as a new gif
pub fn edit_gif(
    input: &Path,
    edit: &[EditedFrame],
    loops: u32,
    output: &Path,
    control: &dyn Control,
) -> MediaResult<()> {
    if edit.is_empty() {
        return Err(MediaError::Unsupported(
            "A GIF needs at least one frame. Put a frame back before saving.".to_owned(),
        ));
    }
    let (info, frames) = read_frames(input)?;
    control.progress(0.2);
    let mut chosen = Vec::with_capacity(edit.len());
    for entry in edit {
        let image = frames.get(entry.source as usize).ok_or_else(|| {
            MediaError::Unsupported(format!(
                "Frame {} is not in this GIF, which has {} frames.",
                entry.source + 1,
                frames.len()
            ))
        })?;
        chosen.push((image, entry.delay_ms));
    }
    let data = encode_frames(&chosen, info.width, info.height, loops, control)?;
    control.progress(0.8);
    // only identical pixels are skipped here, so the picture is exactly the same
    let data = lossy::compress(&data, 0, control)?;
    crate::write_atomic(output, &data)?;
    control.progress(1.0);
    Ok(())
}

/// writes full frames into a gif with one palette shared by every frame
pub fn encode_frames(
    frames: &[(&RgbaImage, u32)],
    width: u32,
    height: u32,
    loops: u32,
    control: &dyn Control,
) -> MediaResult<Vec<u8>> {
    let (w, h) = (
        u16::try_from(width).map_err(|_| too_big())?,
        u16::try_from(height).map_err(|_| too_big())?,
    );
    let has_holes = frames
        .iter()
        .any(|(image, _)| image.pixels().any(|p| p[3] < 128));
    let palette = shared_palette(frames);
    let lookup: HashMap<[u8; 3], u8> = palette
        .colors
        .iter()
        .enumerate()
        .map(|(i, c)| (*c, i as u8))
        .collect();
    let transparent = palette.colors.len() as u8;
    let mut flat: Vec<u8> = palette.colors.iter().flatten().copied().collect();
    flat.extend([0, 0, 0]);

    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, w, h, &flat).map_err(gif_error)?;
        let repeat = match loops {
            0 => Repeat::Infinite,
            n => Repeat::Finite(u16::try_from(n - 1).unwrap_or(u16::MAX)),
        };
        encoder.set_repeat(repeat).map_err(gif_error)?;
        let mut cache: HashMap<[u8; 3], u8> = HashMap::new();
        for (image, delay_ms) in frames {
            control.checkpoint()?;
            let indices: Vec<u8> = image
                .pixels()
                .map(|p| {
                    if p[3] < 128 {
                        return transparent;
                    }
                    let rgb = [p[0], p[1], p[2]];
                    if let Some(i) = lookup.get(&rgb) {
                        return *i;
                    }
                    *cache.entry(rgb).or_insert_with(|| palette.nearest(rgb))
                })
                .collect();
            let frame = Frame {
                width: w,
                height: h,
                // gif delays are in hundredths of a second
                delay: u16::try_from((delay_ms + 5) / 10).unwrap_or(u16::MAX),
                // see-through gifs must clear each frame, or old frames show through
                dispose: if has_holes {
                    DisposalMethod::Background
                } else {
                    DisposalMethod::Keep
                },
                transparent: has_holes.then_some(transparent),
                buffer: indices.into(),
                ..Frame::default()
            };
            encoder.write_frame(&frame).map_err(gif_error)?;
        }
    }
    Ok(out)
}

struct Palette {
    colors: Vec<[u8; 3]>,
    quant: Option<NeuQuant>,
}

impl Palette {
    fn nearest(&self, rgb: [u8; 3]) -> u8 {
        if let Some(quant) = &self.quant {
            return quant.index_of(&[rgb[0], rgb[1], rgb[2], 255]) as u8;
        }
        let distance = |c: &[u8; 3]| {
            (0..3)
                .map(|i| (i32::from(c[i]) - i32::from(rgb[i])).pow(2))
                .sum::<i32>()
        };
        self.colors
            .iter()
            .enumerate()
            .min_by_key(|(_, c)| distance(c))
            .map_or(0, |(i, _)| i as u8)
    }
}

/// every colour exactly if they fit in 255 (one slot stays free for transparency),
/// otherwise a palette learned from all the frames
fn shared_palette(frames: &[(&RgbaImage, u32)]) -> Palette {
    let mut seen: Vec<[u8; 3]> = Vec::new();
    let mut set: std::collections::HashSet<[u8; 3]> = std::collections::HashSet::new();
    'frames: for (image, _) in frames {
        for p in image.pixels().filter(|p| p[3] >= 128) {
            let rgb = [p[0], p[1], p[2]];
            if set.insert(rgb) {
                seen.push(rgb);
                if seen.len() > 255 {
                    break 'frames;
                }
            }
        }
    }
    if seen.len() <= 255 {
        if seen.is_empty() {
            seen.push([0, 0, 0]);
        }
        return Palette {
            colors: seen,
            quant: None,
        };
    }
    // learn from a sample of every frame so no frame's colours are left out
    let mut sample = Vec::new();
    let step = (frames.len() / 32).max(1);
    for (image, _) in frames.iter().step_by(step) {
        let raw = image.as_raw();
        let every = (raw.len() / 4 / 20_000).max(1) * 4;
        for px in raw.chunks_exact(4).step_by(every / 4) {
            sample.extend([px[0], px[1], px[2], 255]);
        }
    }
    let quant = NeuQuant::new(3, 255, &sample);
    let colors = quant
        .color_map_rgba()
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    Palette {
        colors,
        quant: Some(quant),
    }
}

/// every frame as `frame 001.png` and so on in `folder`, which is made if needed.
/// returns how many were written.
pub fn split(input: &Path, folder: &Path, control: &dyn Control) -> MediaResult<usize> {
    let (_, frames) = read_frames(input)?;
    std::fs::create_dir_all(folder).map_err(|err| MediaError::io(folder, err))?;
    let digits = frames.len().to_string().len().max(3);
    for (i, frame) in frames.iter().enumerate() {
        control.checkpoint()?;
        let path = folder.join(format!("frame {:0digits$}.png", i + 1));
        frame.save(&path).map_err(|err| {
            MediaError::Unsupported(format!("Frame {} could not be saved: {err}", i + 1))
        })?;
        control.progress((i + 1) as f32 / frames.len() as f32);
    }
    Ok(frames.len())
}

fn too_big() -> MediaError {
    MediaError::Unsupported("GIFs can be at most 65535 pixels on a side.".to_owned())
}

fn gif_error(err: impl std::fmt::Display) -> MediaError {
    MediaError::Decode(format!("The GIF could not be written: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use vellum_media::Unattended;

    fn solid(rgb: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(8, 6, Rgba([rgb[0], rgb[1], rgb[2], 255]))
    }

    fn write_sample(dir: &Path) -> std::path::PathBuf {
        let frames = [solid([255, 0, 0]), solid([0, 255, 0]), solid([0, 0, 255])];
        let list: Vec<(&RgbaImage, u32)> = frames.iter().map(|f| (f, 100)).collect();
        let data = encode_frames(&list, 8, 6, 3, &Unattended).unwrap();
        let path = dir.join("rgb.gif");
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn edits_reorder_duplicate_drop_and_retime() {
        let dir = tempfile::tempdir().unwrap();
        let input = write_sample(dir.path());
        let (info, frames) = read_frames(&input).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(info.delays_ms, vec![100, 100, 100]);
        assert_eq!(info.loops, 3);

        let edit = [
            EditedFrame {
                source: 2,
                delay_ms: 250,
            },
            EditedFrame {
                source: 0,
                delay_ms: 40,
            },
            EditedFrame {
                source: 0,
                delay_ms: 70,
            },
        ];
        let output = dir.path().join("edited.gif");
        edit_gif(&input, &edit, 0, &output, &Unattended).unwrap();
        let (info, frames) = read_frames(&output).unwrap();
        assert_eq!(info.loops, 0);
        assert_eq!(frames[0].get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(frames.last().unwrap().get_pixel(4, 4).0, [255, 0, 0, 255]);
        // the duplicate is identical, so the optimiser may fold it into the frame before;
        // the total time is what the user set either way
        assert_eq!(info.delays_ms.iter().sum::<u32>(), 360);
        assert_eq!(info.delays_ms[0], 250);
    }

    #[test]
    fn nothing_left_is_refused_and_bad_frames_are_named() {
        let dir = tempfile::tempdir().unwrap();
        let input = write_sample(dir.path());
        let out = dir.path().join("x.gif");
        assert!(edit_gif(&input, &[], 0, &out, &Unattended).is_err());
        let err = edit_gif(
            &input,
            &[EditedFrame {
                source: 9,
                delay_ms: 10,
            }],
            0,
            &out,
            &Unattended,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(err, "Frame 10 is not in this GIF, which has 3 frames.");
        assert!(!out.exists());
    }

    #[test]
    fn transparency_survives() {
        let mut hole = solid([10, 20, 30]);
        hole.put_pixel(1, 1, Rgba([0, 0, 0, 0]));
        let frames = [(&hole, 50), (&hole, 50)];
        let data = encode_frames(&frames, 8, 6, 0, &Unattended).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hole.gif");
        std::fs::write(&path, data).unwrap();
        let (_, back) = read_frames(&path).unwrap();
        assert_eq!(back[1].get_pixel(1, 1)[3], 0);
        assert_eq!(back[1].get_pixel(2, 2).0, [10, 20, 30, 255]);
    }

    #[test]
    fn many_colours_get_a_learned_palette() {
        let mut image = RgbaImage::new(32, 32);
        for (x, y, p) in image.enumerate_pixels_mut() {
            *p = Rgba([(x * 8) as u8, (y * 8) as u8, 128, 255]);
        }
        let frames = [(&image, 100)];
        let data = encode_frames(&frames, 32, 32, 0, &Unattended).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("many.gif");
        std::fs::write(&path, data).unwrap();
        let (_, back) = read_frames(&path).unwrap();
        let p = back[0].get_pixel(16, 16).0;
        assert!(
            (i32::from(p[0]) - 128).abs() < 24 && (i32::from(p[1]) - 128).abs() < 24,
            "{p:?}"
        );
    }

    #[test]
    fn splitting_writes_numbered_pngs() {
        let dir = tempfile::tempdir().unwrap();
        let input = write_sample(dir.path());
        let folder = dir.path().join("frames");
        assert_eq!(split(&input, &folder, &Unattended).unwrap(), 3);
        assert!(folder.join("frame 001.png").is_file());
        assert!(folder.join("frame 003.png").is_file());
    }
}
