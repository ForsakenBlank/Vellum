//! painting: the brush engine, the paint bucket, gradients and the eyedropper.
//!
//! a brush stroke works like photoshop's. dabs are stamped along the path every
//! `spacing` of the brush size; each dab adds `flow` to a coverage mask for the stroke, so
//! going over the same spot builds up, and `opacity` caps how far the whole stroke can go.
//! the stroke is composited onto the pixels it started from, never onto itself, so the
//! cap holds however many times the brush passes.
//!
//! [`Stroke`] paints incrementally while the pointer moves and keeps the result in its own
//! raster, which the window shows as a preview. when the stroke ends that raster becomes
//! the layer's pixels in one undoable step.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::blend::{composite, pixel_noise};
use vellum_gpu::{BlendMode, IRect, Transfer};

use crate::compose::Part;
use crate::document::{EditTarget, ImageDocument, LayerId};
use crate::raster::{Channels, Depth, Raster, tiles_under};
use crate::selection::{ColorBuf, magic_wand};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BrushTool {
    #[default]
    Brush,
    /// hard edged, no anti aliasing
    Pencil,
    Eraser,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct BrushSettings {
    pub tool: BrushTool,
    /// diameter in document pixels
    pub size: f32,
    /// 0 is soft all the way from the middle, 1 is a hard edge
    pub hardness: f32,
    /// 0 to 1, the most one stroke can cover
    pub opacity: f32,
    /// 0 to 1, how much each dab adds
    pub flow: f32,
    /// distance between dabs, as a share of the size
    pub spacing: f32,
    /// 0 to 1, how strongly the line is steadied
    pub smoothing: f32,
    /// how far dabs stray from the line, as a share of the size
    pub scatter: f32,
    /// dabs per step when scattering
    pub count: u32,
    /// tip rotation in degrees, and how round it is (1 is a circle)
    pub angle: f32,
    pub roundness: f32,
    /// 0 to 1, how much the paper texture shows through
    pub texture_depth: f32,
    /// size of the texture's grain in pixels
    pub texture_scale: f32,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    pub pressure_flow: bool,
    /// the smallest size at no pressure, as a share of the size
    pub min_size: f32,
    /// pen tilt flattens and turns the tip
    pub tilt: bool,
    pub mode: BlendMode,
    /// straight srgb, what the brush paints with
    pub color: [f32; 3],
    /// what the eraser paints where see through pixels are locked, and on masks
    pub background: [f32; 3],
    /// seed for scatter, so a replayed stroke looks the same
    pub seed: u32,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            tool: BrushTool::Brush,
            size: 30.0,
            hardness: 0.8,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.1,
            smoothing: 0.1,
            scatter: 0.0,
            count: 1,
            angle: 0.0,
            roundness: 1.0,
            texture_depth: 0.0,
            texture_scale: 100.0,
            pressure_size: true,
            pressure_opacity: false,
            pressure_flow: false,
            min_size: 0.0,
            tilt: false,
            mode: BlendMode::Normal,
            color: [0.0; 3],
            background: [1.0; 3],
            seed: 1,
        }
    }
}

/// one pointer sample, in document pixels. pressure is 0 to 1 (a mouse sends 1), tilt is
/// in degrees from upright on each axis, like pointer events.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    #[serde(default = "full")]
    pub pressure: f32,
    #[serde(default)]
    pub tilt_x: f32,
    #[serde(default)]
    pub tilt_y: f32,
}

fn full() -> f32 {
    1.0
}

/// the value written for a straight srgb colour in a document: 8 and 16 bit documents
/// keep encoded values, 32 bit ones are linear
pub fn doc_color(doc: &ImageDocument, srgb: [f32; 3]) -> [f32; 3] {
    match doc.depth {
        Depth::F32 => srgb.map(|v| Transfer::Srgb.decode(v)),
        _ => srgb,
    }
}

fn luminance(c: [f32; 3]) -> f32 {
    (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).clamp(0.0, 1.0)
}

