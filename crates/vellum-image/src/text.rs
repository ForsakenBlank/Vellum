//! text layers: point text, paragraph text in a box, and text along a path, with warps,
//! opentype features and variable font axes.
//!
//! text is shaped by rustybuzz (the harfbuzz algorithms in rust), so ligatures, kerning,
//! small caps and stylistic sets work the way they do in other apps. glyph outlines come
//! from ttf-parser with the font's variation axes applied, are placed by the layout, bent
//! by the warp if there is one, and filled by the same anti aliased filler as shapes. the
//! warp moves outline points rather than pixels, so warped text stays sharp.
//!
//! a text layer keeps the pixels it was last drawn with. when its font is missing on this
//! machine the pixels stay as they were until the font is replaced or downloaded.

use std::collections::BTreeMap;

use rustybuzz::ttf_parser::{GlyphId, OutlineBuilder, Tag};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::IRect;

use crate::fonts::{self, FontRef};
use crate::raster::{Depth, Raster};
use crate::vector::{Affine, FillRule, VectorPath, apply, fill};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    /// spread full lines to both edges (paragraph text)
    Justify,
}

/// where the text sits
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "type")]
#[ts(export)]
pub enum TextFrame {
    /// one line per paragraph, from a baseline point
    Point { x: f32, y: f32 },
    /// lines wrap inside a box
    Box { x: f32, y: f32, w: f32, h: f32 },
    /// along a path, starting `offset` pixels along it
    Path { path: VectorPath, offset: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WarpStyle {
    #[default]
    Arc,
    Arch,
    Bulge,
    Flag,
    Wave,
    Fish,
    Rise,
    Squeeze,
    Inflate,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextWarp {
    pub style: WarpStyle,
    /// -100 to 100
    pub bend: f32,
    /// -100 to 100
    #[serde(default)]
    pub horizontal: f32,
    /// -100 to 100
    #[serde(default)]
    pub vertical: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextLayer {
    pub text: String,
    pub font: FontRef,
    /// pixels
    pub size: f32,
    /// straight srgb
    pub color: [f32; 4],
    /// thousandths of an em added between letters
    #[serde(default)]
    pub tracking: f32,
    /// pixels between baselines; none is 1.2 times the size
    #[serde(default)]
    pub leading: Option<f32>,
    #[serde(default)]
    pub align: TextAlign,
    pub frame: TextFrame,
    /// opentype features switched on or off, eg `liga` 0, `smcp` 1, `ss01` 1
    #[serde(default)]
    pub features: BTreeMap<String, u32>,
    /// variable font axes, eg `wght` 650
    #[serde(default)]
    pub variations: BTreeMap<String, f32>,
    #[serde(default)]
    pub warp: Option<TextWarp>,
    /// applied after layout, so text can be turned and skewed and stay text
    #[serde(default = "identity")]
    pub transform: Affine,
    /// set when the font was not found the last time the text was drawn
    #[serde(default)]
    pub missing_font: bool,
}

fn identity() -> Affine {
    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
}

impl TextLayer {
    pub fn point(text: impl Into<String>, x: f32, y: f32, size: f32) -> Self {
        Self {
            text: text.into(),
            font: FontRef::default(),
            size,
            color: [0.0, 0.0, 0.0, 1.0],
            tracking: 0.0,
            leading: None,
            align: TextAlign::Left,
            frame: TextFrame::Point { x, y },
            features: BTreeMap::new(),
            variations: BTreeMap::new(),
            warp: None,
            transform: identity(),
            missing_font: false,
        }
    }

    pub fn leading(&self) -> f32 {
        self.leading.unwrap_or(self.size * 1.2)
    }

    /// moves, scales, turns or skews the text after layout
    pub fn then(&mut self, m: &Affine) {
        let [a, b, c, d, e, f] = self.transform;
        let [ma, mb, mc, md, me, mf] = *m;
        self.transform = [
            ma * a + mc * b,
            mb * a + md * b,
            ma * c + mc * d,
            mb * c + md * d,
            ma * e + mc * f + me,
            mb * e + md * f + mf,
        ];
    }
}

fn tag(s: &str) -> Option<Tag> {
    let b = s.as_bytes();
    (b.len() == 4).then(|| Tag::from_bytes(&[b[0], b[1], b[2], b[3]]))
}

/// collects a glyph's outline as straight pieces in document space
struct Outline<'a> {
    contours: &'a mut Vec<Vec<[f64; 2]>>,
    current: Vec<[f64; 2]>,
    map: &'a dyn Fn(f32, f32) -> [f64; 2],
    last: [f32; 2],
    /// pieces per curve, from the glyph's size on screen
    steps: usize,
}

impl Outline<'_> {
    fn flush(&mut self) {
        if self.current.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }
}

impl OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.current.push((self.map)(x, y));
        self.last = [x, y];
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.current.push((self.map)(x, y));
        self.last = [x, y];
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last;
        for i in 1..=self.steps {
            let t = i as f32 / self.steps as f32;
            let u = 1.0 - t;
            let px = u * u * p0[0] + 2.0 * u * t * x1 + t * t * x;
            let py = u * u * p0[1] + 2.0 * u * t * y1 + t * t * y;
            self.current.push((self.map)(px, py));
        }
        self.last = [x, y];
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last;
        for i in 1..=self.steps {
            let t = i as f32 / self.steps as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            let px = a * p0[0] + b * x1 + c * x2 + d * x;
            let py = a * p0[1] + b * y1 + c * y2 + d * y;
            self.current.push((self.map)(px, py));
        }
        self.last = [x, y];
    }
    fn close(&mut self) {
        self.flush();
    }
}

