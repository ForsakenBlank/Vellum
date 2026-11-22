//! selection maths. a selection is a grey mask (1 selected, 0 not, anything between is
//! partly selected), stored as a sparse [`Raster`] so a select all on a huge canvas costs
//! almost nothing.
//!
//! shapes (rectangles, ellipses, lasso polygons) are coverage functions evaluated per
//! pixel straight into tiles. operations that look at neighbours (feather, expand, contract,
//! the magic wand) work on a dense [`GrayBuf`] over the area they touch.
//!
//! expand, contract, border and smooth all go through one signed distance field, so they
//! grow in true circles and keep anti aliased edges smooth.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::IRect;

use crate::raster::{Depth, Raster};

/// the biggest area a neighbourhood operation works on in one piece, about 400 megapixels
pub const MAX_DENSE: u64 = 400_000_000;

/// how a new selection meets the one already there
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Combine {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

/// a dense grey buffer over a rectangle, values outside read as zero
#[derive(Debug, Clone, PartialEq)]
pub struct GrayBuf {
    pub rect: IRect,
    pub data: Vec<f32>,
}

impl GrayBuf {
    pub fn new(rect: IRect) -> Self {
        Self {
            rect,
            data: vec![0.0; rect.area() as usize],
        }
    }

    pub fn from_raster(raster: &Raster, rect: IRect) -> Self {
        Self {
            rect,
            data: raster.read(rect),
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        if !self.rect.contains(x, y) {
            return 0.0;
        }
        self.data[((y - self.rect.y) as u32 * self.rect.w + (x - self.rect.x) as u32) as usize]
    }

    #[inline]
    fn index(&self, x: i32, y: i32) -> usize {
        ((y - self.rect.y) as u32 * self.rect.w + (x - self.rect.x) as u32) as usize
    }

    /// a selection raster holding these values, clipped to `clip`. none when empty.
    pub fn to_raster(&self, clip: IRect) -> Option<Raster> {
        let area = self.rect.intersect(&clip);
        let mut out = Raster::gray(Depth::U8, 0.0);
        if area.is_empty() {
            return None;
        }
        let w = self.rect.w as usize;
        let mut values = Vec::with_capacity(area.area() as usize);
        for y in area.y..area.bottom() {
            let row = (y - self.rect.y) as usize * w + (area.x - self.rect.x) as usize;
            values.extend_from_slice(&self.data[row..row + area.w as usize]);
        }
        out.write(area, &values);
        (!out.is_empty()).then_some(out)
    }
}

/// colour pixels to select from, straight alpha, 16 bits per channel to keep memory sane
#[derive(Debug, Clone)]
pub struct ColorBuf {
    pub rect: IRect,
    pub data: Vec<[u16; 4]>,
}

impl ColorBuf {
    /// pixels from a colour raster
    pub fn from_raster(raster: &Raster, rect: IRect) -> Self {
        let mut data = Vec::with_capacity(rect.area() as usize);
        // a strip at a time, so the float copy stays small
        let mut y = rect.y;
        while y < rect.bottom() {
            let h = (rect.bottom() - y).min(256) as u32;
            let strip = raster.read(IRect::new(rect.x, y, rect.w, h));
            data.extend(
                strip
                    .chunks_exact(4)
                    .map(|p| [q16(p[0]), q16(p[1]), q16(p[2]), q16(p[3])]),
            );
            y += h as i32;
        }
        Self { rect, data }
    }

    /// pixels from premultiplied colour, as the render engine gives them
    pub fn from_premultiplied(rect: IRect, pixels: &[[f32; 4]]) -> Self {
        Self {
            rect,
            data: pixels
                .iter()
                .map(|p| {
                    let a = p[3];
                    let u = |v: f32| if a > 0.0 { v / a } else { 0.0 };
                    [q16(u(p[0])), q16(u(p[1])), q16(u(p[2])), q16(a)]
                })
                .collect(),
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> [f32; 4] {
        if !self.rect.contains(x, y) {
            return [0.0; 4];
        }
        let p =
            self.data[((y - self.rect.y) as u32 * self.rect.w + (x - self.rect.x) as u32) as usize];
        p.map(|v| f32::from(v) / 65535.0)
    }
}

fn q16(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
}

/// a shape a marquee or lasso draws, in document pixels (edges, not centres)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum Shape {
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    Ellipse {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    /// lasso, polygon lasso and magnetic lasso outlines, closed automatically
    Polygon {
        points: Vec<[f64; 2]>,
    },
}

impl Shape {
    /// the pixels the shape can touch
    pub fn bounds(&self) -> IRect {
        let (x0, y0, x1, y1) = match self {
            Shape::Rect { x, y, w, h } | Shape::Ellipse { x, y, w, h } => {
                (x.min(x + w), y.min(y + h), x.max(x + w), y.max(y + h))
            }
            Shape::Polygon { points } => points.iter().fold(
                (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
                |(a, b, c, d), p| (a.min(p[0]), b.min(p[1]), c.max(p[0]), d.max(p[1])),
            ),
        };
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return IRect::EMPTY;
        }
        let lim = f64::from(i32::MAX / 4);
        IRect::from_corners(
            x0.floor().clamp(-lim, lim) as i32,
            y0.floor().clamp(-lim, lim) as i32,
            x1.ceil().clamp(-lim, lim) as i32,
            y1.ceil().clamp(-lim, lim) as i32,
        )
    }

    /// how much of each pixel the shape covers. without anti aliasing a pixel is in when
    /// its centre is.
    pub fn coverage(&self, anti_alias: bool) -> Box<dyn Fn(i32, i32) -> f32 + Sync + '_> {
        match self {
            Shape::Rect { x, y, w, h } => {
                let (x0, x1) = (x.min(x + w), x.max(x + w));
                let (y0, y1) = (y.min(y + h), y.max(y + h));
                Box::new(move |px, py| {
                    let (fx, fy) = (f64::from(px), f64::from(py));
                    if anti_alias {
                        let ox = ((fx + 1.0).min(x1) - fx.max(x0)).clamp(0.0, 1.0);
                        let oy = ((fy + 1.0).min(y1) - fy.max(y0)).clamp(0.0, 1.0);
                        (ox * oy) as f32
                    } else {
                        let (cx, cy) = (fx + 0.5, fy + 0.5);
                        f32::from(u8::from(cx >= x0 && cx < x1 && cy >= y0 && cy < y1))
                    }
                })
            }
            Shape::Ellipse { x, y, w, h } => {
                let (rx, ry) = (w.abs() / 2.0, h.abs() / 2.0);
                let (cx, cy) = (x.min(x + w) + rx, y.min(y + h) + ry);
                Box::new(move |px, py| {
                    if rx <= 0.0 || ry <= 0.0 {
                        return 0.0;
                    }
                    let dx = f64::from(px) + 0.5 - cx;
                    let dy = f64::from(py) + 0.5 - cy;
                    let f = (dx / rx).powi(2) + (dy / ry).powi(2) - 1.0;
                    if !anti_alias {
                        return f32::from(u8::from(f < 0.0));
                    }
                    // distance to the edge from the implicit function and its gradient
                    let g = 2.0 * ((dx / (rx * rx)).powi(2) + (dy / (ry * ry)).powi(2)).sqrt();
                    let d = if g > 1e-12 { f / g } else { -f64::MAX };
                    (0.5 - d).clamp(0.0, 1.0) as f32
                })
            }
            Shape::Polygon { points } => {
                let spans = PolygonSpans::new(points, self.bounds(), anti_alias);
                Box::new(move |px, py| spans.coverage(px, py))
            }
        }
    }
}

/// a polygon turned into covered spans per pixel row: four sub rows per pixel with exact
/// horizontal coverage when anti aliased, the pixel centre row otherwise. filled with the
/// non zero rule, so a lasso that crosses itself stays filled.
struct PolygonSpans {
    top: i32,
    /// per pixel row, (start x, end x, weight) spans
    rows: Vec<Vec<(f64, f64, f32)>>,
    anti_alias: bool,
}

impl PolygonSpans {
    fn new(points: &[[f64; 2]], bounds: IRect, anti_alias: bool) -> Self {
        let subs: &[f64] = if anti_alias {
            &[0.125, 0.375, 0.625, 0.875]
        } else {
            &[0.5]
        };
        let weight = 1.0 / subs.len() as f32;
        let n = points.len();
        let rows = (bounds.y..bounds.bottom())
            .into_par_iter()
            .map(|row| {
                let mut spans = Vec::new();
                if n < 3 {
                    return spans;
                }
                let mut crossings: Vec<(f64, i32)> = Vec::new();
                for sub in subs {
                    let sy = f64::from(row) + sub;
                    crossings.clear();
                    for i in 0..n {
                        let (a, b) = (points[i], points[(i + 1) % n]);
                        if (a[1] <= sy && b[1] > sy) || (b[1] <= sy && a[1] > sy) {
                            let t = (sy - a[1]) / (b[1] - a[1]);
                            let dir = if b[1] > a[1] { 1 } else { -1 };
                            crossings.push((a[0] + t * (b[0] - a[0]), dir));
                        }
                    }
                    crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
                    let mut winding = 0;
                    for pair in crossings.windows(2) {
                        winding += pair[0].1;
                        if winding != 0 && pair[1].0 > pair[0].0 {
                            spans.push((pair[0].0, pair[1].0, weight));
                        }
                    }
                }
                spans
            })
            .collect();
        Self {
            top: bounds.y,
            rows,
            anti_alias,
        }
    }

    fn coverage(&self, x: i32, y: i32) -> f32 {
        let Some(row) = usize::try_from(y - self.top)
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return 0.0;
        };
        let (x0, x1) = (f64::from(x), f64::from(x) + 1.0);
        let mut c = 0.0;
        for &(a, b, w) in row {
            if self.anti_alias {
                let o = (x1.min(b) - x0.max(a)).max(0.0);
                c += o as f32 * w;
            } else if x0 + 0.5 >= a && x0 + 0.5 < b {
                c += w;
            }
        }
        c.min(1.0)
    }
}

/// meets the current selection with a new coverage over `area`. the result is clipped to
/// the canvas; none means nothing is selected.
pub fn combine(
    current: Option<&Raster>,
    canvas: IRect,
    area: IRect,
    mode: Combine,
    coverage: &(dyn Fn(i32, i32) -> f32 + Sync),
) -> Option<Raster> {
    let area = area.intersect(&canvas);
    let blank = || Raster::gray(Depth::U8, 0.0);
    let out = match (mode, current) {
        (Combine::Replace, _) | (Combine::Add, None) => {
            let mut r = blank();
            r.update(area, |x, y, v| v[0] = coverage(x, y));
            r
        }
        (Combine::Add, Some(cur)) => {
            let mut r = cur.clone();
            r.update(area, |x, y, v| {
                let c = coverage(x, y);
                v[0] = v[0] + c - v[0] * c;
            });
            r
        }
        (Combine::Subtract, None) | (Combine::Intersect, None) => return None,
        (Combine::Subtract, Some(cur)) => {
            let mut r = cur.clone();
            r.update(area.intersect(&cur.tile_extent()), |x, y, v| {
                v[0] *= 1.0 - coverage(x, y);
            });
            r
        }
        (Combine::Intersect, Some(cur)) => {
            let mut r = blank();
            r.update(area.intersect(&cur.content_bounds()), |x, y, v| {
                v[0] = cur.get(x, y, 0) * coverage(x, y);
            });
            r
        }
    };
    (!out.is_empty()).then_some(out)
}

/// meets two selection masks, for loading a saved selection
pub fn combine_masks(
    current: Option<&Raster>,
    other: &Raster,
    canvas: IRect,
    mode: Combine,
) -> Option<Raster> {
    let area = match mode {
        Combine::Replace | Combine::Add => other.content_bounds(),
        Combine::Subtract | Combine::Intersect => {
            current.map_or(IRect::EMPTY, Raster::content_bounds)
        }
    };
    combine(current, canvas, area, mode, &|x, y| other.get(x, y, 0))
}

/// every pixel of the canvas
pub fn select_all(canvas: IRect) -> Raster {
    let mut r = Raster::gray(Depth::U8, 0.0);
    r.update(canvas, |_, _, v| v[0] = 1.0);
    r
}

/// everything that was not selected, inside the canvas
pub fn invert(current: Option<&Raster>, canvas: IRect) -> Option<Raster> {
    let mut r = Raster::gray(Depth::U8, 0.0);
    r.update(canvas, |x, y, v| {
        v[0] = 1.0 - current.map_or(0.0, |c| c.get(x, y, 0));
    });
    (!r.is_empty()).then_some(r)
}

fn too_big(rect: IRect) -> Result<(), String> {
    if rect.area() > MAX_DENSE {
        return Err(format!(
            "The selection is too big to change this way ({}×{} pixels). Try it on a smaller area.",
            rect.w, rect.h
        ));
    }
    Ok(())
}

/// softens the edge: a gaussian blur with a standard deviation of half the radius
pub fn feather(sel: &Raster, canvas: IRect, radius: f32) -> Result<Option<Raster>, String> {
    if radius <= 0.0 {
        return Ok(Some(sel.clone()));
    }
    let sigma = radius / 2.0;
    let pad = (sigma * 3.0).ceil() as i32 + 1;
    let rect = sel
        .content_bounds()
        .expand(pad)
        .intersect(&canvas.expand(pad));
    too_big(rect)?;
    let mut buf = GrayBuf::from_raster(sel, rect);
    gaussian(&mut buf, sigma);
    Ok(buf.to_raster(canvas))
}

/// a separable gaussian blur in place; pixels beyond the buffer count as zero
pub fn gaussian(buf: &mut GrayBuf, sigma: f32) {
    if sigma <= 0.0 || buf.rect.is_empty() {
        return;
    }
    let r = (sigma * 3.0).ceil() as i32;
    let mut k: Vec<f32> = (-r..=r)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= sum);
    let (w, h) = (buf.rect.w as usize, buf.rect.h as usize);
    let mut tmp = vec![0f32; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let src = &buf.data[y * w..(y + 1) * w];
        for (x, out) in row.iter_mut().enumerate() {
            let mut acc = 0.0;
            for (j, kv) in k.iter().enumerate() {
                let sx = x as i32 + j as i32 - r;
                if sx >= 0 && (sx as usize) < w {
                    acc += src[sx as usize] * kv;
                }
            }
            *out = acc;
        }
    });
    let data = &mut buf.data;
    data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let mut acc = 0.0;
            for (j, kv) in k.iter().enumerate() {
                let sy = y as i32 + j as i32 - r;
                if sy >= 0 && (sy as usize) < h {
                    acc += tmp[sy as usize * w + x] * kv;
                }
            }
            *out = acc;
        }
    });
}

