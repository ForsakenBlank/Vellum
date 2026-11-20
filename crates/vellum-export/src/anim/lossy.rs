//! "lossy" gif compression: pixels that barely changed since the last frame are left
//! transparent, so the old pixel keeps showing. long runs of the same transparent index
//! compress far better, and the eye does not notice a colour that is off by a few steps.
//!
//! the comparison is always against what the viewer is actually seeing, not the source's
//! previous frame, so small differences cannot pile up into a visible smear over time.
//!
//! gifsicle's `--lossy` (which bends the lzw dictionary) is gpl, so this is vellum's own
//! take, which works on the frames ffmpeg's palette step writes.

use std::collections::HashMap;
use std::io::Cursor;

use gif::{ColorOutput, DecodeOptions, DisposalMethod, Encoder, Frame};
use vellum_media::{Control, MediaError, MediaResult};

/// `level` 0 to 100 becomes a colour distance; 100 lets colours drift by up to 40 steps
fn tolerance(level: u32) -> u32 {
    let distance = level.min(100) * 2 / 5;
    distance * distance
}

struct Decoded {
    width: u16,
    height: u16,
    palette: Vec<[u8; 3]>,
    transparent: Option<u8>,
    repeat: gif::Repeat,
}

/// re-encodes a gif with lossy inter frame compression. level 0 only skips pixels that are
/// exactly the same, which loses nothing. returns the input unchanged when it cannot help:
/// a single frame, see-through areas, no free palette slot, or a result that is not smaller.
pub fn compress(data: &[u8], level: u32, control: &dyn Control) -> MediaResult<Vec<u8>> {
    let threshold = tolerance(level);
    let Some(info) = inspect(data)? else {
        return Ok(data.to_vec());
    };
    let pixels = usize::from(info.width) * usize::from(info.height);
    let lookup: HashMap<[u8; 3], u8> = info
        .palette
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i as u8) != info.transparent)
        .map(|(i, rgb)| (*rgb, i as u8))
        .rev()
        .collect();
    let mut nearest_cache: HashMap<[u8; 3], u8> = HashMap::new();
    let mut index_of = |rgb: [u8; 3]| -> u8 {
        if let Some(i) = lookup.get(&rgb) {
            return *i;
        }
        *nearest_cache
            .entry(rgb)
            .or_insert_with(|| nearest(&info.palette, info.transparent, rgb))
    };

    let mut palette = info.palette.clone();
    let transparent = match info.transparent {
        Some(t) => t,
        None => {
            palette.push([0, 0, 0]);
            (palette.len() - 1) as u8
        }
    };
    let flat: Vec<u8> = palette.iter().flatten().copied().collect();
    let mut out = Vec::with_capacity(data.len());
    {
        let mut encoder =
            Encoder::new(&mut out, info.width, info.height, &flat).map_err(gif_error)?;
        encoder.set_repeat(info.repeat).map_err(gif_error)?;

        let mut source = Composer::new(data, info.width, info.height)?;
        let mut shown: Vec<[u8; 3]> = vec![[0, 0, 0]; pixels];
        let mut pending: Option<Frame<'static>> = None;
        let mut first = true;
        while let Some((rgb, delay)) = source.next()? {
            control.checkpoint()?;
            let frame = if first {
                first = false;
                shown.copy_from_slice(&rgb);
                let indices: Vec<u8> = rgb.iter().map(|c| index_of(*c)).collect();
                Some(frame_of(
                    0,
                    0,
                    info.width,
                    info.height,
                    indices,
                    delay,
                    transparent,
                ))
            } else {
                let mut changed = vec![false; pixels];
                let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
                let w = usize::from(info.width);
                for (p, (now, seen)) in rgb.iter().zip(shown.iter_mut()).enumerate() {
                    if distance(*now, *seen) > threshold {
                        changed[p] = true;
                        *seen = *now;
                        let (x, y) = (p % w, p / w);
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x);
                        y1 = y1.max(y);
                    }
                }
                if x0 == usize::MAX {
                    None
                } else {
                    let mut indices = Vec::with_capacity((x1 - x0 + 1) * (y1 - y0 + 1));
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let p = y * w + x;
                            indices.push(if changed[p] {
                                index_of(rgb[p])
                            } else {
                                transparent
                            });
                        }
                    }
                    // the rectangle is inside a gif, so it fits in u16
                    let rect = (
                        x0 as u16,
                        y0 as u16,
                        (x1 - x0 + 1) as u16,
                        (y1 - y0 + 1) as u16,
                    );
                    Some(frame_of(
                        rect.0,
                        rect.1,
                        rect.2,
                        rect.3,
                        indices,
                        delay,
                        transparent,
                    ))
                }
            };
            match frame {
                Some(frame) => {
                    if let Some(done) = pending.replace(frame) {
                        encoder.write_frame(&done).map_err(gif_error)?;
                    }
                }
                // nothing visible changed, so the previous frame simply stays up longer
                None => {
                    if let Some(prev) = pending.as_mut() {
                        prev.delay = prev.delay.saturating_add(delay);
                    }
                }
            }
        }
        if let Some(done) = pending {
            encoder.write_frame(&done).map_err(gif_error)?;
        }
    }
    // never make it bigger
    if out.len() >= data.len() {
        return Ok(data.to_vec());
    }
    Ok(out)
}