/// one shaped glyph: id, x and y offset from the line's start, advance, and whether it
/// is a space (justified lines stretch at those)
type Placed = (u16, f32, f32, f32, bool);

/// a shaped line: glyphs and where each sits, in pixels from the line's start
struct Line {
    glyphs: Vec<Placed>,
    width: f32,
    spaces: usize,
}

struct Shaper<'a> {
    face: rustybuzz::Face<'a>,
    features: Vec<rustybuzz::Feature>,
    scale: f32,
    tracking: f32,
}

impl Shaper<'_> {
    fn line(&self, text: &str) -> Line {
        let mut buf = rustybuzz::UnicodeBuffer::new();
        buf.push_str(text);
        buf.guess_segment_properties();
        let out = rustybuzz::shape(&self.face, &self.features, buf);
        let mut pen = 0.0;
        let mut glyphs = Vec::with_capacity(out.len());
        let mut spaces = 0;
        for (info, pos) in out.glyph_infos().iter().zip(out.glyph_positions()) {
            let space = text
                .get(info.cluster as usize..)
                .and_then(|s| s.chars().next())
                .is_some_and(char::is_whitespace);
            glyphs.push((
                info.glyph_id as u16,
                pen + pos.x_offset as f32 * self.scale,
                pos.y_offset as f32 * self.scale,
                pos.x_advance as f32 * self.scale,
                space,
            ));
            pen += pos.x_advance as f32 * self.scale + self.tracking;
            spaces += usize::from(space);
        }
        Line {
            glyphs,
            width: (pen - self.tracking).max(0.0),
            spaces,
        }
    }

    /// greedy word wrap: as many words per line as fit in `width`
    fn wrap(&self, para: &str, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current = String::new();
        for word in para.split_inclusive(' ') {
            let trial = format!("{current}{word}");
            if !current.is_empty() && self.line(trial.trim_end()).width > width {
                lines.push(current.trim_end().to_owned());
                current = word.to_owned();
            } else {
                current = trial;
            }
        }
        lines.push(current.trim_end().to_owned());
        lines
    }
}

/// a point along a polyline at distance `s`, with the direction there
fn along(pts: &[[f64; 2]], s: f32) -> ([f32; 2], [f32; 2]) {
    let mut walked = 0.0f32;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).hypot(b[1] - a[1])) as f32;
        if walked + len >= s || len <= 0.0 {
            let t = if len > 0.0 { (s - walked) / len } else { 0.0 };
            let dir = if len > 0.0 {
                [(b[0] - a[0]) as f32 / len, (b[1] - a[1]) as f32 / len]
            } else {
                [1.0, 0.0]
            };
            return (
                [
                    a[0] as f32 + (b[0] - a[0]) as f32 * t,
                    a[1] as f32 + (b[1] - a[1]) as f32 * t,
                ],
                dir,
            );
        }
        walked += len;
    }
    let n = pts.len();
    if n >= 2 {
        let (a, b) = (pts[n - 2], pts[n - 1]);
        let len = ((b[0] - a[0]).hypot(b[1] - a[1]) as f32).max(1e-6);
        let dir = [(b[0] - a[0]) as f32 / len, (b[1] - a[1]) as f32 / len];
        let over = s - walked;
        return (
            [b[0] as f32 + dir[0] * over, b[1] as f32 + dir[1] * over],
            dir,
        );
    }
    ([0.0, 0.0], [1.0, 0.0])
}