/// squared euclidean distance from every cell to the nearest `true` cell (felzenszwalb
/// and huttenlocher's linear time transform)
fn distance_squared(mask: &[bool], w: usize, h: usize) -> Vec<f32> {
    const INF: f32 = 1e20;
    let mut d: Vec<f32> = mask.iter().map(|&m| if m { 0.0 } else { INF }).collect();
    fn pass(f: &[f32], out: &mut [f32]) {
        let n = f.len();
        let mut v = vec![0usize; n];
        let mut z = vec![0f32; n + 1];
        let mut k = 0usize;
        z[0] = -INF;
        z[1] = INF;
        for q in 1..n {
            loop {
                let p = v[k];
                let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32))
                    / (2.0 * (q as f32 - p as f32));
                if s <= z[k] && k > 0 {
                    k -= 1;
                    continue;
                }
                if s <= z[k] {
                    // k is 0 and the new parabola wins everywhere so far
                    v[0] = q;
                    z[0] = -INF;
                    z[1] = INF;
                    break;
                }
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = INF;
                break;
            }
        }
        k = 0;
        for (q, o) in out.iter_mut().enumerate() {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let p = v[k];
            *o = (q as f32 - p as f32).powi(2) + f[p];
        }
    }
    // columns
    let mut col = vec![0f32; h];
    let mut res = vec![0f32; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = d[y * w + x];
        }
        pass(&col, &mut res);
        for y in 0..h {
            d[y * w + x] = res[y];
        }
    }
    d.par_chunks_mut(w).for_each(|row| {
        let f = row.to_vec();
        pass(&f, row);
    });
    d
}