/// the raster a part of the document lives in, and whether painting it is allowed
pub fn part_raster<'a>(doc: &'a ImageDocument, part: &Part) -> Result<&'a Raster, String> {
    match part {
        Part::Pixels(id) => {
            let layer = doc.layer(*id).ok_or("That layer is gone.")?;
            if layer.locks.pixels_locked() {
                return Err(format!(
                    "{} has its pixels locked. Unlock it in the Layers panel to paint.",
                    layer.name
                ));
            }
            layer.raster().ok_or_else(|| {
                "Groups have no pixels to paint. Pick a layer inside the group.".into()
            })
        }
        Part::Mask(id) => doc
            .layer(*id)
            .and_then(|l| l.mask.as_ref())
            .map(|m| &m.raster)
            .ok_or_else(|| "That layer has no mask.".into()),
        Part::Selection => Err("The selection is painted in quick mask mode.".into()),
    }
}

/// the part tools change right now: the selection in quick mask, the mask when it is
/// targeted, the active layer's pixels otherwise
pub fn active_part(doc: &ImageDocument) -> Result<Part, String> {
    if doc.quick_mask {
        return Ok(Part::Selection);
    }
    let id: LayerId = doc.active.ok_or("Pick a layer to paint on first.")?;
    let layer = doc.layer(id).ok_or("Pick a layer to paint on first.")?;
    Ok(match doc.target {
        EditTarget::Mask if layer.mask.is_some() => Part::Mask(id),
        _ => Part::Pixels(id),
    })
}

/// the raster of a part, including the selection for quick mask painting
pub fn raster_for(doc: &ImageDocument, part: &Part) -> Result<Raster, String> {
    match part {
        Part::Selection => Ok(doc
            .selection
            .clone()
            .unwrap_or_else(|| Raster::gray(Depth::U8, 0.0))),
        other => part_raster(doc, other).cloned(),
    }
}

/// writes a raster back into its part of the document
pub fn set_part(doc: &mut ImageDocument, part: &Part, raster: Raster) {
    match part {
        Part::Pixels(id) => {
            if let Some(r) = doc.layer_mut(*id).and_then(|l| l.raster_mut()) {
                *r = raster;
            }
        }
        Part::Mask(id) => {
            if let Some(m) = doc.layer_mut(*id).and_then(|l| l.mask.as_mut()) {
                m.raster = raster;
            }
        }
        Part::Selection => {
            doc.selection = (!raster.is_empty()).then_some(raster);
        }
    }
}

/// how painted coverage lands on a part's pixels
#[derive(Clone)]
struct Painter {
    channels: Channels,
    tool: BrushTool,
    mode: BlendMode,
    color: [f32; 3],
    background: [f32; 3],
    lock_alpha: bool,
    clamp: bool,
}

impl Painter {
    fn new(
        doc: &ImageDocument,
        part: &Part,
        tool: BrushTool,
        mode: BlendMode,
        color: [f32; 3],
        background: [f32; 3],
    ) -> Self {
        let lock_alpha = match part {
            Part::Pixels(id) => doc
                .layer(*id)
                .is_some_and(|l| l.locks.transparency || l.locks.all),
            _ => false,
        };
        let channels = if matches!(part, Part::Pixels(_)) {
            Channels::Rgba
        } else {
            Channels::Gray
        };
        Self {
            channels,
            tool,
            mode,
            color: doc_color(doc, color),
            background: doc_color(doc, background),
            lock_alpha,
            clamp: doc.depth != Depth::F32,
        }
    }

    /// `base` is the pixel before painting, `a` how much paint lands (0 to 1)
    fn apply(&self, base: &[f32], out: &mut [f32], a: f32, x: i32, y: i32) {
        if a <= 0.0 {
            out.copy_from_slice(base);
            return;
        }
        if self.channels == Channels::Gray {
            let target = match self.tool {
                BrushTool::Eraser => luminance(self.background),
                _ => luminance(self.color),
            };
            out[0] = base[0] + (target - base[0]) * a;
            return;
        }
        let erasing = self.tool == BrushTool::Eraser;
        if erasing && !self.lock_alpha {
            out[..3].copy_from_slice(&base[..3]);
            out[3] = base[3] * (1.0 - a);
            return;
        }
        let color = if erasing { self.background } else { self.color };
        let ba = if self.lock_alpha { 1.0 } else { base[3] };
        let backdrop = [base[0] * ba, base[1] * ba, base[2] * ba, ba];
        let mixed = composite(
            self.mode,
            backdrop,
            [color[0], color[1], color[2], 1.0],
            a,
            self.clamp,
            x,
            y,
        );
        let oa = if self.lock_alpha { base[3] } else { mixed[3] };
        if mixed[3] > 0.0 {
            for k in 0..3 {
                out[k] = mixed[k] / mixed[3];
            }
        } else {
            out[..3].copy_from_slice(&base[..3]);
        }
        out[3] = oa;
    }
}

