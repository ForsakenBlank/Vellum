//! pixel buffers shared by both backends.

use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::rect::IRect;

/// how many bits the render graph keeps per channel between nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Precision {
    /// 8 bit unsigned. quick previews and proxies; bands in smooth gradients.
    U8,
    /// 16 bit float, the default
    #[default]
    F16,
    /// 32 bit float, for 16 and 32 bit documents and grading
    F32,
}

impl Precision {
    pub fn bytes_per_pixel(self) -> u64 {
        match self {
            Precision::U8 => 4,
            Precision::F16 => 8,
            Precision::F32 => 16,
        }
    }

    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Precision::U8 => wgpu::TextureFormat::Rgba8Unorm,
            Precision::F16 => wgpu::TextureFormat::Rgba16Float,
            Precision::F32 => wgpu::TextureFormat::Rgba32Float,
        }
    }

    /// rounds a value the way storage at this precision would
    pub fn quantize(self, v: f32) -> f32 {
        match self {
            Precision::U8 => (v.clamp(0.0, 1.0) * 255.0).round() / 255.0,
            Precision::F16 => half::f16::from_f32(v).to_f32(),
            Precision::F32 => v,
        }
    }
}

/// a premultiplied rgba image covering `rect`, in working space values.
/// pixels outside the rect are transparent.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuImage {
    pub rect: IRect,
    pub data: Vec<[f32; 4]>,
}

impl CpuImage {
    pub fn transparent(rect: IRect) -> Self {
        Self {
            rect,
            data: vec![[0.0; 4]; rect.area() as usize],
        }
    }

    pub fn filled(rect: IRect, color: [f32; 4]) -> Self {
        Self {
            rect,
            data: vec![color; rect.area() as usize],
        }
    }

    /// builds an image by calling `f` with the absolute coordinates of every pixel
    pub fn from_fn(rect: IRect, f: impl Fn(i32, i32) -> [f32; 4] + Sync) -> Self {
        let mut img = Self::transparent(rect);
        img.par_fill(f);
        img
    }

    /// overwrites every pixel from `f`, rows in parallel
    pub fn par_fill(&mut self, f: impl Fn(i32, i32) -> [f32; 4] + Sync) {
        let rect = self.rect;
        if rect.is_empty() {
            return;
        }
        self.data
            .par_chunks_mut(rect.w as usize)
            .enumerate()
            .for_each(|(row, line)| {
                let y = rect.y + row as i32;
                for (col, px) in line.iter_mut().enumerate() {
                    *px = f(rect.x + col as i32, y);
                }
            });
    }

    /// the pixel at absolute coordinates, transparent outside
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> [f32; 4] {
        let r = self.rect;
        if x < r.x || y < r.y || x >= r.right() || y >= r.bottom() {
            return [0.0; 4];
        }
        self.data[((y - r.y) as u32 * r.w + (x - r.x) as u32) as usize]
    }

    /// the pixel at absolute coordinates, clamped to the nearest edge
    #[inline]
    pub fn get_clamped(&self, x: i32, y: i32) -> [f32; 4] {
        let r = self.rect;
        if r.is_empty() {
            return [0.0; 4];
        }
        let x = x.clamp(r.x, r.right() - 1);
        let y = y.clamp(r.y, r.bottom() - 1);
        self.data[((y - r.y) as u32 * r.w + (x - r.x) as u32) as usize]
    }

    /// a copy covering exactly `rect`, transparent where this image has nothing
    pub fn crop(&self, rect: IRect) -> CpuImage {
        if rect == self.rect {
            return self.clone();
        }
        let mut out = CpuImage::transparent(rect);
        let both = rect.intersect(&self.rect);
        for y in both.y..both.bottom() {
            let src =
                ((y - self.rect.y) as u32 * self.rect.w + (both.x - self.rect.x) as u32) as usize;
            let dst = ((y - rect.y) as u32 * rect.w + (both.x - rect.x) as u32) as usize;
            out.data[dst..dst + both.w as usize]
                .copy_from_slice(&self.data[src..src + both.w as usize]);
        }
        out
    }

    /// writes `other` into this image where they overlap
    pub fn paste(&mut self, other: &CpuImage) {
        let both = other.rect.intersect(&self.rect);
        for y in both.y..both.bottom() {
            let src = ((y - other.rect.y) as u32 * other.rect.w + (both.x - other.rect.x) as u32)
                as usize;
            let dst =
                ((y - self.rect.y) as u32 * self.rect.w + (both.x - self.rect.x) as u32) as usize;
            self.data[dst..dst + both.w as usize]
                .copy_from_slice(&other.data[src..src + both.w as usize]);
        }
    }