/// signed distance in pixels from each pixel centre to the selection edge, negative inside,
/// with partly selected pixels placing the edge inside them
fn signed_distance(buf: &GrayBuf) -> Vec<f32> {
    let (w, h) = (buf.rect.w as usize, buf.rect.h as usize);
    let inside: Vec<bool> = buf.data.iter().map(|&v| v >= 0.5).collect();
    let outside: Vec<bool> = inside.iter().map(|&b| !b).collect();
    let to_in = distance_squared(&inside, w, h);
    let to_out = distance_squared(&outside, w, h);
    buf.data
        .par_iter()
        .enumerate()
        .map(|(i, &v)| {
            if v > 0.0 && v < 1.0 {
                0.5 - v
            } else if inside[i] {
                -(to_out[i].sqrt() - 0.5)
            } else {
                to_in[i].sqrt() - 0.5
            }
        })
        .collect()
}

/// grows (positive) or shrinks (negative) a selection by `by` pixels, in circles
pub fn expand(sel: &Raster, canvas: IRect, by: f32) -> Result<Option<Raster>, String> {
    if by == 0.0 {
        return Ok(Some(sel.clone()));
    }
    let pad = by.abs().ceil() as i32 + 2;
    let rect = sel
        .content_bounds()
        .expand(pad)
        .intersect(&canvas.expand(pad));
    too_big(rect)?;
    let buf = GrayBuf::from_raster(sel, rect);
    let sd = signed_distance(&buf);
    let out = GrayBuf {
        rect,
        data: sd
            .iter()
            .map(|d| (0.5 - (d - by)).clamp(0.0, 1.0))
            .collect(),
    };
    Ok(out.to_raster(canvas))
}