/// steadies the line: the brush hangs on a string behind the pointer and only moves when
/// the string is pulled tight
#[derive(Debug, Clone)]
struct Smoother {
    length: f32,
    at: Option<StrokePoint>,
    last_input: Option<StrokePoint>,
}

impl Smoother {
    fn new(smoothing: f32, size: f32) -> Self {
        let s = smoothing.clamp(0.0, 1.0);
        Self {
            length: s * s * (40.0 + size * 0.5),
            at: None,
            last_input: None,
        }
    }

    fn push(&mut self, p: StrokePoint) -> Option<StrokePoint> {
        self.last_input = Some(p);
        let Some(at) = self.at else {
            self.at = Some(p);
            return Some(p);
        };
        let d = (p.x - at.x).hypot(p.y - at.y);
        if d <= self.length {
            return None;
        }
        let t = (d - self.length) / d;
        let next = StrokePoint {
            x: at.x + (p.x - at.x) * t,
            y: at.y + (p.y - at.y) * t,
            pressure: at.pressure + (p.pressure - at.pressure) * t,
            tilt_x: p.tilt_x,
            tilt_y: p.tilt_y,
        };
        self.at = Some(next);
        Some(next)
    }

    /// the rest of the way to where the pointer let go
    fn finish(&mut self) -> Option<StrokePoint> {
        let last = self.last_input?;
        let at = self.at?;
        if (last.x - at.x).hypot(last.y - at.y) < 0.01 {
            return None;
        }
        self.at = Some(last);
        Some(last)
    }
}

/// a small random number generator, so scatter replays the same way
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// a tileable grain for the texture option, 0 to 1
fn grain(x: f32, y: f32, scale: f32) -> f32 {
    let s = scale.max(1.0) / 8.0;
    let (fx, fy) = (x / s, y / s);
    let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let v = |a: i32, b: i32| pixel_noise(a.rem_euclid(512), b.rem_euclid(512));
    let top = v(ix, iy) + (v(ix + 1, iy) - v(ix, iy)) * sx;
    let bottom = v(ix, iy + 1) + (v(ix + 1, iy + 1) - v(ix, iy + 1)) * sx;
    top + (bottom - top) * sy
}

/// a stroke in progress
pub struct Stroke {
    pub part: Part,
    settings: BrushSettings,
    painter: Painter,
    base: Raster,
    /// coverage built up by the dabs, 0 to 1
    coverage: Raster,
    /// the opacity each pixel may reach, the highest of the dabs over it
    cap: Raster,
    result: Raster,
    selection: Option<Raster>,
    smoother: Smoother,
    last: Option<StrokePoint>,
    /// distance walked since the last dab
    carry: f32,
    rng: Rng,
    /// every point that went in, for the command that records the stroke
    points: Vec<StrokePoint>,
    changed: IRect,
}

impl Stroke {
    pub fn new(doc: &ImageDocument, settings: BrushSettings) -> Result<Self, String> {
        let part = active_part(doc)?;
        let base = raster_for(doc, &part)?;
        let painter = Painter::new(
            doc,
            &part,
            settings.tool,
            settings.mode,
            settings.color,
            settings.background,
        );
        let selection = match part {
            Part::Selection => None,
            _ => doc.selection.clone(),
        };
        Ok(Self {
            smoother: Smoother::new(settings.smoothing, settings.size),
            rng: Rng(u64::from(settings.seed.max(1)) * 0x9E37_79B9_7F4A_7C15),
            coverage: Raster::gray(Depth::F32, 0.0),
            cap: Raster::gray(Depth::U16, 0.0),
            result: base.clone(),
            part,
            settings,
            painter,
            base,
            selection,
            last: None,
            carry: 0.0,
            points: Vec::new(),
            changed: IRect::EMPTY,
        })
    }

    /// the pixels with the stroke so far
    pub fn result(&self) -> &Raster {
        &self.result
    }

    pub fn points(&self) -> &[StrokePoint] {
        &self.points
    }

