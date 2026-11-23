//! sparse tiled pixels for layers, layer masks and selections.
//!
//! a raster is a map of 256 pixel tiles in document coordinates. tiles that were never
//! written do not exist and read as the raster's fill value (transparent for colour, the
//! mask's default for masks), so an empty layer costs nothing and a layer can reach past
//! the canvas without losing pixels.
//!
//! tile data sits behind an `Arc` and is copied only when written while something else
//! still holds it. that makes a clone of a whole raster a few pointer copies, which is what
//! undo snapshots and render snapshots are built on. every write gives the tile a new
//! revision from one global counter, and a revision is never reused for other pixels, so
//! the render engine can key its caches on revisions and an undo that brings old tiles back
//! hits those caches straight away.

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::IRect;
use vellum_gpu::image::{AlphaMode, SourceData, SourceFormat, SourcePixels};

/// tile size in pixels
pub const TILE: u32 = 256;
const TILE_I: i32 = TILE as i32;
const TILE_PX: usize = (TILE * TILE) as usize;
/// tiles keep a revision for each block this many pixels square, so a dab in the middle
/// of a tile does not make every view tile that touches the tile draw again
pub const BLOCK: i32 = 64;
const BLOCKS: usize = (TILE_I / BLOCK) as usize;

static NEXT_REV: AtomicU64 = AtomicU64::new(1);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// a revision no tile has had before
pub fn next_rev() -> u64 {
    NEXT_REV.fetch_add(1, Ordering::Relaxed)
}

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// bits per channel of a document
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Depth {
    #[default]
    U8,
    U16,
    F32,
}

impl Depth {
    pub fn bits(self) -> u32 {
        match self {
            Depth::U8 => 8,
            Depth::U16 => 16,
            Depth::F32 => 32,
        }
    }

    pub fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            8 => Some(Depth::U8),
            16 => Some(Depth::U16),
            32 => Some(Depth::F32),
            _ => None,
        }
    }

    fn format(self) -> SourceFormat {
        match self {
            Depth::U8 => SourceFormat::Rgba8,
            Depth::U16 => SourceFormat::Rgba16,
            Depth::F32 => SourceFormat::Rgba32F,
        }
    }
}

/// what one pixel holds
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Channels {
    /// straight alpha colour
    Rgba,
    /// one value, for masks and selections
    Gray,
}

impl Channels {
    pub fn n(self) -> usize {
        match self {
            Channels::Rgba => 4,
            Channels::Gray => 1,
        }
    }
}