/// a band `width` pixels wide centred on the selection's edge
pub fn border(sel: &Raster, canvas: IRect, width: f32) -> Result<Option<Raster>, String> {
    let half = width.max(1.0) / 2.0;
    let pad = half.ceil() as i32 + 2;
    let rect = sel
        .content_bounds()
        .expand(pad)
        .intersect(&canvas.expand(pad));
    too_big(rect)?;
    let buf = GrayBuf::from_raster(sel, rect);
    let sd = signed_distance(&buf);
    let out = GrayBuf {
        rect,
        data: sd
            .iter()
            .map(|d| (0.5 - (d.abs() - half)).clamp(0.0, 1.0))
            .collect(),
    };
    Ok(out.to_raster(canvas))
}

/// rounds off corners and removes specks smaller than about `radius`
pub fn smooth(sel: &Raster, canvas: IRect, radius: f32) -> Result<Option<Raster>, String> {
    if radius <= 0.0 {
        return Ok(Some(sel.clone()));
    }
    let pad = radius.ceil() as i32 * 2 + 2;
    let rect = sel
        .content_bounds()
        .expand(pad)
        .intersect(&canvas.expand(pad));
    too_big(rect)?;
    let mut buf = GrayBuf::from_raster(sel, rect);
    let sigma = radius / 2.0;
    gaussian(&mut buf, sigma);
    // then pull the soft edge back to a crisp one. a blurred step rises with a slope of
    // 1 / (sigma * sqrt(2 pi)) per pixel at its middle, so scaling by the inverse leaves a
    // ramp about one pixel wide: anti aliased, not blurry.
    let k = sigma * (2.0 * std::f32::consts::PI).sqrt();
    buf.data
        .par_iter_mut()
        .for_each(|v| *v = ((*v - 0.5) * k.max(1.0) + 0.5).clamp(0.0, 1.0));
    Ok(buf.to_raster(canvas))
}

/// how different two colours are: the biggest channel difference, on premultiplied colour
/// so every see through pixel counts as the same
fn difference(a: [f32; 4], b: [f32; 4]) -> f32 {
    let pa = [a[0] * a[3], a[1] * a[3], a[2] * a[3], a[3]];
    let pb = [b[0] * b[3], b[1] * b[3], b[2] * b[3], b[3]];
    (0..4).map(|k| (pa[k] - pb[k]).abs()).fold(0.0, f32::max)
}

/// softens a hard mask's edge by a pixel, like photoshop's anti alias box
fn soften_edges(buf: &mut GrayBuf) {
    let src = buf.data.clone();
    let (w, h) = (buf.rect.w as i32, buf.rect.h as i32);
    buf.data
        .par_chunks_mut(w as usize)
        .enumerate()
        .for_each(|(y, row)| {
            let y = y as i32;
            for (x, out) in row.iter_mut().enumerate() {
                let x = x as i32;
                let at = |dx: i32, dy: i32| {
                    let (sx, sy) = (x + dx, y + dy);
                    if sx < 0 || sy < 0 || sx >= w || sy >= h {
                        0.0
                    } else {
                        src[(sy * w + sx) as usize]
                    }
                };
                let c = at(0, 0);
                let around = [at(-1, 0), at(1, 0), at(0, -1), at(0, 1)];
                if around.iter().all(|v| *v == c) {
                    continue;
                }
                // a 3x3 tent filter, only where the mask changes
                let mut acc = c * 4.0;
                for v in around {
                    acc += v * 2.0;
                }
                for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                    acc += at(dx, dy);
                }
                *out = acc / 16.0;
            }
        });
}