    pub fn settings(&self) -> &BrushSettings {
        &self.settings
    }

    /// everything the stroke has touched so far
    pub fn changed(&self) -> IRect {
        self.changed
    }

    /// paints along more of the path. returns the area that changed.
    pub fn add(&mut self, points: &[StrokePoint]) -> IRect {
        let mut dirty = IRect::EMPTY;
        for p in points {
            self.points.push(*p);
            if let Some(s) = self.smoother.push(*p) {
                dirty = dirty.union(&self.walk(s));
            }
        }
        self.composite(dirty);
        dirty
    }

    /// paints the last bit of a steadied line and returns the area that changed
    pub fn finish(&mut self) -> IRect {
        let dirty = match self.smoother.finish() {
            Some(p) => self.walk(p),
            None => IRect::EMPTY,
        };
        self.composite(dirty);
        dirty
    }

    fn size_at(&self, pressure: f32) -> f32 {
        let s = &self.settings;
        let p = if s.pressure_size {
            s.min_size.clamp(0.0, 1.0) + (1.0 - s.min_size.clamp(0.0, 1.0)) * pressure
        } else {
            1.0
        };
        (s.size * p).max(1.0)
    }

    /// stamps dabs from the last point to `p`
    fn walk(&mut self, p: StrokePoint) -> IRect {
        let Some(last) = self.last else {
            self.last = Some(p);
            return self.dab(p);
        };
        let len = (p.x - last.x).hypot(p.y - last.y);
        let mut dirty = IRect::EMPTY;
        let mut walked = 0.0;
        loop {
            let at = walked / len.max(1e-6);
            let pressure = last.pressure + (p.pressure - last.pressure) * at.min(1.0);
            let step = (self.settings.spacing.max(0.01) * self.size_at(pressure)).max(0.5);
            let need = step - self.carry;
            if walked + need > len {
                self.carry += len - walked;
                break;
            }
            walked += need;
            self.carry = 0.0;
            let t = walked / len.max(1e-6);
            let q = StrokePoint {
                x: last.x + (p.x - last.x) * t,
                y: last.y + (p.y - last.y) * t,
                pressure: last.pressure + (p.pressure - last.pressure) * t,
                tilt_x: last.tilt_x + (p.tilt_x - last.tilt_x) * t,
                tilt_y: last.tilt_y + (p.tilt_y - last.tilt_y) * t,
            };
            let dir = (
                (p.x - last.x) / len.max(1e-6),
                (p.y - last.y) / len.max(1e-6),
            );
            dirty = dirty.union(&self.scattered(q, dir));
        }
        self.last = Some(p);
        dirty
    }

    fn scattered(&mut self, p: StrokePoint, dir: (f32, f32)) -> IRect {
        let s = &self.settings;
        if s.scatter <= 0.0 {
            return self.dab(p);
        }
        let mut dirty = IRect::EMPTY;
        let size = self.size_at(p.pressure);
        for _ in 0..s.count.clamp(1, 16) {
            let off = (self.rng.next() * 2.0 - 1.0) * self.settings.scatter * size;
            let along = (self.rng.next() * 2.0 - 1.0) * self.settings.scatter * size * 0.25;
            let q = StrokePoint {
                x: p.x - dir.1 * off + dir.0 * along,
                y: p.y + dir.0 * off + dir.1 * along,
                ..p
            };
            dirty = dirty.union(&self.dab(q));
        }
        dirty
    }