/// the pixels of one tile, `TILE * TILE * channels` values
#[derive(Debug, Clone, PartialEq)]
pub enum TileData {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

impl TileData {
    pub fn filled(depth: Depth, channels: Channels, value: f32) -> Self {
        let len = TILE_PX * channels.n();
        match depth {
            Depth::U8 => TileData::U8(vec![to_u8(value); len]),
            Depth::U16 => TileData::U16(vec![to_u16(value); len]),
            Depth::F32 => TileData::F32(vec![value; len]),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            TileData::U8(d) => d.len(),
            TileData::U16(d) => d.len(),
            TileData::F32(d) => d.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> f32 {
        match self {
            TileData::U8(d) => f32::from(d[i]) * (1.0 / 255.0),
            TileData::U16(d) => f32::from(d[i]) * (1.0 / 65535.0),
            TileData::F32(d) => d[i],
        }
    }

    #[inline]
    pub fn set(&mut self, i: usize, v: f32) {
        match self {
            TileData::U8(d) => d[i] = to_u8(v),
            TileData::U16(d) => d[i] = to_u16(v),
            TileData::F32(d) => d[i] = v,
        }
    }

    pub fn bytes(&self) -> usize {
        match self {
            TileData::U8(d) => d.len(),
            TileData::U16(d) => d.len() * 2,
            TileData::F32(d) => d.len() * 4,
        }
    }

    fn depth(&self) -> Depth {
        match self {
            TileData::U8(_) => Depth::U8,
            TileData::U16(_) => Depth::U16,
            TileData::F32(_) => Depth::F32,
        }
    }

    /// true when every value is `v` as this depth stores it
    fn is_uniform(&self, v: f32) -> bool {
        match self {
            TileData::U8(d) => {
                let q = to_u8(v);
                d.iter().all(|x| *x == q)
            }
            TileData::U16(d) => {
                let q = to_u16(v);
                d.iter().all(|x| *x == q)
            }
            TileData::F32(d) => d.iter().all(|x| *x == v),
        }
    }
}

#[inline]
fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

#[inline]
fn to_u16(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
}

#[derive(Clone, Debug)]
pub struct Tile {
    pub data: Arc<TileData>,
    pub rev: u64,
    /// the revision of each block, row by row; a write gives the blocks it touched the
    /// tile's new revision and leaves the rest alone
    pub blocks: [u64; BLOCKS * BLOCKS],
}

impl Tile {
    fn new(data: Arc<TileData>, rev: u64) -> Self {
        Self {
            data,
            rev,
            blocks: [rev; BLOCKS * BLOCKS],
        }
    }

    /// a new revision for the tile and for the blocks under `local`, in tile pixels
    fn touch(&mut self, local: IRect) {
        let rev = next_rev();
        self.rev = rev;
        if local.is_empty() {
            return;
        }
        let (bx0, by0) = (local.x / BLOCK, local.y / BLOCK);
        let (bx1, by1) = ((local.right() - 1) / BLOCK, (local.bottom() - 1) / BLOCK);
        for by in by0.max(0)..=by1.min(BLOCKS as i32 - 1) {
            for bx in bx0.max(0)..=bx1.min(BLOCKS as i32 - 1) {
                self.blocks[by as usize * BLOCKS + bx as usize] = rev;
            }
        }
    }
}

/// (level, tile x, tile y) to (stamp of the level 0 tiles it was built from, tile)
type MipMap = HashMap<(u32, i32, i32), (u64, Option<Arc<TileData>>)>;

/// derived data shared by every snapshot of one raster. entries are keyed by the
/// revisions they were built from, so a snapshot can never see another's stale entry.
#[derive(Default)]
struct Caches {
    mips: Mutex<MipMap>,
    bounds: Mutex<HashMap<(i32, i32), (u64, IRect)>>,
}

#[derive(Clone)]
pub struct Raster {
    id: u64,
    channels: Channels,
    depth: Depth,
    fill: f32,
    tiles: HashMap<(i32, i32), Tile>,
    caches: Arc<Caches>,
}

impl std::fmt::Debug for Raster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Raster")
            .field("id", &self.id)
            .field("channels", &self.channels)
            .field("depth", &self.depth)
            .field("fill", &self.fill)
            .field("tiles", &self.tiles.len())
            .finish()
    }
}

/// the tile holding a pixel, and the pixel's index inside it
#[inline]
pub fn tile_of(x: i32, y: i32) -> ((i32, i32), usize) {
    let (tx, ty) = (x.div_euclid(TILE_I), y.div_euclid(TILE_I));
    let (lx, ly) = (x.rem_euclid(TILE_I), y.rem_euclid(TILE_I));
    ((tx, ty), (ly * TILE_I + lx) as usize)
}

pub fn tile_rect(tx: i32, ty: i32) -> IRect {
    IRect::new(tx * TILE_I, ty * TILE_I, TILE, TILE)
}

/// the tiles a rectangle touches, rows top to bottom
pub fn tiles_under(rect: IRect) -> Vec<(i32, i32)> {
    if rect.is_empty() {
        return Vec::new();
    }
    let (x0, y0) = (rect.x.div_euclid(TILE_I), rect.y.div_euclid(TILE_I));
    let (x1, y1) = (
        (rect.right() - 1).div_euclid(TILE_I),
        (rect.bottom() - 1).div_euclid(TILE_I),
    );
    let mut out = Vec::with_capacity(((x1 - x0 + 1) * (y1 - y0 + 1)) as usize);
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            out.push((tx, ty));
        }
    }
    out
}