/// the magic wand: pixels whose colour is within `tolerance` (0 to 255, like photoshop) of
/// the clicked pixel, touching it when `contiguous`
pub fn magic_wand(
    img: &ColorBuf,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
) -> GrayBuf {
    let mut out = GrayBuf::new(img.rect);
    if !img.rect.contains(seed.0, seed.1) {
        return out;
    }
    let target = img.get(seed.0, seed.1);
    let limit = (tolerance.max(0.0) + 0.5) / 255.0;
    let near = |x: i32, y: i32| difference(img.get(x, y), target) <= limit;
    if contiguous {
        let r = img.rect;
        let mut seen = vec![false; r.area() as usize];
        let mut stack = vec![seed];
        while let Some((x, y)) = stack.pop() {
            let i = out.index(x, y);
            if seen[i] {
                continue;
            }
            // walk the row both ways, then queue the rows above and below
            let mut left = x;
            while left > r.x && !seen[out.index(left - 1, y)] && near(left - 1, y) {
                left -= 1;
            }
            let mut right = x;
            while right + 1 < r.right() && !seen[out.index(right + 1, y)] && near(right + 1, y) {
                right += 1;
            }
            if !near(x, y) {
                seen[i] = true;
                continue;
            }
            for sx in left..=right {
                let j = out.index(sx, y);
                seen[j] = true;
                out.data[j] = 1.0;
                for ny in [y - 1, y + 1] {
                    if ny >= r.y && ny < r.bottom() && !seen[out.index(sx, ny)] && near(sx, ny) {
                        stack.push((sx, ny));
                    }
                }
            }
        }
    } else {
        let r = img.rect;
        out.data
            .par_chunks_mut(r.w as usize)
            .enumerate()
            .for_each(|(row, line)| {
                let y = r.y + row as i32;
                for (col, v) in line.iter_mut().enumerate() {
                    if near(r.x + col as i32, y) {
                        *v = 1.0;
                    }
                }
            });
    }
    if anti_alias {
        soften_edges(&mut out);
    }
    out
}

/// colour range: every pixel close to any of the sample colours (straight rgb, 0 to 1).
/// fully selected within half of `fuzziness` (0 to 200, like photoshop), fading out to
/// nothing at `fuzziness`.
pub fn color_range(img: &ColorBuf, samples: &[[f32; 3]], fuzziness: f32) -> GrayBuf {
    let mut out = GrayBuf::new(img.rect);
    let r = img.rect;
    let fuzz = fuzziness.max(0.0) / 255.0;
    out.data
        .par_chunks_mut(r.w as usize)
        .enumerate()
        .for_each(|(row, line)| {
            let y = r.y + row as i32;
            for (col, v) in line.iter_mut().enumerate() {
                let c = img.get(r.x + col as i32, y);
                let mut best = 0f32;
                for s in samples {
                    let d = (0..3).map(|k| (c[k] - s[k]).abs()).fold(0.0, f32::max);
                    let cov = if fuzz <= 0.0 {
                        f32::from(u8::from(d <= 0.5 / 255.0))
                    } else {
                        ((fuzz - d) / (fuzz * 0.5)).clamp(0.0, 1.0)
                    };
                    best = best.max(cov);
                }
                *v = best * c[3];
            }
        });
    out
}

/// the quick selection brush: grows a region out from the pixels under a brush stroke into
/// neighbours of similar colour, stopping at strong edges and a few brush widths away
pub fn quick_select(img: &ColorBuf, path: &[[f32; 2]], radius: f32) -> GrayBuf {
    let r = img.rect;
    let mut out = GrayBuf::new(r);
    if path.is_empty() || r.is_empty() {
        return out;
    }
    let radius = radius.max(1.0);
    // pixels under the brush seed the region and say what colour it is
    let mut seeds = Vec::new();
    let mut sum = [0f64; 3];
    let mut sum2 = [0f64; 3];
    let mut on_path = vec![false; r.area() as usize];
    let mut prev = path[0];
    for p in path {
        let steps = ((p[0] - prev[0]).hypot(p[1] - prev[1]) / (radius * 0.5))
            .ceil()
            .max(1.0) as i32;
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let (cx, cy) = (
                prev[0] + (p[0] - prev[0]) * t,
                prev[1] + (p[1] - prev[1]) * t,
            );
            let ri = radius.ceil() as i32;
            for y in (cy as i32 - ri)..=(cy as i32 + ri) {
                for x in (cx as i32 - ri)..=(cx as i32 + ri) {
                    if !r.contains(x, y)
                        || (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) > radius
                    {
                        continue;
                    }
                    let i = out.index(x, y);
                    if on_path[i] {
                        continue;
                    }
                    on_path[i] = true;
                    let c = img.get(x, y);
                    for k in 0..3 {
                        sum[k] += f64::from(c[k]);
                        sum2[k] += f64::from(c[k]).powi(2);
                    }
                    seeds.push((x, y));
                }
            }
        }
        prev = *p;
    }
    if seeds.is_empty() {
        return out;
    }
    let n = seeds.len() as f64;
    let mean: [f32; 3] = std::array::from_fn(|k| (sum[k] / n) as f32);
    let spread = (0..3)
        .map(|k| ((sum2[k] / n - (sum[k] / n).powi(2)).max(0.0)).sqrt() as f32)
        .fold(0.0, f32::max);
    let global = (spread * 2.5).max(24.0 / 255.0);
    let local = (spread * 1.2).max(10.0 / 255.0);
    // how far the region may spread from the stroke
    let (bx0, by0, bx1, by1) = path
        .iter()
        .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |a, p| {
            (a.0.min(p[0]), a.1.min(p[1]), a.2.max(p[0]), a.3.max(p[1]))
        });
    let reach = radius * 4.0;
    let limit = IRect::from_corners(
        (bx0 - reach).floor() as i32,
        (by0 - reach).floor() as i32,
        (bx1 + reach).ceil() as i32,
        (by1 + reach).ceil() as i32,
    )
    .intersect(&r);
    let mut stack = seeds;
    for &(x, y) in &stack {
        let i = out.index(x, y);
        out.data[i] = 1.0;
    }
    while let Some((x, y)) = stack.pop() {
        let c = img.get(x, y);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            if !limit.contains(nx, ny) {
                continue;
            }
            let j = out.index(nx, ny);
            if out.data[j] > 0.0 {
                continue;
            }
            let q = img.get(nx, ny);
            let to_mean = (0..3).map(|k| (q[k] - mean[k]).abs()).fold(0.0, f32::max);
            let step = difference(q, c);
            if to_mean <= global && step <= local {
                out.data[j] = 1.0;
                stack.push((nx, ny));
            }
        }
    }
    soften_edges(&mut out);
    out
}

