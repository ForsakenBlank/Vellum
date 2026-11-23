//! big images: a tiled in-memory source with a mip pyramid, a procedural source for tests
//! and benchmarks, and rendering outputs larger than any gpu texture one tile at a time.
//!
//! sources are split into 256 pixel tiles that each carry a revision. a source's cache key
//! for a region hashes the revisions of the tiles under it, so writing to one tile only
//! invalidates renders that touch that tile. mip levels are built from the level below on
//! first use and rebuilt when a tile under them changes.

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;

use crate::color::{ColorSpace, ColorTransform, IccProfile, WorkingSpace};
use crate::engine::{Engine, RenderRequest};
use crate::graph::{Graph, ImageSource, RenderError};
use crate::image::{AlphaMode, CpuImage, SourceData, SourceFormat, SourcePixels};
use crate::rect::IRect;

/// tile size of tiled sources, in pixels
pub const SOURCE_TILE: u32 = 256;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// how a source's values relate to colour
#[derive(Clone, Debug)]
pub enum SourceColor {
    Space(ColorSpace),
    Icc(IccProfile),
}

/// working space transforms for an icc source, built once per working space
#[derive(Default)]
struct IccLuts(Mutex<HashMap<WorkingSpace, ColorTransform>>);

impl IccLuts {
    fn get(&self, icc: &IccProfile, working: WorkingSpace) -> ColorTransform {
        let mut luts = self.0.lock();
        if let Some(t) = luts.get(&working) {
            return t.clone();
        }
        let t = icc.to_working(working).unwrap_or_else(|err| {
            tracing::warn!(%err, "falling back to srgb for a profile that could not be used");
            ColorTransform::between(ColorSpace::SRGB, working.space())
        });
        luts.insert(working, t.clone());
        t
    }
}

fn to_working(color: &SourceColor, luts: &IccLuts, working: WorkingSpace) -> ColorTransform {
    match color {
        SourceColor::Space(space) => ColorTransform::between(*space, working.space()),
        SourceColor::Icc(icc) => luts.get(icc, working),
    }
}

/// (level, tile x, tile y) to (hash of the level 0 revisions it was built from, tile)
type MipCache = HashMap<(u32, u32, u32), (u64, Arc<SourceData>)>;

#[derive(Clone)]
struct Tile {
    revision: u64,
    data: Arc<SourceData>,
}

/// pixels stored in tiles of the source format, straight alpha, with mip levels built on
/// demand. writing pixels bumps the revision of every tile touched.
pub struct TiledImage {
    id: u64,
    width: u32,
    height: u32,
    format: SourceFormat,
    color: SourceColor,
    tiles: RwLock<Vec<Tile>>,
    cols: u32,
    rows: u32,
    mips: Mutex<MipCache>,
    luts: IccLuts,
    revision: AtomicU64,
}

fn zeroed(format: SourceFormat, pixels: usize) -> SourceData {
    match format {
        SourceFormat::Rgba8 => SourceData::U8(vec![0; pixels * 4]),
        SourceFormat::Rgba16 => SourceData::U16(vec![0; pixels * 4]),
        SourceFormat::Rgba32F => SourceData::F32(vec![0.0; pixels * 4]),
    }
}