impl Raster {
    pub fn new(channels: Channels, depth: Depth, fill: f32) -> Self {
        Self {
            id: next_id(),
            channels,
            depth,
            fill: if channels == Channels::Rgba {
                0.0
            } else {
                fill
            },
            tiles: HashMap::new(),
            caches: Arc::default(),
        }
    }

    pub fn rgba(depth: Depth) -> Self {
        Self::new(Channels::Rgba, depth, 0.0)
    }

    pub fn gray(depth: Depth, fill: f32) -> Self {
        Self::new(Channels::Gray, depth, fill)
    }

    /// the same pixels under a new identity, for a duplicated layer
    pub fn duplicate(&self) -> Self {
        Self {
            id: next_id(),
            caches: Arc::default(),
            ..self.clone()
        }
    }

    /// an empty raster like this one
    pub fn empty_like(&self) -> Self {
        Self::new(self.channels, self.depth, self.fill)
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn channels(&self) -> Channels {
        self.channels
    }

    pub fn depth(&self) -> Depth {
        self.depth
    }

    pub fn fill(&self) -> f32 {
        self.fill
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn tile(&self, tx: i32, ty: i32) -> Option<&Tile> {
        self.tiles.get(&(tx, ty))
    }

    pub fn tiles(&self) -> impl Iterator<Item = (&(i32, i32), &Tile)> {
        self.tiles.iter()
    }

    /// tile positions in a stable order, for writing files
    pub fn tile_keys(&self) -> Vec<(i32, i32)> {
        let mut keys: Vec<_> = self.tiles.keys().copied().collect();
        keys.sort_by_key(|&(x, y)| (y, x));
        keys
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// bytes of pixel data held, counting shared tiles once per raster
    pub fn bytes(&self) -> usize {
        self.tiles.values().map(|t| t.data.bytes()).sum()
    }

    /// true if both rasters show the same pixels without looking at them, which is the
    /// case for an undo snapshot of an unchanged raster
    pub fn same_as(&self, other: &Raster) -> bool {
        self.id == other.id
            && self.fill == other.fill
            && self.tiles.len() == other.tiles.len()
            && self
                .tiles
                .iter()
                .all(|(k, t)| other.tiles.get(k).is_some_and(|o| o.rev == t.rev))
    }

    /// puts a tile in place (or removes it with none), with a new revision
    pub fn set_tile(&mut self, tx: i32, ty: i32, data: Option<Arc<TileData>>) {
        match data {
            Some(data) => {
                debug_assert_eq!(data.len(), TILE_PX * self.channels.n());
                debug_assert_eq!(data.depth(), self.depth);
                self.tiles.insert((tx, ty), Tile::new(data, next_rev()));
            }
            None => {
                self.tiles.remove(&(tx, ty));
            }
        }
    }

    /// puts back a tile exactly as it was, revision included. only for tiles this raster
    /// had before, so a revision keeps meaning the same pixels.
    pub fn restore_tile(&mut self, tx: i32, ty: i32, tile: Option<Tile>) {
        match tile {
            Some(t) => {
                self.tiles.insert((tx, ty), t);
            }
            None => {
                self.tiles.remove(&(tx, ty));
            }
        }
    }

    /// a tile to write into, created from the fill value if missing and copied first if a
    /// snapshot still shares it. call once per tile per edit, since each call is a new
    /// revision.
    pub fn tile_mut(&mut self, tx: i32, ty: i32) -> &mut TileData {
        let (depth, channels, fill) = (self.depth, self.channels, self.fill);
        let tile = self
            .tiles
            .entry((tx, ty))
            .or_insert_with(|| Tile::new(Arc::new(TileData::filled(depth, channels, fill)), 0));
        tile.touch(IRect::new(0, 0, TILE, TILE));
        Arc::make_mut(&mut tile.data)
    }

    /// one channel of one pixel
    pub fn get(&self, x: i32, y: i32, c: usize) -> f32 {
        let (key, i) = tile_of(x, y);
        match self.tiles.get(&key) {
            Some(t) => t.data.get(i * self.channels.n() + c),
            None => self.fill,
        }
    }

    /// every value inside `rect`, row by row, `channels.n()` per pixel, as stored (straight
    /// alpha for colour). missing tiles read as the fill value.
    pub fn read(&self, rect: IRect) -> Vec<f32> {
        let n = self.channels.n();
        let mut out = vec![self.fill; rect.area() as usize * n];
        if rect.is_empty() {
            return out;
        }
        let w = rect.w as usize;
        for key in tiles_under(rect) {
            let Some(tile) = self.tiles.get(&key) else {
                continue;
            };
            let both = tile_rect(key.0, key.1).intersect(&rect);
            for y in both.y..both.bottom() {
                let src =
                    (((y - key.1 * TILE_I) * TILE_I + (both.x - key.0 * TILE_I)) as usize) * n;
                let dst = (((y - rect.y) as usize) * w + (both.x - rect.x) as usize) * n;
                for k in 0..both.w as usize * n {
                    out[dst + k] = tile.data.get(src + k);
                }
            }
        }
        out
    }

    /// writes values laid out like [`Raster::read`] into the raster. tiles left holding
    /// nothing but the fill value are dropped.
    pub fn write(&mut self, rect: IRect, values: &[f32]) {
        let n = self.channels.n();
        debug_assert_eq!(values.len(), rect.area() as usize * n);
        if rect.is_empty() {
            return;
        }
        let w = rect.w as usize;
        let keys = tiles_under(rect);
        let mut work: Vec<((i32, i32), Option<Tile>)> =
            keys.iter().map(|k| (*k, self.tiles.remove(k))).collect();
        let (depth, channels, fill) = (self.depth, self.channels, self.fill);
        work.par_iter_mut().for_each(|(key, tile)| {
            let both = tile_rect(key.0, key.1).intersect(&rect);
            let t = tile.get_or_insert_with(|| {
                Tile::new(Arc::new(TileData::filled(depth, channels, fill)), 0)
            });
            let data = Arc::make_mut(&mut t.data);
            for y in both.y..both.bottom() {
                let dst =
                    (((y - key.1 * TILE_I) * TILE_I + (both.x - key.0 * TILE_I)) as usize) * n;
                let src = (((y - rect.y) as usize) * w + (both.x - rect.x) as usize) * n;
                for k in 0..both.w as usize * n {
                    data.set(dst + k, values[src + k]);
                }
            }
            t.touch(both.translate(-key.0 * TILE_I, -key.1 * TILE_I));
            if blank(&t.data, channels, fill) {
                *tile = None;
            }
        });
        for (key, tile) in work {
            if let Some(t) = tile {
                self.tiles.insert(key, t);
            }
        }
    }

    /// changes every pixel inside `rect` in place, tiles in parallel. `f` gets the pixel's
    /// document position and its values.
    pub fn update(&mut self, rect: IRect, f: impl Fn(i32, i32, &mut [f32]) + Sync) {
        let n = self.channels.n();
        if rect.is_empty() {
            return;
        }
        let keys = tiles_under(rect);
        let mut work: Vec<((i32, i32), Option<Tile>)> =
            keys.iter().map(|k| (*k, self.tiles.remove(k))).collect();
        let (depth, channels, fill) = (self.depth, self.channels, self.fill);
        work.par_iter_mut().for_each(|(key, tile)| {
            let both = tile_rect(key.0, key.1).intersect(&rect);
            let t = tile.get_or_insert_with(|| {
                Tile::new(Arc::new(TileData::filled(depth, channels, fill)), 0)
            });
            let data = Arc::make_mut(&mut t.data);
            let mut px = [0f32; 4];
            for y in both.y..both.bottom() {
                for x in both.x..both.right() {
                    let i = (((y - key.1 * TILE_I) * TILE_I + (x - key.0 * TILE_I)) as usize) * n;
                    for (k, v) in px.iter_mut().enumerate().take(n) {
                        *v = data.get(i + k);
                    }
                    f(x, y, &mut px[..n]);
                    for (k, v) in px.iter().enumerate().take(n) {
                        data.set(i + k, *v);
                    }
                }
            }
            t.touch(both.translate(-key.0 * TILE_I, -key.1 * TILE_I));
            if blank(&t.data, channels, fill) {
                *tile = None;
            }
        });
        for (key, tile) in work {
            if let Some(t) = tile {
                self.tiles.insert(key, t);
            }
        }
    }

    /// drops every tile inside the rect area, so it reads as the fill value
    pub fn clear(&mut self) {
        self.tiles.clear();
    }

    /// sets the value pixels without tiles read as. only masks have one.
    pub fn set_fill(&mut self, fill: f32) {
        if self.channels == Channels::Gray {
            self.fill = fill;
        }
    }

    /// the area covered by tiles, a little bigger than the content
    pub fn tile_extent(&self) -> IRect {
        let mut out = IRect::EMPTY;
        for &(tx, ty) in self.tiles.keys() {
            out = out.union(&tile_rect(tx, ty));
        }
        out
    }

    /// the smallest rectangle holding every pixel that is not see through (colour) or
    /// differs from the fill value (masks)
    pub fn content_bounds(&self) -> IRect {
        let n = self.channels.n();
        let alpha = if self.channels == Channels::Rgba {
            3
        } else {
            0
        };
        let fill = self.fill;
        let mut out = IRect::EMPTY;
        let mut cache = self.caches.bounds.lock();
        for (&(tx, ty), tile) in &self.tiles {
            let local = match cache.get(&(tx, ty)) {
                Some((rev, r)) if *rev == tile.rev => *r,
                _ => {
                    let (mut x0, mut y0, mut x1, mut y1) = (TILE_I, TILE_I, -1, -1);
                    for y in 0..TILE_I {
                        for x in 0..TILE_I {
                            let v = tile.data.get(((y * TILE_I + x) as usize) * n + alpha);
                            let shows = if alpha == 3 { v > 0.0 } else { v != fill };
                            if shows {
                                x0 = x0.min(x);
                                y0 = y0.min(y);
                                x1 = x1.max(x);
                                y1 = y1.max(y);
                            }
                        }
                    }
                    let r = IRect::from_corners(x0, y0, x1 + 1, y1 + 1);
                    cache.insert((tx, ty), (tile.rev, r));
                    r
                }
            };
            if !local.is_empty() {
                out = out.union(&local.translate(tx * TILE_I, ty * TILE_I));
            }
        }
        out
    }

    /// feeds the revisions of the level 0 blocks under a level 0 rectangle into `h`.
    /// only the blocks the rectangle touches count, so a change elsewhere in a tile keeps
    /// the key.
    pub fn hash_region(&self, rect: IRect, h: &mut dyn Hasher) {
        h.write_u64(self.id);
        h.write_u32(self.fill.to_bits());
        for key in tiles_under(rect) {
            let Some(t) = self.tiles.get(&key) else {
                h.write_u64(0);
                continue;
            };
            let local = tile_rect(key.0, key.1)
                .intersect(&rect)
                .translate(-key.0 * TILE_I, -key.1 * TILE_I);
            let (bx0, by0) = (local.x / BLOCK, local.y / BLOCK);
            let (bx1, by1) = ((local.right() - 1) / BLOCK, (local.bottom() - 1) / BLOCK);
            for by in by0..=by1 {
                for bx in bx0..=bx1 {
                    h.write_u64(t.blocks[by as usize * BLOCKS + bx as usize]);
                }
            }
        }
    }

    /// one tile of a mip level: level 1 is half size. none means all fill.
    pub fn level_tile(&self, level: u32, tx: i32, ty: i32) -> Option<Arc<TileData>> {
        if level == 0 {
            return self.tiles.get(&(tx, ty)).map(|t| t.data.clone());
        }
        let span = 1i32 << level.min(20);
        let mut h = std::hash::DefaultHasher::new();
        let mut any = false;
        for y in ty * span..(ty + 1) * span {
            for x in tx * span..(tx + 1) * span {
                let rev = self.tiles.get(&(x, y)).map_or(0, |t| t.rev);
                any |= rev != 0;
                h.write_u64(rev);
            }
        }
        if !any {
            return None;
        }
        let stamp = h.finish();
        if let Some((s, data)) = self.caches.mips.lock().get(&(level, tx, ty))
            && *s == stamp
        {
            return data.clone();
        }
        let children: Vec<Option<Arc<TileData>>> = [(0, 0), (1, 0), (0, 1), (1, 1)]
            .par_iter()
            .map(|(dx, dy)| self.level_tile(level - 1, tx * 2 + dx, ty * 2 + dy))
            .collect();
        let built = self.downsample(&children);
        self.caches
            .mips
            .lock()
            .insert((level, tx, ty), (stamp, built.clone()));
        built
    }

    /// halves four child tiles (top left, top right, bottom left, bottom right) into one,
    /// alpha weighted for colour so see through pixels do not darken edges
    fn downsample(&self, children: &[Option<Arc<TileData>>]) -> Option<Arc<TileData>> {
        if children.iter().all(Option::is_none) {
            return None;
        }
        let n = self.channels.n();
        let mut out = TileData::filled(self.depth, self.channels, self.fill);
        let half = TILE_I / 2;
        for (q, child) in children.iter().enumerate() {
            let (ox, oy) = ((q as i32 % 2) * half, (q as i32 / 2) * half);
            let read = |x: i32, y: i32, k: usize| -> f32 {
                match child {
                    Some(d) => d.get(((y * TILE_I + x) as usize) * n + k),
                    None => self.fill,
                }
            };
            for y in 0..half {
                for x in 0..half {
                    let o = (((oy + y) * TILE_I + ox + x) as usize) * n;
                    let pts = [
                        (x * 2, y * 2),
                        (x * 2 + 1, y * 2),
                        (x * 2, y * 2 + 1),
                        (x * 2 + 1, y * 2 + 1),
                    ];
                    if n == 1 {
                        let s: f32 = pts.iter().map(|&(a, b)| read(a, b, 0)).sum();
                        out.set(o, s / 4.0);
                    } else {
                        let mut acc = [0f32; 4];
                        for &(a, b) in &pts {
                            let al = read(a, b, 3);
                            for (k, v) in acc.iter_mut().enumerate().take(3) {
                                *v += read(a, b, k) * al;
                            }
                            acc[3] += al;
                        }
                        if acc[3] > 0.0 {
                            for k in 0..3 {
                                out.set(o + k, acc[k] / acc[3]);
                            }
                            out.set(o + 3, acc[3] / 4.0);
                        }
                    }
                }
            }
        }
        Some(Arc::new(out))
    }

    /// pixels over a rectangle of a mip level, as source pixels for the render graph.
    /// grey rasters come back as opaque grey colour. none if nothing is there.
    pub fn read_level(&self, region: IRect, level: u32) -> Option<SourcePixels> {
        let extent = self.tile_extent().at_level(level);
        let r = region.intersect(&extent);
        if r.is_empty() {
            return None;
        }
        let n = self.channels.n();
        let count = r.area() as usize * 4;
        let fill_px = if n == 1 {
            [self.fill, self.fill, self.fill, 1.0]
        } else {
            [0.0; 4]
        };
        let mut data = match self.depth {
            Depth::U8 => SourceData::U8(vec![0; count]),
            Depth::U16 => SourceData::U16(vec![0; count]),
            Depth::F32 => SourceData::F32(vec![0.0; count]),
        };
        let w = r.w as usize;
        let put = |data: &mut SourceData, i: usize, v: f32| match data {
            SourceData::U8(d) => d[i] = to_u8(v),
            SourceData::U16(d) => d[i] = to_u16(v),
            SourceData::F32(d) => d[i] = v,
        };
        if n == 1 || self.fill != 0.0 {
            for i in 0..r.area() as usize {
                for (k, v) in fill_px.iter().enumerate() {
                    put(&mut data, i * 4 + k, *v);
                }
            }
        }
        for key in tiles_under(r) {
            let Some(tile) = self.level_tile(level, key.0, key.1) else {
                continue;
            };
            let both = tile_rect(key.0, key.1).intersect(&r);
            for y in both.y..both.bottom() {
                for x in both.x..both.right() {
                    let src = (((y - key.1 * TILE_I) * TILE_I + (x - key.0 * TILE_I)) as usize) * n;
                    let dst = (((y - r.y) as usize) * w + (x - r.x) as usize) * 4;
                    if n == 1 {
                        let v = tile.get(src);
                        put(&mut data, dst, v);
                        put(&mut data, dst + 1, v);
                        put(&mut data, dst + 2, v);
                    } else {
                        // raw copies keep 16 bit values exact
                        match (&mut data, &*tile) {
                            (SourceData::U8(d), TileData::U8(t)) => {
                                d[dst..dst + 4].copy_from_slice(&t[src..src + 4])
                            }
                            (SourceData::U16(d), TileData::U16(t)) => {
                                d[dst..dst + 4].copy_from_slice(&t[src..src + 4])
                            }
                            (SourceData::F32(d), TileData::F32(t)) => {
                                d[dst..dst + 4].copy_from_slice(&t[src..src + 4])
                            }
                            (d, t) => {
                                for k in 0..4 {
                                    put(d, dst + k, t.get(src + k));
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(SourcePixels {
            rect: r,
            format: self.depth.format(),
            alpha: if n == 1 {
                AlphaMode::Opaque
            } else {
                AlphaMode::Straight
            },
            data: Arc::new(data),
        })
    }

    /// a colour raster from tightly packed straight rgba8 pixels placed at the origin
    pub fn from_rgba8(width: u32, height: u32, pixels: &[u8], depth: Depth) -> Self {
        let mut r = Self::rgba(depth);
        let values: Vec<f32> = pixels.iter().map(|v| f32::from(*v) / 255.0).collect();
        r.write(IRect::of_size(width, height), &values);
        r
    }

    /// the same pixels at another depth
    pub fn to_depth(&self, depth: Depth) -> Self {
        if depth == self.depth {
            return self.clone();
        }
        let mut out = Self::new(self.channels, depth, self.fill);
        out.id = self.id;
        for (&(tx, ty), tile) in &self.tiles {
            let mut data = TileData::filled(depth, self.channels, self.fill);
            for i in 0..tile.data.len() {
                data.set(i, tile.data.get(i));
            }
            out.set_tile(tx, ty, Some(Arc::new(data)));
        }
        out.id = next_id();
        out
    }
}

/// true when a tile shows nothing: fully transparent colour, or a mask at its fill value
fn blank(data: &TileData, channels: Channels, fill: f32) -> bool {
    match channels {
        Channels::Gray => data.is_uniform(fill),
        Channels::Rgba => match data {
            TileData::U8(d) => d
                .as_chunks::<4>().0.iter()
                .all(|p| p[3] == 0 && p[0] == 0 && p[1] == 0 && p[2] == 0),
            TileData::U16(d) => d.as_chunks::<4>().0.iter().all(|p| *p == [0, 0, 0, 0]),
            TileData::F32(d) => d
                .as_chunks::<4>().0.iter()
                .all(|p| p[3] == 0.0 && p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_come_back_as_written_across_tiles() {
        let mut r = Raster::rgba(Depth::U8);
        let rect = IRect::new(-10, 250, 20, 12);
        let values: Vec<f32> = (0..rect.area() * 4)
            .map(|i| (i % 256) as f32 / 255.0)
            .collect();
        r.write(rect, &values);
        assert_eq!(r.tile_count(), 4);
        let back = r.read(rect);
        for (a, b) in values.iter().zip(&back) {
            assert!((a - b).abs() < 1.0 / 510.0);
        }
        assert_eq!(r.get(-100, -100, 3), 0.0);
    }

    #[test]
    fn clones_share_tiles_until_written() {
        let mut a = Raster::gray(Depth::U8, 0.0);
        a.write(IRect::new(0, 0, 2, 2), &[1.0; 4]);
        let b = a.clone();
        assert!(a.same_as(&b));
        a.update(IRect::new(0, 0, 1, 1), |_, _, v| v[0] = 0.5);
        assert!(!a.same_as(&b));
        assert_eq!(b.get(0, 0, 0), 1.0);
        assert!((a.get(0, 0, 0) - 0.5).abs() < 0.01);
    }

    #[test]
    fn blank_tiles_are_dropped_and_masks_keep_their_fill() {
        let mut m = Raster::gray(Depth::U8, 1.0);
        assert_eq!(m.get(5, 5, 0), 1.0);
        m.write(IRect::new(0, 0, 4, 4), &[0.0; 16]);
        assert_eq!(m.tile_count(), 1);
        m.write(IRect::new(0, 0, 4, 4), &[1.0; 16]);
        assert_eq!(m.tile_count(), 0);
    }

    #[test]
    fn content_bounds_are_tight() {
        let mut r = Raster::rgba(Depth::U16);
        r.write(IRect::new(300, 40, 3, 2), &[0.5; 24]);
        assert_eq!(r.content_bounds(), IRect::new(300, 40, 3, 2));
        assert_eq!(r.tile_extent(), IRect::new(256, 0, 256, 256));
    }

    #[test]
    fn keys_change_only_where_a_tile_was_painted() {
        let mut r = Raster::rgba(Depth::U8);
        r.write(IRect::new(0, 0, 256, 256), &vec![0.5; 256 * 256 * 4]);
        let key = |r: &Raster, rect: IRect| {
            let mut h = std::hash::DefaultHasher::new();
            r.hash_region(rect, &mut h);
            h.finish()
        };
        let far = IRect::new(200, 200, 40, 40);
        let near = IRect::new(0, 0, 40, 40);
        let (far_before, near_before) = (key(&r, far), key(&r, near));
        let old = r.tile(0, 0).cloned();
        r.update(IRect::new(10, 10, 5, 5), |_, _, px| px[0] = 1.0);
        assert_eq!(key(&r, far), far_before, "an untouched block keeps its key");
        assert_ne!(key(&r, near), near_before);
        // undo puts the old tile back, blocks and all, so the old keys come back
        r.restore_tile(0, 0, old);
        assert_eq!(key(&r, near), near_before);
    }

    #[test]
    fn mips_average_and_follow_writes() {
        let mut r = Raster::rgba(Depth::F32);
        r.write(
            IRect::new(0, 0, 2, 2),
            &[
                1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0,
            ],
        );
        let px = r.read_level(IRect::new(0, 0, 1, 1), 1).unwrap();
        let c = px.get(0);
        assert!((c[3] - 0.75).abs() < 1e-6);
        assert!((c[0] - 2.0 / 3.0).abs() < 1e-6 && (c[2] - 1.0 / 3.0).abs() < 1e-6);
        let mut before = std::hash::DefaultHasher::new();
        r.hash_region(IRect::new(0, 0, 2, 2), &mut before);
        r.update(IRect::new(1, 1, 1, 1), |_, _, v| v[3] = 1.0);
        let mut after = std::hash::DefaultHasher::new();
        r.hash_region(IRect::new(0, 0, 2, 2), &mut after);
        assert_ne!(before.finish(), after.finish());
        assert_eq!(
            r.read_level(IRect::new(0, 0, 1, 1), 1).unwrap().get(0)[3],
            1.0
        );
        // a deep level of an empty area is nothing at all
        assert!(r.read_level(IRect::new(100, 100, 4, 4), 3).is_none());
    }
}