/// edge strength for the magnetic lasso, 0 to 1
#[derive(Debug, Clone)]
pub struct EdgeMap {
    pub rect: IRect,
    data: Vec<f32>,
}

impl EdgeMap {
    /// sobel edges on brightness, scaled so `contrast` (0 to 1) of the strongest edge in
    /// the picture counts as a full edge
    pub fn new(img: &ColorBuf, contrast: f32) -> Self {
        let r = img.rect;
        let lum = |x: i32, y: i32| {
            let c = img.get(x.clamp(r.x, r.right() - 1), y.clamp(r.y, r.bottom() - 1));
            (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) * c[3]
        };
        let mut data = vec![0f32; r.area() as usize];
        data.par_chunks_mut(r.w as usize)
            .enumerate()
            .for_each(|(row, line)| {
                let y = r.y + row as i32;
                for (col, v) in line.iter_mut().enumerate() {
                    let x = r.x + col as i32;
                    let gx = lum(x + 1, y - 1) + 2.0 * lum(x + 1, y) + lum(x + 1, y + 1)
                        - lum(x - 1, y - 1)
                        - 2.0 * lum(x - 1, y)
                        - lum(x - 1, y + 1);
                    let gy = lum(x - 1, y + 1) + 2.0 * lum(x, y + 1) + lum(x + 1, y + 1)
                        - lum(x - 1, y - 1)
                        - 2.0 * lum(x, y - 1)
                        - lum(x + 1, y - 1);
                    *v = gx.hypot(gy);
                }
            });
        let max = data.iter().copied().fold(0.0, f32::max).max(1e-6);
        let scale = 1.0 / (max * contrast.clamp(0.05, 1.0));
        data.par_iter_mut().for_each(|v| *v = (*v * scale).min(1.0));
        Self { rect: r, data }
    }

    fn get(&self, x: i32, y: i32) -> f32 {
        if !self.rect.contains(x, y) {
            return 0.0;
        }
        self.data[((y - self.rect.y) as u32 * self.rect.w + (x - self.rect.x) as u32) as usize]
    }
}

#[derive(PartialEq)]
struct Visit(f32, i32, i32);

impl Eq for Visit {}

impl Ord for Visit {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0)
    }
}

impl PartialOrd for Visit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// the magnetic lasso's path from an anchor towards the pointer: the cheapest route along
/// strong edges to the strongest edge within `width` pixels of the pointer. pixel centres,
/// starting at `from`.
pub fn live_wire(edges: &EdgeMap, from: (i32, i32), to: (i32, i32), width: i32) -> Vec<(i32, i32)> {
    let width = width.clamp(1, 64);
    // snap the end to the strongest edge near the pointer
    let mut target = to;
    let mut best = -1.0;
    for y in to.1 - width..=to.1 + width {
        for x in to.0 - width..=to.0 + width {
            let d = ((x - to.0) as f32).hypot((y - to.1) as f32);
            if d > width as f32 || !edges.rect.contains(x, y) {
                continue;
            }
            let score = edges.get(x, y) - d * 0.01;
            if score > best {
                best = score;
                target = (x, y);
            }
        }
    }
    let window = IRect::from_corners(
        from.0.min(target.0) - width * 2,
        from.1.min(target.1) - width * 2,
        from.0.max(target.0) + width * 2 + 1,
        from.1.max(target.1) + width * 2 + 1,
    )
    .intersect(&edges.rect);
    if !window.contains(from.0, from.1) || !window.contains(target.0, target.1) {
        return vec![from, to];
    }
    let idx = |x: i32, y: i32| ((y - window.y) as u32 * window.w + (x - window.x) as u32) as usize;
    let mut cost = vec![f32::INFINITY; window.area() as usize];
    let mut came = vec![u32::MAX; window.area() as usize];
    let mut heap = BinaryHeap::new();
    cost[idx(from.0, from.1)] = 0.0;
    heap.push(Visit(0.0, from.0, from.1));
    while let Some(Visit(c, x, y)) = heap.pop() {
        if (x, y) == target {
            break;
        }
        if c > cost[idx(x, y)] {
            continue;
        }
        for (dx, dy) in [
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (1, -1),
            (-1, 1),
            (1, 1),
        ] {
            let (nx, ny) = (x + dx, y + dy);
            if !window.contains(nx, ny) {
                continue;
            }
            let step = if dx != 0 && dy != 0 {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
            let weak = 1.0 - edges.get(nx, ny);
            let nc = c + step * (0.15 + weak * weak * 4.0);
            let j = idx(nx, ny);
            if nc < cost[j] {
                cost[j] = nc;
                came[j] = idx(x, y) as u32;
                heap.push(Visit(nc, nx, ny));
            }
        }
    }
    let mut path = Vec::new();
    let mut at = idx(target.0, target.1);
    let start = idx(from.0, from.1);
    if came[at] == u32::MAX && at != start {
        return vec![from, target];
    }
    while at != start {
        let (x, y) = (at as u32 % window.w, at as u32 / window.w);
        path.push((window.x + x as i32, window.y + y as i32));
        at = came[at] as usize;
    }
    path.push(from);
    path.reverse();
    path
}

/// the outline of a mask at the 50 percent line, along pixel edges, as closed loops in
/// the buffer's own pixel units. corners only: straight runs are one segment. this is
/// what marching ants draw, so they sit exactly on pixel edges at any zoom.
pub fn outline(buf: &GrayBuf) -> Vec<Vec<[i32; 2]>> {
    let r = buf.rect;
    let inside = |x: i32, y: i32| buf.get(x, y) >= 0.5;
    // directed edges with the selected side on the right (clockwise on screen, y down)
    let mut next: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            if !inside(x, y) {
                continue;
            }
            if !inside(x, y - 1) {
                next.entry((x, y)).or_default().push((x + 1, y));
            }
            if !inside(x + 1, y) {
                next.entry((x + 1, y)).or_default().push((x + 1, y + 1));
            }
            if !inside(x, y + 1) {
                next.entry((x + 1, y + 1)).or_default().push((x, y + 1));
            }
            if !inside(x - 1, y) {
                next.entry((x, y + 1)).or_default().push((x, y));
            }
        }
    }
    let mut starts: Vec<(i32, i32)> = next.keys().copied().collect();
    starts.sort_by_key(|&(x, y)| (y, x));
    let mut loops = Vec::new();
    for s in starts {
        while let Some(first) = next.get_mut(&s).and_then(Vec::pop) {
            let mut pts = vec![s];
            let mut prev = s;
            let mut at = first;
            while at != s {
                let dir = (at.0 - prev.0, at.1 - prev.1);
                let Some(outs) = next.get_mut(&at) else {
                    break;
                };
                if outs.is_empty() {
                    break;
                }
                // where two corners touch, turn right so loops stay apart
                let pick = if outs.len() > 1 {
                    let right = (-dir.1, dir.0);
                    outs.iter()
                        .position(|o| (o.0 - at.0, o.1 - at.1) == right)
                        .unwrap_or(0)
                } else {
                    0
                };
                let n = outs.swap_remove(pick);
                pts.push(at);
                prev = at;
                at = n;
            }
            loops.push(corners(&pts));
        }
    }
    loops.retain(|l| l.len() >= 3);
    loops
}