fn write_px(data: &mut SourceData, i: usize, c: [f32; 4]) {
    let j = i * 4;
    match data {
        SourceData::U8(d) => {
            for k in 0..4 {
                d[j + k] = (c[k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        SourceData::U16(d) => {
            for k in 0..4 {
                d[j + k] = (c[k].clamp(0.0, 1.0) * 65535.0).round() as u16;
            }
        }
        SourceData::F32(d) => d[j..j + 4].copy_from_slice(&c),
    }
}

impl TiledImage {
    /// a transparent image
    pub fn new(width: u32, height: u32, format: SourceFormat, color: SourceColor) -> Self {
        let cols = width.div_ceil(SOURCE_TILE);
        let rows = height.div_ceil(SOURCE_TILE);
        let blank = Arc::new(zeroed(format, (SOURCE_TILE * SOURCE_TILE) as usize));
        let tiles = (0..cols * rows)
            .map(|_| Tile {
                revision: 0,
                data: blank.clone(),
            })
            .collect();
        Self {
            id: next_id(),
            width,
            height,
            format,
            color,
            tiles: RwLock::new(tiles),
            cols,
            rows,
            mips: Mutex::new(HashMap::new()),
            luts: IccLuts::default(),
            revision: AtomicU64::new(1),
        }
    }

    /// an image from tightly packed straight alpha pixels covering the whole image
    pub fn from_pixels(px: &SourcePixels, color: SourceColor) -> Self {
        let img = Self::new(px.rect.w, px.rect.h, px.format, color);
        img.write(&SourcePixels {
            rect: IRect::of_size(px.rect.w, px.rect.h),
            ..px.clone()
        });
        img
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// writes pixels (straight alpha, any format) into the image, converting to its format
    pub fn write(&self, px: &SourcePixels) {
        let area = px.rect.intersect(&IRect::of_size(self.width, self.height));
        if area.is_empty() {
            return;
        }
        // pixels already in this image's format are copied a row at a time
        let same = px.format == self.format && px.alpha == AlphaMode::Straight;
        let premultiplied = px.alpha == AlphaMode::Premultiplied;
        let cols = self.cols as i32;
        let s = SOURCE_TILE as i32;
        let mut tiles = self.tiles.write();
        tiles.par_iter_mut().enumerate().for_each(|(i, tile)| {
            let (tx, ty) = (i as i32 % cols, i as i32 / cols);
            let tile_rect = IRect::new(tx * s, ty * s, SOURCE_TILE, SOURCE_TILE).intersect(&area);
            if tile_rect.is_empty() {
                return;
            }
            let data = Arc::make_mut(&mut tile.data);
            for y in tile_rect.y..tile_rect.bottom() {
                let src_row = ((y - px.rect.y) as u32 * px.rect.w
                    + (tile_rect.x - px.rect.x) as u32) as usize;
                let dst_row = ((y - ty * s) * s + (tile_rect.x - tx * s)) as usize;
                if same {
                    let n = tile_rect.w as usize * 4;
                    let (a, b) = (src_row * 4, dst_row * 4);
                    match (&*px.data, &mut *data) {
                        (SourceData::U8(src), SourceData::U8(dst)) => {
                            dst[b..b + n].copy_from_slice(&src[a..a + n])
                        }
                        (SourceData::U16(src), SourceData::U16(dst)) => {
                            dst[b..b + n].copy_from_slice(&src[a..a + n])
                        }
                        (SourceData::F32(src), SourceData::F32(dst)) => {
                            dst[b..b + n].copy_from_slice(&src[a..a + n])
                        }
                        _ => {}
                    }
                    continue;
                }
                for x in 0..tile_rect.w as usize {
                    let mut c = px.get(src_row + x);
                    if premultiplied && c[3] > 0.0 {
                        c = [c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]];
                    }
                    write_px(data, dst_row + x, c);
                }
            }
            tile.revision = self.revision.fetch_add(1, Ordering::Relaxed);
        });
    }

    /// the size of the image at a mip level
    fn level_rect(&self, level: u32) -> IRect {
        IRect::of_size(self.width, self.height).at_level(level)
    }

    /// hashes the revisions of the level 0 tiles under a level rectangle
    fn revisions(&self, rect: IRect, level: u32, h: &mut dyn Hasher) {
        let r0 = rect
            .to_level0(level)
            .intersect(&IRect::of_size(self.width, self.height));
        if r0.is_empty() {
            return;
        }
        let s = SOURCE_TILE as i32;
        let tiles = self.tiles.read();
        for ty in (r0.y / s)..=((r0.bottom() - 1) / s) {
            for tx in (r0.x / s)..=((r0.right() - 1) / s) {
                h.write_u64(tiles[(ty as u32 * self.cols + tx as u32) as usize].revision);
            }
        }
    }

    /// one tile of a mip level, built from the level below if it is missing or stale
    fn mip_tile(&self, level: u32, tx: u32, ty: u32) -> Arc<SourceData> {
        let rect = IRect::new(
            (tx * SOURCE_TILE) as i32,
            (ty * SOURCE_TILE) as i32,
            SOURCE_TILE,
            SOURCE_TILE,
        );
        let mut h = std::hash::DefaultHasher::new();
        self.revisions(rect, level, &mut h);
        let stamp = h.finish();
        if let Some((s, data)) = self.mips.lock().get(&(level, tx, ty))
            && *s == stamp
        {
            return data.clone();
        }
        let below = self.level_rect(level - 1);
        let src_rect = IRect::new(rect.x * 2, rect.y * 2, SOURCE_TILE * 2, SOURCE_TILE * 2);
        let src = self.pixels(src_rect.intersect(&below), level - 1);
        let mut out = zeroed(self.format, (SOURCE_TILE * SOURCE_TILE) as usize);
        let here = rect.intersect(&self.level_rect(level));
        for y in here.y..here.bottom() {
            for x in here.x..here.right() {
                // average the 2x2 block weighted by alpha, skipping pixels past the edge
                let mut acc = [0f32; 4];
                let mut n = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (x * 2 + dx, y * 2 + dy);
                    if !below.contains(sx, sy) {
                        continue;
                    }
                    let c = src.get(sx, sy);
                    acc[0] += c[0] * c[3];
                    acc[1] += c[1] * c[3];
                    acc[2] += c[2] * c[3];
                    acc[3] += c[3];
                    n += 1.0;
                }
                if n == 0.0 {
                    continue;
                }
                let c = if acc[3] > 0.0 {
                    [
                        acc[0] / acc[3],
                        acc[1] / acc[3],
                        acc[2] / acc[3],
                        acc[3] / n,
                    ]
                } else {
                    [0.0; 4]
                };
                let i = ((y - rect.y) as u32 * SOURCE_TILE + (x - rect.x) as u32) as usize;
                write_px(&mut out, i, c);
            }
        }
        let data = Arc::new(out);
        self.mips
            .lock()
            .insert((level, tx, ty), (stamp, data.clone()));
        data
    }

    /// straight alpha pixels over a level rectangle, as a cpu image of floats. used to
    /// build mips; reads for rendering go through `read` and keep the source format.
    fn pixels(&self, rect: IRect, level: u32) -> CpuImage {
        let mut out = CpuImage::transparent(rect);
        let px = self.read_level(rect, level);
        if let Some(px) = px {
            for y in px.rect.y..px.rect.bottom() {
                for x in px.rect.x..px.rect.right() {
                    let i = ((y - px.rect.y) as u32 * px.rect.w + (x - px.rect.x) as u32) as usize;
                    let o = ((y - rect.y) as u32 * rect.w + (x - rect.x) as u32) as usize;
                    out.data[o] = px.get(i);
                }
            }
        }
        out
    }

    fn read_level(&self, region: IRect, level: u32) -> Option<SourcePixels> {
        let r = region.intersect(&self.level_rect(level));
        if r.is_empty() {
            return None;
        }
        let mut data = zeroed(self.format, r.area() as usize);
        let s = SOURCE_TILE as i32;
        let level0 = if level == 0 {
            Some(self.tiles.read())
        } else {
            None
        };
        // mip tiles are built in parallel; each builds its own children the same way, so
        // a whole pyramid uses every core
        let rects = r.tiles(SOURCE_TILE);
        let sources: Vec<Arc<SourceData>> = match &level0 {
            Some(tiles) => rects
                .iter()
                .map(|t| {
                    tiles[((t.y / s) as u32 * self.cols + (t.x / s) as u32) as usize]
                        .data
                        .clone()
                })
                .collect(),
            None => rects
                .par_iter()
                .map(|t| self.mip_tile(level, (t.x / s) as u32, (t.y / s) as u32))
                .collect(),
        };
        for (tile_rect, tile) in rects.iter().zip(sources) {
            for y in tile_rect.y..tile_rect.bottom() {
                let src = ((y as u32 % SOURCE_TILE) * SOURCE_TILE
                    + (tile_rect.x as u32 % SOURCE_TILE)) as usize;
                let dst = ((y - r.y) as u32 * r.w + (tile_rect.x - r.x) as u32) as usize;
                let n = tile_rect.w as usize * 4;
                match (&mut data, &*tile) {
                    (SourceData::U8(d), SourceData::U8(t)) => {
                        d[dst * 4..dst * 4 + n].copy_from_slice(&t[src * 4..src * 4 + n]);
                    }
                    (SourceData::U16(d), SourceData::U16(t)) => {
                        d[dst * 4..dst * 4 + n].copy_from_slice(&t[src * 4..src * 4 + n]);
                    }
                    (SourceData::F32(d), SourceData::F32(t)) => {
                        d[dst * 4..dst * 4 + n].copy_from_slice(&t[src * 4..src * 4 + n]);
                    }
                    _ => {}
                }
            }
        }
        Some(SourcePixels {
            rect: r,
            format: self.format,
            alpha: AlphaMode::Straight,
            data: Arc::new(data),
        })
    }

    /// the rows and columns of tiles, for tests
    pub fn tile_grid(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }
}

impl ImageSource for TiledImage {
    fn bounds(&self) -> IRect {
        IRect::of_size(self.width, self.height)
    }

    fn key(&self, region: IRect, level: u32, h: &mut dyn Hasher) {
        h.write_u64(self.id);
        self.revisions(region, level, h);
    }

    fn read(&self, region: IRect, level: u32) -> Option<SourcePixels> {
        self.read_level(region, level)
    }

    fn to_working(&self, working: WorkingSpace) -> ColorTransform {
        to_working(&self.color, &self.luts, working)
    }
}

/// a picture computed from coordinates: any size, no memory. `f` gets level 0 pixel
/// centres and returns straight srgb colour.
pub struct ProceduralSource<F> {
    id: u64,
    width: u32,
    height: u32,
    f: F,
}

impl<F: Fn(f32, f32) -> [f32; 4] + Send + Sync + 'static> ProceduralSource<F> {
    pub fn new(width: u32, height: u32, f: F) -> Self {
        Self {
            id: next_id(),
            width,
            height,
            f,
        }
    }
}

impl<F: Fn(f32, f32) -> [f32; 4] + Send + Sync + 'static> ImageSource for ProceduralSource<F> {
    fn bounds(&self) -> IRect {
        IRect::of_size(self.width, self.height)
    }

    fn key(&self, _region: IRect, _level: u32, h: &mut dyn Hasher) {
        h.write_u64(self.id);
    }

    fn read(&self, region: IRect, level: u32) -> Option<SourcePixels> {
        let r = region.intersect(&self.bounds().at_level(level));
        if r.is_empty() {
            return None;
        }
        let s = (1u64 << level) as f32;
        let mut data = vec![0f32; r.area() as usize * 4];
        let w = r.w as usize;
        use rayon::prelude::*;
        data.par_chunks_mut(w * 4)
            .enumerate()
            .for_each(|(row, line)| {
                let y = (r.y + row as i32) as f32;
                for col in 0..w {
                    let x = (r.x + col as i32) as f32;
                    let c = (self.f)((x + 0.5) * s, (y + 0.5) * s);
                    line[col * 4..col * 4 + 4].copy_from_slice(&c);
                }
            });
        Some(SourcePixels::rgba32f(r, AlphaMode::Straight, data))
    }

    fn to_working(&self, working: WorkingSpace) -> ColorTransform {
        ColorTransform::between(ColorSpace::SRGB, working.space())
    }
}

/// renders an output of any size one tile at a time, handing each finished tile to `sink`.
/// memory stays within the engine's cache budgets whatever the output size. return
/// `Err(RenderError::Cancelled)` from the sink to stop early.
pub fn render_tiled(
    engine: &Engine,
    graph: &Graph,
    req: &RenderRequest,
    tile: u32,
    mut sink: impl FnMut(IRect, CpuImage) -> Result<(), RenderError>,
) -> Result<(), RenderError> {
    let tile = tile.clamp(64, 8192);
    for rect in req.region.tiles(tile) {
        let image = engine.render_image(
            graph,
            &RenderRequest {
                region: rect,
                ..*req
            },
        )?;
        sink(rect, image)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> SourcePixels {
        let mut d = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                d.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128, 255]);
            }
        }
        SourcePixels::rgba8(IRect::of_size(w, h), AlphaMode::Straight, d)
    }

    #[test]
    fn reads_come_back_as_written() {
        let img =
            TiledImage::from_pixels(&gradient(600, 300), SourceColor::Space(ColorSpace::SRGB));
        assert_eq!(img.tile_grid(), (3, 2));
        let px = img.read(IRect::new(250, 250, 20, 100), 0).unwrap();
        assert_eq!(px.rect, IRect::new(250, 250, 20, 50));
        let c = px.get(0);
        assert!((c[0] - 250.0 / 255.0).abs() < 1e-6 && (c[1] - 250.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn mips_average_and_follow_writes() {
        let img = TiledImage::new(
            4,
            4,
            SourceFormat::Rgba32F,
            SourceColor::Space(ColorSpace::LINEAR_SRGB),
        );
        img.write(&SourcePixels::rgba32f(
            IRect::of_size(2, 2),
            AlphaMode::Straight,
            vec![
                1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0,
            ],
        ));
        let m = img.read(IRect::of_size(2, 2), 1).unwrap();
        let c = m.get(0);
        // three opaque pixels (two red, one blue) and one clear: alpha weighted colour
        assert!((c[3] - 0.75).abs() < 1e-6);
        assert!((c[0] - 2.0 / 3.0).abs() < 1e-6 && (c[2] - 1.0 / 3.0).abs() < 1e-6);
        let mut before = std::hash::DefaultHasher::new();
        img.key(IRect::of_size(2, 2), 1, &mut before);
        img.write(&SourcePixels::rgba32f(
            IRect::new(1, 1, 1, 1),
            AlphaMode::Straight,
            vec![0.0, 1.0, 0.0, 1.0],
        ));
        let mut after = std::hash::DefaultHasher::new();
        img.key(IRect::of_size(2, 2), 1, &mut after);
        assert_ne!(before.finish(), after.finish());
        assert_eq!(img.read(IRect::of_size(2, 2), 1).unwrap().get(0)[3], 1.0);
    }

    #[test]
    fn keys_only_change_for_touched_tiles() {
        let img =
            TiledImage::from_pixels(&gradient(512, 512), SourceColor::Space(ColorSpace::SRGB));
        let key = |r: IRect| {
            let mut h = std::hash::DefaultHasher::new();
            img.key(r, 0, &mut h);
            h.finish()
        };
        let left = IRect::new(0, 0, 256, 256);
        let right = IRect::new(256, 0, 256, 256);
        let (l0, r0) = (key(left), key(right));
        img.write(&SourcePixels::rgba8(
            IRect::new(300, 10, 1, 1),
            AlphaMode::Straight,
            vec![1, 2, 3, 4],
        ));
        assert_eq!(key(left), l0);
        assert_ne!(key(right), r0);
    }
}