    /// stamps one dab into the coverage and cap masks
    fn dab(&mut self, p: StrokePoint) -> IRect {
        let s = &self.settings;
        let pencil = s.tool == BrushTool::Pencil;
        let radius = self.size_at(p.pressure) / 2.0;
        let flow =
            if pencil { 1.0 } else { s.flow } * if s.pressure_flow { p.pressure } else { 1.0 };
        let opacity = s.opacity * if s.pressure_opacity { p.pressure } else { 1.0 };
        let (mut angle, mut round) = (s.angle.to_radians(), s.roundness.clamp(0.05, 1.0));
        if s.tilt {
            let mag = p.tilt_x.hypot(p.tilt_y).min(90.0);
            if mag > 1.0 {
                angle = p.tilt_y.atan2(p.tilt_x);
                round *= 1.0 - 0.7 * mag / 90.0;
            }
        }
        let (sin, cos) = angle.sin_cos();
        let hard = if pencil {
            1.0
        } else {
            s.hardness.clamp(0.0, 1.0)
        };
        let texture = (s.texture_depth.clamp(0.0, 1.0), s.texture_scale);
        let rect = IRect::from_corners(
            (p.x - radius - 1.0).floor() as i32,
            (p.y - radius - 1.0).floor() as i32,
            (p.x + radius + 1.0).ceil() as i32,
            (p.y + radius + 1.0).ceil() as i32,
        );
        let tip = move |x: i32, y: i32| -> f32 {
            let (dx, dy) = (x as f32 + 0.5 - p.x, y as f32 + 0.5 - p.y);
            // into the tip's own frame, where it is a unit circle
            let u = dx * cos + dy * sin;
            let v = (-dx * sin + dy * cos) / round;
            let dist = u.hypot(v);
            if pencil {
                return f32::from(u8::from(dist <= radius.max(0.5)));
            }
            let edge = (radius - dist + 0.5).clamp(0.0, 1.0);
            if edge <= 0.0 {
                return 0.0;
            }
            let d = dist / radius.max(1e-3);
            let fall = if d <= hard || hard >= 1.0 {
                1.0
            } else {
                let t = ((d - hard) / (1.0 - hard)).clamp(0.0, 1.0);
                1.0 - t * t * (3.0 - 2.0 * t)
            };
            let mut a = fall * edge;
            if texture.0 > 0.0 {
                a *= 1.0 - texture.0 * (1.0 - grain(x as f32, y as f32, texture.1));
            }
            a
        };
        self.coverage.update(rect, |x, y, v| {
            let a = tip(x, y) * flow;
            v[0] += a * (1.0 - v[0]);
        });
        self.cap.update(rect, |x, y, v| {
            if tip(x, y) > 0.0 {
                v[0] = v[0].max(opacity);
            }
        });
        rect
    }

    /// redraws the result over `dirty` from the untouched base
    fn composite(&mut self, dirty: IRect) {
        if dirty.is_empty() {
            return;
        }
        self.changed = self.changed.union(&dirty);
        let n = self.base.channels().n();
        let (base, coverage, cap, sel, painter) = (
            &self.base,
            &self.coverage,
            &self.cap,
            &self.selection,
            &self.painter,
        );
        self.result.update(dirty, |x, y, out| {
            let mut px = [0f32; 4];
            for (k, v) in px.iter_mut().enumerate().take(n) {
                *v = base.get(x, y, k);
            }
            let a = coverage.get(x, y, 0)
                * cap.get(x, y, 0)
                * sel.as_ref().map_or(1.0, |s| s.get(x, y, 0));
            painter.apply(&px[..n], out, a, x, y);
        });
    }
}

/// paint bucket settings
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct FillSettings {
    /// 0 to 255, like photoshop
    pub tolerance: f32,
    pub contiguous: bool,
    pub anti_alias: bool,
    /// pick the area from what every layer shows, not just the current one
    pub sample_all: bool,
    pub opacity: f32,
    pub mode: BlendMode,
    pub color: [f32; 3],
}

impl Default for FillSettings {
    fn default() -> Self {
        Self {
            tolerance: 32.0,
            contiguous: true,
            anti_alias: true,
            sample_all: false,
            opacity: 1.0,
            mode: BlendMode::Normal,
            color: [0.0; 3],
        }
    }
}

/// paints `color` into a part wherever `amount` (0 to 1) says, limited by the selection
pub fn paint_area(
    doc: &mut ImageDocument,
    part: &Part,
    area: IRect,
    color: [f32; 3],
    mode: BlendMode,
    amount: &(dyn Fn(i32, i32) -> f32 + Sync),
) -> Result<(), String> {
    let painter = Painter::new(doc, part, BrushTool::Brush, mode, color, color);
    let mut raster = raster_for(doc, part)?;
    let sel = match part {
        Part::Selection => None,
        _ => doc.selection.clone(),
    };
    let n = raster.channels().n();
    let base = raster.clone();
    raster.update(area, |x, y, out| {
        let mut px = [0f32; 4];
        for (k, v) in px.iter_mut().enumerate().take(n) {
            *v = base.get(x, y, k);
        }
        let a = amount(x, y) * sel.as_ref().map_or(1.0, |s| s.get(x, y, 0));
        painter.apply(&px[..n], out, a, x, y);
    });
    set_part(doc, part, raster);
    Ok(())
}

