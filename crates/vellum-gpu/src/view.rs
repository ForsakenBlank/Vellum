//! the viewport: which display tiles a window needs, which of those it already has, and
//! a compact binary format to send the rest.
//!
//! the window draws tiles itself with webgl, so panning, zooming and rotating never wait
//! on rust (`ARCHITECTURE.md` section 24 explains why). rust renders 256 pixel tiles at the
//! mip level that suits the zoom, remembers the cache key of every tile it sent, and only
//! sends a tile again when its key changes. an overview level that fits the whole document
//! in a few tiles is always kept up to date, so there is never a blank screen.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::color::{ColorTransform, WorkingSpace};
use crate::engine::{BackendChoice, Engine, RenderRequest};
use crate::graph::{Graph, NodeId, RenderError};
use crate::image::Precision;
use crate::rect::IRect;

/// display tiles are this many pixels on a side at their level
pub const VIEW_TILE: u32 = 256;

/// pixels rendered past each edge of a tile
pub const TILE_GUTTER: u32 = 1;

/// the overview level is the first whose whole document fits in this many pixels
const OVERVIEW_SIZE: u32 = 1024;

/// tiles rendered per gpu batch, read back with one wait
const BATCH: usize = 16;

/// where the window is looking. sizes are in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ViewState {
    /// the document pixel shown at the centre of the view
    pub center_x: f64,
    pub center_y: f64,
    /// device pixels per document pixel
    pub zoom: f64,
    /// clockwise, in degrees
    pub rotation: f64,
    pub flip_x: bool,
    pub width: u32,
    pub height: u32,
}

impl ViewState {
    /// a document pixel to a device pixel in the view
    pub fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        let (s, c) = self.rotation.to_radians().sin_cos();
        let dx = (x - self.center_x) * self.zoom * if self.flip_x { -1.0 } else { 1.0 };
        let dy = (y - self.center_y) * self.zoom;
        (
            dx * c - dy * s + f64::from(self.width) / 2.0,
            dx * s + dy * c + f64::from(self.height) / 2.0,
        )
    }

    /// a device pixel in the view to a document pixel
    pub fn to_document(&self, sx: f64, sy: f64) -> (f64, f64) {
        let (s, c) = self.rotation.to_radians().sin_cos();
        let (px, py) = (
            sx - f64::from(self.width) / 2.0,
            sy - f64::from(self.height) / 2.0,
        );
        let dx = px * c + py * s;
        let dy = -px * s + py * c;
        let zoom = self.zoom.max(1e-9);
        (
            self.center_x + dx / zoom * if self.flip_x { -1.0 } else { 1.0 },
            self.center_y + dy / zoom,
        )
    }

    /// the level 0 rectangle the view can see, rotation included
    pub fn visible(&self) -> IRect {
        let corners = [
            self.to_document(0.0, 0.0),
            self.to_document(f64::from(self.width), 0.0),
            self.to_document(0.0, f64::from(self.height)),
            self.to_document(f64::from(self.width), f64::from(self.height)),
        ];
        let lim = f64::from(i32::MAX / 4);
        let x0 = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
        let y0 = corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
        let x1 = corners
            .iter()
            .map(|c| c.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let y1 = corners
            .iter()
            .map(|c| c.1)
            .fold(f64::NEG_INFINITY, f64::max);
        IRect::from_corners(
            x0.floor().clamp(-lim, lim) as i32,
            y0.floor().clamp(-lim, lim) as i32,
            x1.ceil().clamp(-lim, lim) as i32,
            y1.ceil().clamp(-lim, lim) as i32,
        )
    }
}

/// the mip level for a zoom: the smallest picture that still has at least one pixel per
/// device pixel, so the window never magnifies a mip and only shrinks by up to half
pub fn level_for_zoom(zoom: f64) -> u32 {
    if !(zoom.is_finite()) || zoom >= 1.0 || zoom <= 0.0 {
        return 0;
    }
    ((1.0 / zoom).log2().floor() as u32).min(24)
}