fn frame_of(
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    indices: Vec<u8>,
    delay: u16,
    transparent: u8,
) -> Frame<'static> {
    Frame {
        left,
        top,
        width,
        height,
        delay,
        dispose: DisposalMethod::Keep,
        transparent: Some(transparent),
        buffer: indices.into(),
        ..Frame::default()
    }
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    let d = |x: u8, y: u8| {
        let v = i32::from(x) - i32::from(y);
        // a square of at most 255, always fits
        (v * v) as u32
    };
    d(a[0], b[0]) + d(a[1], b[1]) + d(a[2], b[2])
}

fn nearest(palette: &[[u8; 3]], skip: Option<u8>, rgb: [u8; 3]) -> u8 {
    palette
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i as u8) != skip)
        .min_by_key(|(_, c)| distance(**c, rgb))
        .map_or(0, |(i, _)| i as u8)
}

fn gif_error(err: impl std::fmt::Display) -> MediaError {
    MediaError::Decode(format!("The GIF could not be compressed: {err}"))
}

/// the first pass: palette, size, loop count, and whether this gif can be handled
fn inspect(data: &[u8]) -> MediaResult<Option<Decoded>> {
    let mut options = DecodeOptions::new();
    options.set_color_output(ColorOutput::Indexed);
    let mut decoder = options.read_info(Cursor::new(data)).map_err(gif_error)?;
    let global = decoder.global_palette().map(<[u8]>::to_vec);
    let (width, height) = (decoder.width(), decoder.height());
    let mut palette: Option<Vec<u8>> = global;
    let mut transparent = None;
    let mut used = [false; 256];
    let mut frames = 0;
    while let Some(frame) = decoder.read_next_frame().map_err(gif_error)? {
        frames += 1;
        if palette.is_none() {
            palette.clone_from(&frame.palette);
        }
        if transparent.is_none() {
            transparent = frame.transparent;
        }
        for index in frame.buffer.iter() {
            used[usize::from(*index)] = true;
        }
    }
    let repeat = decoder.repeat();
    let Some(palette) = palette else {
        return Ok(None);
    };
    let palette: Vec<[u8; 3]> = palette
        .chunks_exact(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    // encoders pad palettes to a power of two, so a full looking palette often has slots
    // no pixel uses, and one of those can be the transparent colour
    if transparent.is_none() && palette.len() >= 256 {
        transparent = (0..palette.len()).find(|i| !used[*i]).map(|i| i as u8);
    }
    if frames < 2 || (transparent.is_none() && palette.len() >= 256) {
        return Ok(None);
    }
    // a transparent index past the palette would need a palette entry it does not have
    if transparent.is_some_and(|t| usize::from(t) >= palette.len()) {
        return Ok(None);
    }
    let mut composer = Composer::new(data, width, height)?;
    while let Some(_frame) = composer.next()? {
        if composer.saw_holes {
            return Ok(None);
        }
    }
    Ok(Some(Decoded {
        width,
        height,
        palette,
        transparent,
        repeat,
    }))
}

/// replays a gif into whole frames, following each frame's disposal
struct Composer<'a> {
    decoder: gif::Decoder<Cursor<&'a [u8]>>,
    width: usize,
    canvas: Vec<Option<[u8; 3]>>,
    global: Option<Vec<u8>>,
    /// a pixel was still see-through after drawing, so the gif has transparent areas
    saw_holes: bool,
}

impl<'a> Composer<'a> {
    fn new(data: &'a [u8], width: u16, height: u16) -> MediaResult<Self> {
        let mut options = DecodeOptions::new();
        options.set_color_output(ColorOutput::Indexed);
        let decoder = options.read_info(Cursor::new(data)).map_err(gif_error)?;
        let global = decoder.global_palette().map(<[u8]>::to_vec);
        Ok(Self {
            decoder,
            width: usize::from(width),
            canvas: vec![None; usize::from(width) * usize::from(height)],
            global,
            saw_holes: false,
        })
    }