/// drops points in the middle of straight runs
fn corners(pts: &[(i32, i32)]) -> Vec<[i32; 2]> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n / 4);
    for i in 0..n {
        let a = pts[(i + n - 1) % n];
        let b = pts[i];
        let c = pts[(i + 1) % n];
        if (b.0 - a.0, b.1 - a.1) != (c.0 - b.0, c.1 - b.1) {
            out.push([b.0, b.1]);
        }
    }
    out
}

/// the selection's outline at a mip level (level 1 is half size), scaled back to document
/// pixels, for the window to draw as marching ants
pub fn outline_at_level(sel: &Raster, level: u32) -> Vec<Vec<[f32; 2]>> {
    let bounds = sel.content_bounds();
    if bounds.is_empty() {
        return Vec::new();
    }
    let rect = bounds.at_level(level);
    let buf = if level == 0 {
        GrayBuf::from_raster(sel, rect)
    } else {
        match sel.read_level(rect, level) {
            Some(px) => GrayBuf {
                rect: px.rect,
                data: (0..px.len()).map(|i| px.get(i)[0]).collect(),
            },
            None => return Vec::new(),
        }
    };
    let scale = (1u64 << level.min(30)) as f32;
    outline(&buf)
        .into_iter()
        .map(|l| {
            l.into_iter()
                .map(|p| [p[0] as f32 * scale, p[1] as f32 * scale])
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(r: &Raster, canvas: IRect) -> f64 {
        GrayBuf::from_raster(r, canvas)
            .data
            .iter()
            .map(|v| f64::from(*v))
            .sum()
    }

    const CANVAS: IRect = IRect::new(0, 0, 200, 200);

    #[test]
    fn rect_coverage_is_exact_area() {
        let s = Shape::Rect {
            x: 10.25,
            y: 20.5,
            w: 30.5,
            h: 10.0,
        };
        let r = combine(
            None,
            CANVAS,
            s.bounds(),
            Combine::Replace,
            &*s.coverage(true),
        )
        .unwrap();
        // eight bit storage rounds partial pixels a little
        assert!((area(&r, CANVAS) - 305.0).abs() < 0.5);
        let hard = combine(
            None,
            CANVAS,
            s.bounds(),
            Combine::Replace,
            &*s.coverage(false),
        )
        .unwrap();
        // pixel centres from 10.5 to 40.5 and 20.5 to 29.5 are inside
        assert_eq!(area(&hard, CANVAS).round(), 310.0);
    }

    #[test]
    fn ellipse_and_polygon_areas_match_geometry() {
        let e = Shape::Ellipse {
            x: 50.0,
            y: 50.0,
            w: 80.0,
            h: 40.0,
        };
        let r = combine(
            None,
            CANVAS,
            e.bounds(),
            Combine::Replace,
            &*e.coverage(true),
        )
        .unwrap();
        let want = std::f64::consts::PI * 40.0 * 20.0;
        assert!((area(&r, CANVAS) - want).abs() / want < 0.002);
        let tri = Shape::Polygon {
            points: vec![[10.0, 10.0], [110.0, 10.0], [10.0, 110.0]],
        };
        let r = combine(
            None,
            CANVAS,
            tri.bounds(),
            Combine::Replace,
            &*tri.coverage(true),
        )
        .unwrap();
        assert!((area(&r, CANVAS) - 5000.0).abs() < 2.0);
    }

    #[test]
    fn modes_combine_like_set_operations() {
        let a = Shape::Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let b = Shape::Rect {
            x: 50.0,
            y: 50.0,
            w: 100.0,
            h: 100.0,
        };
        let base = combine(
            None,
            CANVAS,
            a.bounds(),
            Combine::Replace,
            &*a.coverage(true),
        );
        let cov = b.coverage(true);
        let add = combine(base.as_ref(), CANVAS, b.bounds(), Combine::Add, &*cov).unwrap();
        let sub = combine(base.as_ref(), CANVAS, b.bounds(), Combine::Subtract, &*cov).unwrap();
        let int = combine(base.as_ref(), CANVAS, b.bounds(), Combine::Intersect, &*cov).unwrap();
        assert_eq!(area(&add, CANVAS), 17500.0);
        assert_eq!(area(&sub, CANVAS), 7500.0);
        assert_eq!(area(&int, CANVAS), 2500.0);
        let inv = invert(Some(&int), CANVAS).unwrap();
        assert_eq!(area(&inv, CANVAS), 40000.0 - 2500.0);
    }

    #[test]
    fn expand_and_contract_grow_in_circles() {
        let a = Shape::Rect {
            x: 50.0,
            y: 50.0,
            w: 100.0,
            h: 100.0,
        };
        let sel = combine(
            None,
            CANVAS,
            a.bounds(),
            Combine::Replace,
            &*a.coverage(true),
        )
        .unwrap();
        let grown = expand(&sel, CANVAS, 10.0).unwrap().unwrap();
        // a square grown by r gains four sides and four quarter circles
        let want = 10000.0 + 4.0 * 100.0 * 10.0 + std::f64::consts::PI * 100.0;
        assert!(
            (area(&grown, CANVAS) - want).abs() < 12.0,
            "{}",
            area(&grown, CANVAS)
        );
        let shrunk = expand(&sel, CANVAS, -10.0).unwrap().unwrap();
        assert!(
            (area(&shrunk, CANVAS) - 6400.0).abs() < 2.0,
            "{}",
            area(&shrunk, CANVAS)
        );
        let band = border(&sel, CANVAS, 10.0).unwrap().unwrap();
        assert!(band.get(50, 100, 0) > 0.9 && band.get(100, 100, 0) == 0.0);
    }

    #[test]
    fn feather_keeps_the_area_and_softens() {
        let a = Shape::Rect {
            x: 50.0,
            y: 50.0,
            w: 100.0,
            h: 100.0,
        };
        let sel = combine(
            None,
            CANVAS,
            a.bounds(),
            Combine::Replace,
            &*a.coverage(true),
        )
        .unwrap();
        let soft = feather(&sel, CANVAS, 8.0).unwrap().unwrap();
        assert!((area(&soft, CANVAS) - 10000.0).abs() < 40.0);
        // the first pixel's centre is half a pixel in: the normal cdf at 0.5 / sigma
        let edge = soft.get(50, 100, 0);
        assert!((edge - 0.5497).abs() < 0.01, "{edge}");
    }

    fn picture() -> ColorBuf {
        let mut data = vec![[0u16, 0, 0, 65535]; 40 * 40];
        for y in 0..40 {
            for x in 0..40 {
                if !(15..25).contains(&x) && y < 20 {
                    data[y * 40 + x] = [65535, 0, 0, 65535];
                }
            }
        }
        ColorBuf {
            rect: IRect::of_size(40, 40),
            data,
        }
    }

    #[test]
    fn wand_finds_touching_or_all_similar_pixels() {
        let img = picture();
        let near = magic_wand(&img, (2, 2), 32.0, true, false);
        let all = magic_wand(&img, (2, 2), 32.0, false, false);
        assert_eq!(near.data.iter().sum::<f32>(), 15.0 * 20.0);
        assert_eq!(all.data.iter().sum::<f32>(), 30.0 * 20.0);
        let range = color_range(&img, &[[1.0, 0.0, 0.0]], 40.0);
        assert_eq!(range.data.iter().sum::<f32>(), 30.0 * 20.0);
    }

    #[test]
    fn outlines_follow_pixel_edges() {
        let mut buf = GrayBuf::new(IRect::of_size(10, 10));
        for y in 2..5 {
            for x in 3..7 {
                buf.data[y * 10 + x] = 1.0;
            }
        }
        buf.data[8 * 10 + 8] = 1.0;
        let loops = outline(&buf);
        assert_eq!(loops.len(), 2);
        assert_eq!(loops[0], [[3, 2], [7, 2], [7, 5], [3, 5]]);
        assert_eq!(loops[1].len(), 4);
    }

    #[test]
    fn live_wire_follows_an_edge() {
        let mut data = vec![[0u16, 0, 0, 65535]; 60 * 60];
        for y in 0..60 {
            for x in 30..60 {
                data[y * 60 + x] = [65535, 65535, 65535, 65535];
            }
        }
        let img = ColorBuf {
            rect: IRect::of_size(60, 60),
            data,
        };
        let edges = EdgeMap::new(&img, 0.5);
        let path = live_wire(&edges, (30, 5), (33, 50), 6);
        assert_eq!(path.first(), Some(&(30, 5)));
        // the wire hugs the boundary between the dark and light halves
        assert!(path.iter().all(|p| (29..=30).contains(&p.0)), "{path:?}");
    }
}