    pub fn bytes(&self) -> u64 {
        self.data.len() as u64 * 16
    }

    /// rounds every value the way storage at `precision` would, so cpu results can be
    /// compared with gpu results at that precision
    pub fn quantized(mut self, precision: Precision) -> Self {
        if precision != Precision::F32 {
            for px in &mut self.data {
                *px = px.map(|v| precision.quantize(v));
            }
        }
        self
    }
}

/// how source pixels are stored before they enter the graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceFormat {
    /// four bytes per pixel
    Rgba8,
    /// four 16 bit values per pixel, as photoshop's 16 bit mode stores them
    Rgba16,
    /// four 32 bit floats per pixel
    Rgba32F,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlphaMode {
    Straight,
    Premultiplied,
    /// no alpha channel worth reading; treated as fully opaque
    Opaque,
}

/// raw pixels handed to the graph by a source: a decoded photo, a layer tile, a video
/// frame. they are converted to working space by the ingest kernel.
#[derive(Debug, Clone)]
pub struct SourcePixels {
    pub rect: IRect,
    pub format: SourceFormat,
    pub alpha: AlphaMode,
    pub data: Arc<SourceData>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceData {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

impl SourcePixels {
    pub fn rgba8(rect: IRect, alpha: AlphaMode, data: Vec<u8>) -> Self {
        Self {
            rect,
            format: SourceFormat::Rgba8,
            alpha,
            data: Arc::new(SourceData::U8(data)),
        }
    }

    pub fn rgba16(rect: IRect, alpha: AlphaMode, data: Vec<u16>) -> Self {
        Self {
            rect,
            format: SourceFormat::Rgba16,
            alpha,
            data: Arc::new(SourceData::U16(data)),
        }
    }

    pub fn rgba32f(rect: IRect, alpha: AlphaMode, data: Vec<f32>) -> Self {
        Self {
            rect,
            format: SourceFormat::Rgba32F,
            alpha,
            data: Arc::new(SourceData::F32(data)),
        }
    }

    /// the pixel at an index as floats in [0, 1] (or beyond for float data), as stored
    #[inline]
    pub fn get(&self, i: usize) -> [f32; 4] {
        let j = i * 4;
        let px = match &*self.data {
            SourceData::U8(d) => [
                f32::from(d[j]) / 255.0,
                f32::from(d[j + 1]) / 255.0,
                f32::from(d[j + 2]) / 255.0,
                f32::from(d[j + 3]) / 255.0,
            ],
            SourceData::U16(d) => [
                f32::from(d[j]) / 65535.0,
                f32::from(d[j + 1]) / 65535.0,
                f32::from(d[j + 2]) / 65535.0,
                f32::from(d[j + 3]) / 65535.0,
            ],
            SourceData::F32(d) => [d[j], d[j + 1], d[j + 2], d[j + 3]],
        };
        if self.alpha == AlphaMode::Opaque {
            [px[0], px[1], px[2], 1.0]
        } else {
            px
        }
    }

    pub fn len(&self) -> usize {
        self.rect.area() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.rect.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        match &*self.data {
            SourceData::U8(d) => d.len() as u64,
            SourceData::U16(d) => d.len() as u64 * 2,
            SourceData::F32(d) => d.len() as u64 * 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_and_paste() {
        let img = CpuImage::from_fn(IRect::new(0, 0, 4, 4), |x, y| {
            [x as f32, y as f32, 0.0, 1.0]
        });
        let c = img.crop(IRect::new(2, 2, 4, 4));
        assert_eq!(c.get(3, 3), [3.0, 3.0, 0.0, 1.0]);
        assert_eq!(c.get(5, 5), [0.0; 4]);
        let mut big = CpuImage::transparent(IRect::new(-2, -2, 8, 8));
        big.paste(&img);
        assert_eq!(big.get(1, 2), [1.0, 2.0, 0.0, 1.0]);
        assert_eq!(big.get(-1, -1), [0.0; 4]);
        assert_eq!(img.get_clamped(-5, 9), [0.0, 3.0, 0.0, 1.0]);
    }

    #[test]
    fn source_pixels_read_as_floats() {
        let p = SourcePixels::rgba16(
            IRect::new(0, 0, 1, 1),
            AlphaMode::Straight,
            vec![65535, 0, 32768, 65535],
        );
        let px = p.get(0);
        assert!((px[2] - 0.5).abs() < 1e-4 && px[0] == 1.0);
        let p = SourcePixels::rgba8(
            IRect::new(0, 0, 1, 1),
            AlphaMode::Opaque,
            vec![255, 0, 0, 7],
        );
        assert_eq!(p.get(0)[3], 1.0);
    }
}