    fn next(&mut self) -> MediaResult<Option<(Vec<[u8; 3]>, u16)>> {
        let Some(frame) = self.decoder.read_next_frame().map_err(gif_error)? else {
            return Ok(None);
        };
        let palette = frame
            .palette
            .as_deref()
            .or(self.global.as_deref())
            .unwrap_or(&[]);
        let before = (frame.dispose == DisposalMethod::Previous).then(|| self.canvas.clone());
        let (left, top) = (usize::from(frame.left), usize::from(frame.top));
        let fw = usize::from(frame.width);
        let height = self.canvas.len() / self.width.max(1);
        for (i, index) in frame.buffer.iter().enumerate() {
            if Some(*index) == frame.transparent {
                continue;
            }
            let (x, y) = (left + i % fw.max(1), top + i / fw.max(1));
            if x >= self.width || y >= height {
                continue;
            }
            let at = usize::from(*index) * 3;
            if let Some(rgb) = palette.get(at..at + 3) {
                self.canvas[y * self.width + x] = Some([rgb[0], rgb[1], rgb[2]]);
            }
        }
        if self.canvas.iter().any(Option::is_none) {
            self.saw_holes = true;
        }
        let image: Vec<[u8; 3]> = self.canvas.iter().map(|p| p.unwrap_or([0, 0, 0])).collect();
        let delay = frame.delay;
        match frame.dispose {
            DisposalMethod::Background => {
                let fh = usize::from(frame.height);
                for y in top..(top + fh).min(height) {
                    for x in left..(left + fw).min(self.width) {
                        self.canvas[y * self.width + x] = None;
                    }
                }
            }
            DisposalMethod::Previous => {
                if let Some(before) = before {
                    self.canvas = before;
                }
            }
            _ => {}
        }
        Ok(Some((image, delay)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_media::Unattended;

    /// 16x16 frames of a grey ramp that brightens by `step` every frame, plus a red square
    /// that moves, so some pixels change a lot and the rest only a little
    fn sample(step: u8) -> Vec<u8> {
        let mut palette = Vec::new();
        for i in 0..=250u8 {
            palette.extend([i, i, i]);
        }
        palette.extend([255, 0, 0]);
        let mut out = Vec::new();
        {
            let mut encoder = Encoder::new(&mut out, 16, 16, &palette).unwrap();
            encoder.set_repeat(gif::Repeat::Infinite).unwrap();
            for f in 0..8u8 {
                let mut indices = Vec::new();
                for y in 0..16u8 {
                    for x in 0..16u8 {
                        let red = (x / 4) == f % 4 && y < 4;
                        indices.push(if red {
                            251
                        } else {
                            (u16::from(x) * 8 + u16::from(f) * u16::from(step)).min(250) as u8
                        });
                    }
                }
                let frame = Frame {
                    width: 16,
                    height: 16,
                    delay: 5,
                    buffer: indices.into(),
                    ..Frame::default()
                };
                encoder.write_frame(&frame).unwrap();
            }
        }
        out
    }

    fn frames(data: &[u8]) -> Vec<(Vec<[u8; 3]>, u16)> {
        let mut composer = Composer::new(data, 16, 16).unwrap();
        let mut all = Vec::new();
        while let Some(frame) = composer.next().unwrap() {
            all.push(frame);
        }
        all
    }

    #[test]
    fn small_changes_are_dropped_and_big_ones_kept() {
        let input = sample(2);
        let output = compress(&input, 60, &Unattended).unwrap();
        assert!(
            output.len() < input.len(),
            "{} vs {}",
            output.len(),
            input.len()
        );
        let before = frames(&input);
        let after = frames(&output);
        // the timing is the same in total
        let total = |f: &[(Vec<[u8; 3]>, u16)]| f.iter().map(|(_, d)| u32::from(*d)).sum::<u32>();
        assert_eq!(total(&before), total(&after));
        // the red square still moves exactly
        let last = &after.last().unwrap().0;
        assert_eq!(last[3 * 4], [255, 0, 0]);
        // and the grey is never further off than the tolerance allows
        let (src, _) = before.last().unwrap();
        for (a, b) in src.iter().zip(last) {
            assert!(distance(*a, *b) <= tolerance(60), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn level_zero_keeps_every_pixel() {
        let input = sample(30);
        let output = compress(&input, 0, &Unattended).unwrap();
        let before = frames(&input);
        let after = frames(&output);
        assert_eq!(before.last().unwrap().0, after.last().unwrap().0);
    }

    #[test]
    fn frames_with_nothing_new_fold_into_the_last() {
        let input = sample(0);
        let output = compress(&input, 100, &Unattended).unwrap();
        let after = frames(&output);
        // only the square moves, four positions in eight frames
        assert!(after.len() <= 8);
        assert_eq!(after.iter().map(|(_, d)| u32::from(*d)).sum::<u32>(), 40);
    }
}