/// bends a point of text in the box (cx, cy, half width, half height) by a warp
fn warp_point(w: &TextWarp, b: [f32; 4], p: [f64; 2]) -> [f64; 2] {
    let (cx, cy, hw, hh) = (b[0], b[1], b[2].max(1.0), b[3].max(1.0));
    let (mut x, mut y) = (p[0] as f32, p[1] as f32);
    let u = ((x - cx) / hw).clamp(-1.5, 1.5);
    let v = ((y - cy) / hh).clamp(-1.5, 1.5);
    let k = w.bend / 100.0;
    let pi = std::f32::consts::PI;
    // styles that bend the line curve by an amount that follows the text's width, like
    // a real arc, so a single long line bends as much as a block of text
    let amp = hh.max(hw * 0.3);
    match w.style {
        WarpStyle::Arc => y -= k * amp * (1.0 - u * u),
        WarpStyle::Arch => y -= k * amp * (1.0 - u * u) * (1.0 - v) * 0.75,
        WarpStyle::Bulge => y = cy + (y - cy) * (1.0 + k * (1.0 - u * u)),
        WarpStyle::Flag => y += k * amp * 0.5 * (pi * u).sin(),
        WarpStyle::Wave => y += k * amp * 0.5 * (pi * u + v).sin(),
        WarpStyle::Fish => y = cy + (y - cy) * (1.0 + k * (pi * u).cos() * 0.5),
        WarpStyle::Rise => y -= k * amp * u,
        WarpStyle::Squeeze => x = cx + (x - cx) * (1.0 - k * 0.5 * (1.0 - v * v)),
        WarpStyle::Inflate => {
            let s = 1.0 + k * 0.5 * (1.0 - u * u) * (1.0 - v * v);
            x = cx + (x - cx) * s;
            y = cy + (y - cy) * s;
        }
    }
    // perspective-like distortion: wider at one end, taller at one side
    let hd = w.horizontal / 100.0;
    let vd = w.vertical / 100.0;
    x = cx + (x - cx) * (1.0 + hd * v * 0.5);
    y = cy + (y - cy) * (1.0 + vd * u * 0.5);
    [f64::from(x), f64::from(y)]
}

/// what drawing a text layer gives: its pixels and whether its font was missing
pub struct Drawn {
    pub pixels: Raster,
    pub missing_font: bool,
    pub bounds: IRect,
}

