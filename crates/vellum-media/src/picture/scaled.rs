//! shrinking images, including while they decode so big files never need full size memory.

use std::fs::File;
use std::io::BufReader;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use ::image::imageops;
use ::image::metadata::Orientation;
use ::image::{DynamicImage, RgbaImage};

/// the size that fits inside a `size` by `size` square keeping the aspect ratio, never
/// bigger than the original and never zero
pub fn fit(width: u32, height: u32, size: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= size || longest == 0 {
        return (width.max(1), height.max(1));
    }
    let scale = f64::from(size) / f64::from(longest);
    let w = (f64::from(width) * scale).round().max(1.0) as u32;
    let h = (f64::from(height) * scale).round().max(1.0) as u32;
    (w, h)
}

/// shrinks to fit `size`, leaving smaller images alone. area averaging keeps fine detail
/// from turning into noise, which matters for thumbnails of photos and screenshots.
pub fn shrink(image: RgbaImage, size: u32) -> RgbaImage {
    let (w, h) = fit(image.width(), image.height(), size);
    if (w, h) == image.dimensions() {
        return image;
    }
    imageops::thumbnail(&image, w, h)
}

/// decodes a jpeg at 1/2, 1/4 or 1/8 size straight from the compressed data, which is
/// several times faster and smaller than decoding it whole. none means use the normal path.
pub fn jpeg(path: &Path, size: u32) -> Option<(RgbaImage, u32, u32)> {
    // jpeg-decoder panics on a few malformed files, so a panic here just means "fall back"
    let result = panic::catch_unwind(AssertUnwindSafe(|| jpeg_inner(path, size)));
    result.ok().flatten()
}

fn jpeg_inner(path: &Path, size: u32) -> Option<(RgbaImage, u32, u32)> {
    let file = File::open(path).ok()?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    let (full_w, full_h) = (u32::from(info.width), u32::from(info.height));
    if full_w.max(full_h) < size.saturating_mul(2) {
        return None;
    }
    let (want_w, want_h) = fit(full_w, full_h, size);
    decoder
        .scale(u16::try_from(want_w).ok()?, u16::try_from(want_h).ok()?)
        .ok()?;
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let rgba: Vec<u8> = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&l| [l, l, l, 255]).collect(),
        // cmyk and 16 bit jpegs are rare, the full decoder handles them properly
        _ => return None,
    };
    let image = RgbaImage::from_raw(w, h, rgba)?;
    let orientation = crate::probe::exif_orientation(path).unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::ImageRgba8(image);
    image.apply_orientation(orientation);
    let (full_w, full_h) = super::upright(full_w, full_h, orientation);
    Some((image.into_rgba8(), full_w, full_h))
}

/// decodes a png one row at a time, averaging blocks of pixels as it goes, so memory only
/// ever holds the small result. interlaced pngs arrive out of order and use the normal path.
pub fn png(path: &Path, size: u32) -> Option<(RgbaImage, u32, u32)> {
    let file = File::open(path).ok()?;
    let mut decoder = ::png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(::png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    if info.interlaced || w.max(h) < size.saturating_mul(2) {
        return None;
    }
    let orientation = info
        .exif_metadata
        .as_deref()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    let channels = match reader.output_color_type().0 {
        ::png::ColorType::Grayscale => 1,
        ::png::ColorType::GrayscaleAlpha => 2,
        ::png::ColorType::Rgb => 3,
        ::png::ColorType::Rgba => 4,
        ::png::ColorType::Indexed => return None,
    };

    // whole number block size so every output pixel averages the same area
    let factor = w.max(h).div_ceil(size).max(2);
    let out_w = w.div_ceil(factor);
    let out_h = h.div_ceil(factor);
    let mut out = Vec::with_capacity((out_w * out_h * 4) as usize);
    // per output column: alpha weighted red, green and blue, alpha, pixel count
    let mut sums = vec![[0_u64; 5]; out_w as usize];
    for y in 0..h {
        let row = reader.next_row().ok()??;
        for (x, px) in row.data().chunks_exact(channels).enumerate() {
            let [r, g, b, a] = match channels {
                1 => [px[0], px[0], px[0], 255],
                2 => [px[0], px[0], px[0], px[1]],
                3 => [px[0], px[1], px[2], 255],
                _ => [px[0], px[1], px[2], px[3]],
            };
            let a64 = u64::from(a);
            let cell = &mut sums[x / factor as usize];
            cell[0] += u64::from(r) * a64;
            cell[1] += u64::from(g) * a64;
            cell[2] += u64::from(b) * a64;
            cell[3] += a64;
            cell[4] += 1;
        }
        if (y + 1) % factor == 0 || y + 1 == h {
            for cell in &mut sums {
                let alpha = cell[3];
                let count = cell[4].max(1);
                let channel = |sum: u64| sum.checked_div(alpha).unwrap_or(0) as u8;
                out.extend([
                    channel(cell[0]),
                    channel(cell[1]),
                    channel(cell[2]),
                    (alpha / count) as u8,
                ]);
                *cell = [0; 5];
            }
        }
    }
    let image = RgbaImage::from_raw(out_w, out_h, out)?;
    let mut image = DynamicImage::ImageRgba8(image);
    image.apply_orientation(orientation);
    let (full_w, full_h) = super::upright(w, h, orientation);
    Some((image.into_rgba8(), full_w, full_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_aspect_ratio_and_never_grows() {
        assert_eq!(fit(4000, 3000, 400), (400, 300));
        assert_eq!(fit(3000, 4000, 400), (300, 400));
        assert_eq!(fit(200, 100, 400), (200, 100));
        assert_eq!(fit(10_000, 1, 100), (100, 1));
        assert_eq!(fit(0, 0, 100), (1, 1));
    }

    #[test]
    fn shrink_leaves_small_images_alone() {
        let image = RgbaImage::new(50, 20);
        assert_eq!(shrink(image, 100).dimensions(), (50, 20));
        let image = RgbaImage::new(500, 200);
        assert_eq!(shrink(image, 100).dimensions(), (100, 40));
    }
}