/// the first level whose whole document fits in a few tiles
pub fn overview_level(doc: IRect) -> u32 {
    let mut level = 0;
    while level < 24 && doc.at_level(level).w.max(doc.at_level(level).h) > OVERVIEW_SIZE {
        level += 1;
    }
    level
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TileId {
    pub level: u32,
    /// column and row in the tile grid of that level
    pub col: i32,
    pub row: i32,
}

impl TileId {
    /// the tile's pixels at its level
    pub fn rect(&self) -> IRect {
        let s = VIEW_TILE as i32;
        IRect::new(self.col * s, self.row * s, VIEW_TILE, VIEW_TILE)
    }

    /// what is rendered for the tile: one extra pixel all round, so the window can filter
    /// across tile edges without seams
    pub fn padded(&self) -> IRect {
        self.rect().expand(TILE_GUTTER as i32)
    }
}

/// tiles covering a level 0 rectangle at a level, nearest to `center` first
fn tiles_in(area: IRect, doc: IRect, level: u32, center: (f64, f64)) -> Vec<TileId> {
    let r = area.at_level(level).intersect(&doc.at_level(level));
    if r.is_empty() {
        return Vec::new();
    }
    let s = VIEW_TILE as i32;
    let mut out = Vec::new();
    for row in r.y.div_euclid(s)..=(r.bottom() - 1).div_euclid(s) {
        for col in r.x.div_euclid(s)..=(r.right() - 1).div_euclid(s) {
            out.push(TileId { level, col, row });
        }
    }
    let scale = f64::from(1u32 << level.min(30));
    let (cx, cy) = (center.0 / scale, center.1 / scale);
    let half = f64::from(VIEW_TILE) / 2.0;
    out.sort_by(|a, b| {
        let d = |t: &TileId| {
            let (x, y) = (
                f64::from(t.col * s) + half - cx,
                f64::from(t.row * s) + half - cy,
            );
            x * x + y * y
        };
        d(a).total_cmp(&d(b))
    });
    out
}

/// how tiles are rendered for this viewport
#[derive(Debug, Clone)]
pub struct ViewSettings {
    pub precision: Precision,
    pub working: WorkingSpace,
    /// working space to the display's encoding
    pub display: ColorTransform,
    pub backend: BackendChoice,
}

impl Default for ViewSettings {
    fn default() -> Self {
        let working = WorkingSpace::default();
        Self {
            precision: Precision::F16,
            working,
            display: ColorTransform::between(working.space(), crate::color::ColorSpace::SRGB),
            backend: BackendChoice::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TileUpdate {
    pub id: TileId,
    /// the pixels' rectangle at the tile's level: the tile plus its gutter, clipped to the
    /// document
    pub rect: IRect,
    /// premultiplied rgba8 in the display's encoding
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct FrameUpdate {
    pub tiles: Vec<TileUpdate>,
    /// tiles the window holds whose content changed and are not being replaced yet
    pub dropped: Vec<TileId>,
    /// tiles still to send for this view; ask again next frame if not zero
    pub pending: u32,
    pub level: u32,
}

/// what a viewport shows: one node of a graph. `revision` goes up whenever the graph
/// changes, so the viewport knows when to look for stale tiles.
#[derive(Clone, Copy)]
pub struct Picture<'a> {
    pub graph: &'a Graph,
    pub node: NodeId,
    pub revision: u64,
}

/// one window's view of one graph
pub struct Viewport {
    doc: IRect,
    /// what the window holds: tile to the key it had when sent
    sent: HashMap<TileId, u64>,
    graph_revision: Option<u64>,
}

impl Viewport {
    pub fn new(doc: IRect) -> Self {
        Self {
            doc,
            sent: HashMap::new(),
            graph_revision: None,
        }
    }

    pub fn document(&self) -> IRect {
        self.doc
    }

    /// a new document size; the window should drop every tile
    pub fn resize(&mut self, doc: IRect) {
        self.doc = doc;
        self.sent.clear();
    }

    /// the window evicted these tiles from its cache
    pub fn forget(&mut self, ids: &[TileId]) {
        for id in ids {
            self.sent.remove(id);
        }
    }

    /// the window lost every tile, eg after a reload
    pub fn reset(&mut self) {
        self.sent.clear();
    }

    pub fn held(&self) -> usize {
        self.sent.len()
    }

    /// every tile the view wants, most important first: the overview, then the visible
    /// tiles at the view's level from the centre out, then a ring around them for panning
    pub fn wanted(&self, view: &ViewState) -> Vec<TileId> {
        let level = level_for_zoom(view.zoom).min(overview_level(self.doc));
        let overview = overview_level(self.doc);
        let visible = view.visible();
        let center = (view.center_x, view.center_y);
        let mut out = tiles_in(self.doc, self.doc, overview, center);
        if level != overview {
            out.extend(tiles_in(visible, self.doc, level, center));
            let ring = visible.expand((VIEW_TILE << level) as i32);
            let seen: HashSet<TileId> = out.iter().copied().collect();
            out.extend(
                tiles_in(ring, self.doc, level, center)
                    .into_iter()
                    .filter(|t| !seen.contains(t)),
            );
        }
        out
    }

    /// renders what the window is missing for `view`, for at most `budget` of wall time
    /// (always at least one batch). when `picture.revision` changes, tiles the window holds
    /// but this view does not need are checked for changes too.
    pub fn frame(
        &mut self,
        engine: &Engine,
        picture: &Picture,
        view: &ViewState,
        settings: &ViewSettings,
        budget: Duration,
    ) -> Result<FrameUpdate, RenderError> {
        let Picture {
            graph,
            node,
            revision: graph_revision,
        } = *picture;
        let start = Instant::now();
        let wanted = self.wanted(view);
        // the display transform is applied after the graph, so it joins every tile's key
        let display = settings.display.key();
        let tile_key = |key: u64| key.rotate_left(17) ^ display;
        let request = |id: &TileId| {
            RenderRequest::new(node, id.padded().intersect(&self.doc.at_level(id.level)))
                .level(id.level)
                .precision(settings.precision)
                .working(settings.working)
                .backend(settings.backend)
        };

        let mut update = FrameUpdate {
            level: level_for_zoom(view.zoom).min(overview_level(self.doc)),
            ..Default::default()
        };

        // tiles the window keeps for other zoom levels must not show old content
        if self.graph_revision != Some(graph_revision) {
            self.graph_revision = Some(graph_revision);
            let wanted_set: HashSet<TileId> = wanted.iter().copied().collect();
            let held: Vec<(TileId, u64)> = self
                .sent
                .iter()
                .filter(|(id, _)| !wanted_set.contains(id))
                .map(|(id, k)| (*id, *k))
                .collect();
            for (id, key) in held {
                if tile_key(engine.key(graph, &request(&id))?) != key {
                    self.sent.remove(&id);
                    update.dropped.push(id);
                }
            }
        }

        let mut stale = Vec::new();
        for id in &wanted {
            let key = tile_key(engine.key(graph, &request(id))?);
            if self.sent.get(id) != Some(&key) {
                stale.push((*id, key));
            }
        }

        let mut done = 0;
        while done < stale.len() {
            let batch = &stale[done..(done + BATCH).min(stale.len())];
            let reqs: Vec<RenderRequest> = batch.iter().map(|(id, _)| request(id)).collect();
            let pixels = engine.render_rgba8(graph, &reqs, &settings.display)?;
            for (((id, key), req), px) in batch.iter().zip(&reqs).zip(pixels) {
                self.sent.insert(*id, *key);
                update.tiles.push(TileUpdate {
                    id: *id,
                    rect: req.region,
                    pixels: px,
                });
            }
            done += batch.len();
            if start.elapsed() >= budget {
                break;
            }
        }
        update.pending = (stale.len() - done) as u32;
        Ok(update)
    }
}

/// the binary frame the window reads, little endian:
///
/// ```text
/// "VTL1"  u32 tile count  u32 dropped count  u32 pending  u32 level
/// dropped: (u32 level, i32 col, i32 row) each
/// tiles:   (u32 level, i32 col, i32 row, i32 x, i32 y, u32 w, u32 h, w*h*4 bytes) each
/// ```
pub fn pack(update: &FrameUpdate) -> Vec<u8> {
    let pixels: usize = update.tiles.iter().map(|t| t.pixels.len() + 28).sum();
    let mut out = Vec::with_capacity(20 + update.dropped.len() * 12 + pixels);
    out.extend_from_slice(b"VTL1");
    for v in [
        update.tiles.len() as u32,
        update.dropped.len() as u32,
        update.pending,
        update.level,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for d in &update.dropped {
        out.extend_from_slice(&d.level.to_le_bytes());
        out.extend_from_slice(&d.col.to_le_bytes());
        out.extend_from_slice(&d.row.to_le_bytes());
    }
    for t in &update.tiles {
        out.extend_from_slice(&t.id.level.to_le_bytes());
        out.extend_from_slice(&t.id.col.to_le_bytes());
        out.extend_from_slice(&t.id.row.to_le_bytes());
        out.extend_from_slice(&t.rect.x.to_le_bytes());
        out.extend_from_slice(&t.rect.y.to_le_bytes());
        out.extend_from_slice(&t.rect.w.to_le_bytes());
        out.extend_from_slice(&t.rect.h.to_le_bytes());
        out.extend_from_slice(&t.pixels);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(zoom: f64, rotation: f64) -> ViewState {
        ViewState {
            center_x: 1000.0,
            center_y: 500.0,
            zoom,
            rotation,
            flip_x: false,
            width: 800,
            height: 600,
        }
    }

    #[test]
    fn screen_and_document_coordinates_round_trip() {
        for (zoom, rot, flip) in [(1.0, 0.0, false), (0.3, 37.0, false), (4.0, -90.0, true)] {
            let v = ViewState {
                flip_x: flip,
                ..view(zoom, rot)
            };
            let (sx, sy) = v.to_screen(1234.5, 321.25);
            let (x, y) = v.to_document(sx, sy);
            assert!((x - 1234.5).abs() < 1e-6 && (y - 321.25).abs() < 1e-6);
        }
        let v = view(2.0, 0.0);
        assert_eq!(v.to_screen(1000.0, 500.0), (400.0, 300.0));
        assert_eq!(v.visible(), IRect::new(800, 350, 400, 300));
    }

    #[test]
    fn rotation_widens_what_is_visible() {
        let flat = view(1.0, 0.0).visible();
        let turned = view(1.0, 45.0).visible();
        assert!(turned.w > flat.w && turned.h > flat.h);
        assert_eq!(view(1.0, 90.0).visible().w, 600);
    }

    #[test]
    fn levels_follow_zoom() {
        assert_eq!(level_for_zoom(8.0), 0);
        assert_eq!(level_for_zoom(1.0), 0);
        assert_eq!(level_for_zoom(0.6), 0);
        assert_eq!(level_for_zoom(0.5), 1);
        assert_eq!(level_for_zoom(0.3), 1);
        assert_eq!(level_for_zoom(0.01), 6);
        assert_eq!(overview_level(IRect::of_size(20_000, 20_000)), 5);
        assert_eq!(overview_level(IRect::of_size(800, 600)), 0);
    }

    #[test]
    fn wanted_tiles_start_with_the_overview_then_the_centre() {
        let vp = Viewport::new(IRect::of_size(20_000, 20_000));
        let v = ViewState {
            center_x: 10_000.0,
            center_y: 10_000.0,
            ..view(1.0, 0.0)
        };
        let wanted = vp.wanted(&v);
        let overview = overview_level(IRect::of_size(20_000, 20_000));
        let first_detail = wanted.iter().position(|t| t.level == 0).unwrap();
        assert!(wanted[..first_detail].iter().all(|t| t.level == overview));
        // the first detail tile holds the centre of the view
        let t = wanted[first_detail];
        assert!(t.rect().contains(10_000, 10_000), "{t:?}");
        // no duplicates
        let set: HashSet<_> = wanted.iter().collect();
        assert_eq!(set.len(), wanted.len());
    }

    #[test]
    fn frames_pack_into_the_documented_layout() {
        let update = FrameUpdate {
            tiles: vec![TileUpdate {
                id: TileId {
                    level: 2,
                    col: -1,
                    row: 3,
                },
                rect: IRect::new(-256, 768, 2, 1),
                pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
            }],
            dropped: vec![TileId {
                level: 0,
                col: 5,
                row: 6,
            }],
            pending: 7,
            level: 2,
        };
        let b = pack(&update);
        assert_eq!(&b[..4], b"VTL1");
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let i32_at = |i: usize| i32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        assert_eq!((u32_at(4), u32_at(8), u32_at(12), u32_at(16)), (1, 1, 7, 2));
        assert_eq!((u32_at(20), i32_at(24), i32_at(28)), (0, 5, 6));
        let t = 32;
        assert_eq!((u32_at(t), i32_at(t + 4), i32_at(t + 8)), (2, -1, 3));
        assert_eq!(
            (
                i32_at(t + 12),
                i32_at(t + 16),
                u32_at(t + 20),
                u32_at(t + 24)
            ),
            (-256, 768, 2, 1)
        );
        assert_eq!(&b[t + 28..], &[1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