/// the outlines of a text layer in document space, and whether its font was missing
pub fn outlines(t: &TextLayer) -> (Vec<Vec<[f64; 2]>>, bool) {
    let Some((face_data, missing)) = fonts::resolve(&t.font) else {
        return (Vec::new(), true);
    };
    let Some(mut face) = rustybuzz::Face::from_slice(&face_data.data, face_data.index) else {
        return (Vec::new(), true);
    };
    let vars: Vec<rustybuzz::Variation> = t
        .variations
        .iter()
        .filter_map(|(k, v)| tag(k).map(|tag| rustybuzz::Variation { tag, value: *v }))
        .collect();
    if !vars.is_empty() {
        face.set_variations(&vars);
    }
    let upem = (face.units_per_em() as f32).max(1.0);
    let size = t.size.max(0.5);
    let scale = size / upem;
    let features: Vec<rustybuzz::Feature> = t
        .features
        .iter()
        .filter_map(|(k, v)| tag(k).map(|tag| rustybuzz::Feature::new(tag, *v, ..)))
        .collect();
    let ascent = f32::from(face.ascender()) * scale;
    let shaper = Shaper {
        face,
        features,
        scale,
        tracking: t.tracking / 1000.0 * size,
    };
    let leading = t.leading();
    let steps = ((size / 8.0).ceil() as usize).clamp(3, 24);
    let mut contours: Vec<Vec<[f64; 2]>> = Vec::new();
    // every glyph goes through one placing function: from glyph units to document pixels
    let mut place = |gid: u16, map: &dyn Fn(f32, f32) -> [f64; 2]| {
        let mut o = Outline {
            contours: &mut contours,
            current: Vec::new(),
            map,
            last: [0.0, 0.0],
            steps,
        };
        shaper.face.outline_glyph(GlyphId(gid), &mut o);
        o.flush();
    };
    match &t.frame {
        TextFrame::Point { x, y } | TextFrame::Box { x, y, .. } => {
            let (bx, width, top) = match &t.frame {
                TextFrame::Box { w, .. } => (*x, Some(*w), *y + ascent),
                _ => (*x, None, *y),
            };
            let mut lines: Vec<(String, bool)> = Vec::new();
            for para in t.text.split('\n') {
                match width {
                    Some(w) => {
                        let wrapped = shaper.wrap(para, w);
                        let n = wrapped.len();
                        for (i, l) in wrapped.into_iter().enumerate() {
                            lines.push((l, i + 1 == n));
                        }
                    }
                    None => lines.push((para.to_owned(), true)),
                }
            }
            for (i, (text, last)) in lines.iter().enumerate() {
                let line = shaper.line(text);
                let baseline = top + i as f32 * leading;
                let box_w = width.unwrap_or(0.0);
                let mut extra = 0.0;
                let start = match t.align {
                    TextAlign::Left => bx,
                    TextAlign::Center if width.is_some() => bx + (box_w - line.width) / 2.0,
                    TextAlign::Center => bx - line.width / 2.0,
                    TextAlign::Right if width.is_some() => bx + box_w - line.width,
                    TextAlign::Right => bx - line.width,
                    TextAlign::Justify => {
                        if width.is_some() && !last && line.spaces > 0 {
                            extra = (box_w - line.width) / line.spaces as f32;
                        }
                        bx
                    }
                };
                let mut shift = 0.0;
                for (gid, gx, gy, _, space) in &line.glyphs {
                    let ox = start + gx + shift;
                    let oy = baseline - gy;
                    place(*gid, &|ux, uy| {
                        [f64::from(ox + ux * scale), f64::from(oy - uy * scale)]
                    });
                    if *space {
                        shift += extra;
                    }
                }
            }
        }
        TextFrame::Path { path, offset } => {
            let line = shaper.line(&t.text.replace('\n', " "));
            let pts: Vec<[f64; 2]> = path
                .subpaths
                .first()
                .map(|s| s.flatten())
                .unwrap_or_default();
            if pts.len() >= 2 {
                let total: f32 = pts
                    .windows(2)
                    .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) as f32)
                    .sum();
                let start = match t.align {
                    TextAlign::Center => offset + (total - line.width) / 2.0,
                    TextAlign::Right => offset + total - line.width,
                    _ => *offset,
                };
                for (gid, gx, gy, adv, _) in &line.glyphs {
                    let mid = start + gx + adv / 2.0;
                    let (at, dir) = along(&pts, mid);
                    let half = adv / 2.0;
                    place(*gid, &|ux, uy| {
                        let lx = ux * scale - half;
                        let ly = -(uy * scale) - gy;
                        [
                            f64::from(at[0] + dir[0] * lx - dir[1] * ly),
                            f64::from(at[1] + dir[1] * lx + dir[0] * ly),
                        ]
                    });
                }
            }
        }
    }
    if let Some(w) = &t.warp
        && (w.bend != 0.0 || w.horizontal != 0.0 || w.vertical != 0.0)
    {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in contours.iter().flatten() {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        if x0 < x1 {
            let b = [
                ((x0 + x1) / 2.0) as f32,
                ((y0 + y1) / 2.0) as f32,
                ((x1 - x0) / 2.0) as f32,
                ((y1 - y0) / 2.0) as f32,
            ];
            for p in contours.iter_mut().flatten() {
                *p = warp_point(w, b, *p);
            }
        }
    }
    if t.transform != identity() {
        for p in contours.iter_mut().flatten() {
            let q = apply(&t.transform, [p[0] as f32, p[1] as f32]);
            *p = [f64::from(q[0]), f64::from(q[1])];
        }
    }
    (contours, missing)
}