/// the paint bucket: fills the area the magic wand would pick at (x, y). `sample` holds
/// the pixels the area is picked from (the layer, or everything when sampling all layers).
pub fn bucket(
    doc: &mut ImageDocument,
    sample: &ColorBuf,
    x: i32,
    y: i32,
    settings: &FillSettings,
) -> Result<(), String> {
    let part = active_part(doc)?;
    let area = magic_wand(
        sample,
        (x, y),
        settings.tolerance,
        settings.contiguous,
        settings.anti_alias,
    );
    let opacity = settings.opacity.clamp(0.0, 1.0);
    paint_area(
        doc,
        &part,
        area.rect,
        settings.color,
        settings.mode,
        &|px, py| area.get(px, py) * opacity,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GradientKind {
    #[default]
    Linear,
    Radial,
    Angle,
    Reflected,
    Diamond,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColorStop {
    /// 0 to 1 along the gradient
    pub at: f32,
    /// straight srgb with alpha
    pub color: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct GradientSettings {
    pub kind: GradientKind,
    pub stops: Vec<ColorStop>,
    pub reverse: bool,
    /// a touch of noise so smooth gradients do not band in 8 bit documents
    pub dither: bool,
    pub opacity: f32,
    pub mode: BlendMode,
}

impl Default for GradientSettings {
    fn default() -> Self {
        Self {
            kind: GradientKind::Linear,
            stops: vec![
                ColorStop {
                    at: 0.0,
                    color: [0.0, 0.0, 0.0, 1.0],
                },
                ColorStop {
                    at: 1.0,
                    color: [1.0, 1.0, 1.0, 1.0],
                },
            ],
            reverse: false,
            dither: true,
            opacity: 1.0,
            mode: BlendMode::Normal,
        }
    }
}

/// where a pixel falls along a gradient drawn from `a` to `b`, 0 to 1
pub fn gradient_t(kind: GradientKind, a: [f32; 2], b: [f32; 2], x: f32, y: f32) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    let len = len2.sqrt();
    let (px, py) = (x - a[0], y - a[1]);
    match kind {
        GradientKind::Linear => ((px * dx + py * dy) / len2).clamp(0.0, 1.0),
        GradientKind::Reflected => ((px * dx + py * dy) / len2).abs().min(1.0),
        GradientKind::Radial => (px.hypot(py) / len).min(1.0),
        GradientKind::Diamond => {
            // distance along the drawn direction and across it, the larger wins
            let along = (px * dx + py * dy).abs() / len;
            let across = (px * dy - py * dx).abs() / len;
            (along.max(across) / len).min(1.0)
        }
        GradientKind::Angle => {
            let start = dy.atan2(dx);
            let ang = py.atan2(px) - start;
            // clockwise on screen from the drawn line, all the way round
            ang.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU
        }
    }
}

/// the colour at `t` from sorted stops, straight with alpha
pub fn stop_color(stops: &[ColorStop], t: f32) -> [f32; 4] {
    match stops {
        [] => [0.0; 4],
        [only] => only.color,
        _ => {
            if t <= stops[0].at {
                return stops[0].color;
            }
            for pair in stops.windows(2) {
                let (s0, s1) = (pair[0], pair[1]);
                if t <= s1.at {
                    let f = if s1.at > s0.at {
                        (t - s0.at) / (s1.at - s0.at)
                    } else {
                        1.0
                    };
                    return std::array::from_fn(|k| s0.color[k] + (s1.color[k] - s0.color[k]) * f);
                }
            }
            stops[stops.len() - 1].color
        }
    }
}

/// fills the selection (or the canvas) with a gradient drawn from `a` to `b`
pub fn gradient(
    doc: &mut ImageDocument,
    a: [f32; 2],
    b: [f32; 2],
    settings: &GradientSettings,
) -> Result<(), String> {
    let part = active_part(doc)?;
    let area = match (&part, doc.edit_area()) {
        (Part::Selection, _) | (_, None) => doc.canvas(),
        (_, Some(r)) => r.intersect(&doc.canvas()),
    };
    let mut stops = settings.stops.clone();
    stops.sort_by(|p, q| p.at.total_cmp(&q.at));
    let dither = settings.dither && doc.depth == Depth::U8;
    let opacity = settings.opacity.clamp(0.0, 1.0);
    let painter_color = |x: i32, y: i32| {
        let mut t = gradient_t(settings.kind, a, b, x as f32 + 0.5, y as f32 + 0.5);
        if settings.reverse {
            t = 1.0 - t;
        }
        let mut c = stop_color(&stops, t);
        if dither {
            let n = (pixel_noise(x, y) - 0.5) / 255.0;
            for v in c.iter_mut().take(3) {
                *v += n;
            }
        }
        c
    };
    // each pixel has its own colour, so paint pixel by pixel through the same compositing
    let n = raster_for(doc, &part)?.channels().n();
    let sel = match part {
        Part::Selection => None,
        _ => doc.selection.clone(),
    };
    let lock_alpha = match &part {
        Part::Pixels(id) => doc
            .layer(*id)
            .is_some_and(|l| l.locks.transparency || l.locks.all),
        _ => false,
    };
    let clamp = doc.depth != Depth::F32;
    let linear = doc.depth == Depth::F32;
    let mode = settings.mode;
    let mut raster = raster_for(doc, &part)?;
    let base = raster.clone();
    raster.update(area, |x, y, out| {
        let mut c = painter_color(x, y);
        if linear {
            for v in c.iter_mut().take(3) {
                *v = Transfer::Srgb.decode(*v);
            }
        }
        let a = c[3] * opacity * sel.as_ref().map_or(1.0, |s| s.get(x, y, 0));
        let mut px = [0f32; 4];
        for (k, v) in px.iter_mut().enumerate().take(n) {
            *v = base.get(x, y, k);
        }
        if n == 1 {
            out[0] = px[0] + (luminance([c[0], c[1], c[2]]) - px[0]) * a;
            return;
        }
        let ba = if lock_alpha { 1.0 } else { px[3] };
        let backdrop = [px[0] * ba, px[1] * ba, px[2] * ba, ba];
        let m = composite(mode, backdrop, [c[0], c[1], c[2], 1.0], a, clamp, x, y);
        if m[3] > 0.0 {
            for k in 0..3 {
                out[k] = m[k] / m[3];
            }
        }
        out[3] = if lock_alpha { px[3] } else { m[3] };
    });
    set_part(doc, &part, raster);
    Ok(())
}

/// the eyedropper: the average straight colour of a `size` by `size` square around (x, y)
/// (1, 3, 5, 11, 31, 51 or 101 in photoshop), averaged on premultiplied values
pub fn sample(img: &ColorBuf, x: i32, y: i32, size: u32) -> [f32; 4] {
    let half = (size.max(1) / 2) as i32;
    let mut acc = [0f64; 4];
    let mut n = 0.0;
    for sy in y - half..=y + half {
        for sx in x - half..=x + half {
            if !img.rect.contains(sx, sy) {
                continue;
            }
            let c = img.get(sx, sy);
            for k in 0..3 {
                acc[k] += f64::from(c[k] * c[3]);
            }
            acc[3] += f64::from(c[3]);
            n += 1.0;
        }
    }
    if n == 0.0 || acc[3] <= 0.0 {
        return [0.0; 4];
    }
    [
        (acc[0] / acc[3]) as f32,
        (acc[1] / acc[3]) as f32,
        (acc[2] / acc[3]) as f32,
        (acc[3] / n) as f32,
    ]
}

/// the tiles a stroke touched, for tests and the benchmark
pub fn touched_tiles(rect: IRect) -> HashSet<(i32, i32)> {
    tiles_under(rect).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ImageDocument;

    fn doc() -> ImageDocument {
        ImageDocument::new("t", 200, 100, Depth::U8, Some([1.0, 1.0, 1.0, 1.0]))
    }

    fn line(x0: f32, x1: f32, y: f32) -> Vec<StrokePoint> {
        (0..=20)
            .map(|i| StrokePoint {
                x: x0 + (x1 - x0) * i as f32 / 20.0,
                y,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
            })
            .collect()
    }

    #[test]
    fn opacity_caps_a_stroke_however_often_it_passes() {
        let d = doc();
        let mut s = Stroke::new(
            &d,
            BrushSettings {
                opacity: 0.5,
                smoothing: 0.0,
                hardness: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        s.add(&line(20.0, 180.0, 50.0));
        s.add(&line(180.0, 20.0, 50.0));
        s.finish();
        // black at half opacity over white is mid grey, not darker
        let v = s.result().get(100, 50, 0);
        assert!((v - 0.5).abs() < 0.01, "{v}");
        // outside the brush nothing changed
        assert_eq!(s.result().get(100, 10, 0), 1.0);
    }

    #[test]
    fn flow_builds_up_under_the_cap() {
        let d = doc();
        let settings = BrushSettings {
            flow: 0.2,
            smoothing: 0.0,
            hardness: 1.0,
            ..Default::default()
        };
        let mut once = Stroke::new(&d, settings.clone()).unwrap();
        once.add(&line(20.0, 180.0, 50.0));
        let mut twice = Stroke::new(&d, settings).unwrap();
        twice.add(&line(20.0, 180.0, 50.0));
        twice.add(&line(180.0, 20.0, 50.0));
        assert!(twice.result().get(100, 50, 0) < once.result().get(100, 50, 0));
    }

    #[test]
    fn pressure_changes_the_width() {
        let d = doc();
        let mut s = Stroke::new(
            &d,
            BrushSettings {
                smoothing: 0.0,
                size: 40.0,
                hardness: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        let pts: Vec<StrokePoint> = (0..=40)
            .map(|i| StrokePoint {
                x: 10.0 + i as f32 * 4.5,
                y: 50.0,
                pressure: i as f32 / 40.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
            })
            .collect();
        s.add(&pts);
        let thick = |x: i32| (0..100).filter(|y| s.result().get(x, *y, 0) < 0.5).count();
        assert!(thick(180) > thick(40) * 2, "{} {}", thick(180), thick(40));
    }

    #[test]
    fn eraser_clears_and_selection_limits_paint() {
        let mut d = doc();
        d.selection = crate::selection::combine(
            None,
            d.canvas(),
            IRect::new(0, 0, 100, 100),
            crate::selection::Combine::Replace,
            &|_, _| 1.0,
        );
        let mut s = Stroke::new(
            &d,
            BrushSettings {
                tool: BrushTool::Eraser,
                smoothing: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        s.add(&line(20.0, 180.0, 50.0));
        assert_eq!(s.result().get(50, 50, 3), 0.0);
        assert_eq!(s.result().get(150, 50, 3), 1.0);
    }

    #[test]
    fn gradients_place_pixels_along_the_line() {
        let k = GradientKind::Linear;
        assert_eq!(gradient_t(k, [0.0, 0.0], [10.0, 0.0], 5.0, 3.0), 0.5);
        assert_eq!(
            gradient_t(GradientKind::Radial, [0.0, 0.0], [10.0, 0.0], 0.0, 5.0),
            0.5
        );
        assert_eq!(
            gradient_t(GradientKind::Reflected, [5.0, 0.0], [10.0, 0.0], 0.0, 0.0),
            1.0
        );
        assert!(
            (gradient_t(GradientKind::Angle, [0.0, 0.0], [1.0, 0.0], 0.0, 1.0) - 0.25).abs() < 1e-6
        );
        assert_eq!(
            gradient_t(GradientKind::Diamond, [0.0, 0.0], [10.0, 0.0], 5.0, 5.0),
            0.5
        );
        let mut d = doc();
        gradient(
            &mut d,
            [0.0, 0.0],
            [200.0, 0.0],
            &GradientSettings {
                dither: false,
                ..Default::default()
            },
        )
        .unwrap();
        let r = d.layers[0].raster().unwrap();
        assert!(r.get(0, 0, 0) < 0.01 && r.get(199, 0, 0) > 0.99);
        assert!((r.get(100, 0, 0) - 0.5).abs() < 0.01);
    }

    #[test]
    fn eyedropper_averages() {
        let img = ColorBuf {
            rect: IRect::of_size(3, 1),
            data: vec![[65535, 0, 0, 65535], [0, 0, 65535, 65535], [0, 0, 0, 0]],
        };
        let c = sample(&img, 1, 0, 3);
        assert!((c[0] - 0.5).abs() < 1e-3 && (c[2] - 0.5).abs() < 1e-3);
        assert!((c[3] - 2.0 / 3.0).abs() < 1e-3);
    }
}