/// draws a text layer into straight colour. colours go through `to_doc`.
pub fn draw(
    t: &TextLayer,
    depth: Depth,
    clip: IRect,
    to_doc: &dyn Fn([f32; 4]) -> [f32; 4],
) -> Drawn {
    let (contours, missing) = outlines(t);
    let cover = fill(&contours, FillRule::NonZero, clip);
    let bounds = cover.content_bounds();
    let c = to_doc(t.color);
    let mut pixels = Raster::rgba(depth);
    pixels.update(bounds, |x, y, px| {
        let a = cover.get(x, y, 0) * c[3];
        if a > 0.0 {
            px.copy_from_slice(&[c[0], c[1], c[2], a]);
        }
    });
    Drawn {
        pixels,
        missing_font: missing,
        bounds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_fonts() -> bool {
        fonts::resolve(&FontRef::default()).is_some()
    }

    fn ink(r: &Raster) -> f32 {
        let b = r.content_bounds();
        let mut s = 0.0;
        for y in b.y..b.bottom() {
            for x in b.x..b.right() {
                s += r.get(x, y, 3);
            }
        }
        s
    }

    fn big() -> IRect {
        IRect::new(-500, -500, 3000, 3000)
    }

    #[test]
    fn point_text_draws_on_its_baseline() {
        if !have_fonts() {
            return;
        }
        let t = TextLayer::point("Hello", 10.0, 50.0, 32.0);
        let d = draw(&t, Depth::U8, big(), &|c| c);
        assert!(!d.missing_font);
        assert!(d.bounds.bottom() <= 52 && d.bounds.y > 10, "{:?}", d.bounds);
        assert!(d.bounds.x >= 10 && d.bounds.w > 40);
        let mut wide = t.clone();
        wide.tracking = 200.0;
        assert!(draw(&wide, Depth::U8, big(), &|c| c).bounds.w > d.bounds.w);
    }

    #[test]
    fn paragraphs_wrap_and_align() {
        if !have_fonts() {
            return;
        }
        let mut t = TextLayer::point("one two three four five six seven eight", 0.0, 0.0, 20.0);
        t.frame = TextFrame::Box {
            x: 0.0,
            y: 0.0,
            w: 120.0,
            h: 400.0,
        };
        let d = draw(&t, Depth::U8, big(), &|c| c);
        assert!(d.bounds.right() <= 122, "{:?}", d.bounds);
        assert!(d.bounds.h > 60, "wrapped onto several lines");
        t.align = TextAlign::Right;
        let r = draw(&t, Depth::U8, big(), &|c| c);
        assert!(r.bounds.right() >= 115);
    }

    #[test]
    fn warps_paths_and_missing_fonts() {
        if !have_fonts() {
            return;
        }
        let mut t = TextLayer::point("Warped", 0.0, 100.0, 40.0);
        let plain = draw(&t, Depth::U8, big(), &|c| c);
        t.warp = Some(TextWarp {
            style: WarpStyle::Arc,
            bend: 50.0,
            horizontal: 0.0,
            vertical: 0.0,
        });
        let bent = draw(&t, Depth::U8, big(), &|c| c);
        // the middle of the line rises while the ends stay put
        assert!(
            bent.bounds.y + 4 < plain.bounds.y,
            "{:?} {:?}",
            plain.bounds,
            bent.bounds
        );
        assert_eq!(bent.bounds.x, plain.bounds.x);
        let mut on_path = t.clone();
        on_path.warp = None;
        on_path.frame = TextFrame::Path {
            path: crate::vector::Geometry::Ellipse {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h: 300.0,
            }
            .to_path(),
            offset: 0.0,
        };
        let p = draw(&on_path, Depth::U8, big(), &|c| c);
        assert!(ink(&p.pixels) > ink(&plain.pixels) * 0.5);
        let mut gone = TextLayer::point("Hi", 0.0, 40.0, 30.0);
        gone.font.family = "No Such Family Anywhere".into();
        assert!(draw(&gone, Depth::U8, big(), &|c| c).missing_font);
    }

    #[test]
    fn features_and_variations_are_accepted() {
        if !have_fonts() {
            return;
        }
        let mut t = TextLayer::point("office affine", 0.0, 40.0, 30.0);
        t.features.insert("liga".into(), 0);
        t.variations.insert("wght".into(), 700.0);
        t.features.insert("bad tag".into(), 1);
        let d = draw(&t, Depth::U8, big(), &|c| c);
        assert!(ink(&d.pixels) > 0.0);
    }
}
