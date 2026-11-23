//! smart objects and the filter gallery.
//!
//! a smart object keeps its pixels as they were placed, in their own coordinates, with a
//! transform that puts them in the document. scaling one down and up again loses nothing,
//! and its filters (smart filters) stay settings that can be changed, hidden, reordered,
//! faded and masked at any time: the render graph runs them every time.
//!
//! linked smart objects also remember the file they came from, so they can be brought up
//! to date when that file changes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::graph::Node;
use vellum_gpu::nodes::blend::mode_param;
use vellum_gpu::{BlendMode, IRect, NodeRegistry, ParamValue};

use crate::raster::Raster;

/// a filter on a smart object
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SmartFilter {
    pub id: u32,
    /// the gallery entry it came from, for its name
    pub entry: String,
    /// the render node
    pub kind: String,
    #[serde(default)]
    pub params: BTreeMap<String, ParamValue>,
    pub visible: bool,
    /// 0 to 1
    pub opacity: f32,
    pub blend: BlendMode,
}

#[derive(Debug, Clone)]
pub struct SmartObject {
    /// the placed pixels in their own coordinates, top left at 0, 0
    pub source: Raster,
    pub width: u32,
    pub height: u32,
    /// where the pixels land: x' = a x + c y + e, y' = b x + d y + f
    pub transform: [f32; 6],
    /// the file a linked smart object follows
    pub linked: Option<String>,
    pub filters: Vec<SmartFilter>,
    pub filters_visible: bool,
    /// grey, in document coordinates; white shows the filters
    pub filter_mask: Option<Raster>,
    pub next_filter: u32,
}

impl SmartObject {
    pub fn new(source: Raster, width: u32, height: u32, at: [f32; 2]) -> Self {
        Self {
            source,
            width,
            height,
            transform: [1.0, 0.0, 0.0, 1.0, at[0], at[1]],
            linked: None,
            filters: Vec::new(),
            filters_visible: true,
            filter_mask: None,
            next_filter: 1,
        }
    }

    /// the transform as the render engine's 3x3 matrix, rows first
    pub fn matrix(&self) -> [f32; 9] {
        let [a, b, c, d, e, f] = self.transform;
        [a, c, e, b, d, f, 0.0, 0.0, 1.0]
    }

    /// where the object lands in the document
    pub fn bounds(&self) -> IRect {
        let [a, b, c, d, e, f] = self.transform;
        let (w, h) = (self.width as f32, self.height as f32);
        let pts = [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]]
            .map(|[x, y]| [a * x + c * y + e, b * x + d * y + f]);
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        IRect::from_corners(
            x0.floor() as i32,
            y0.floor() as i32,
            x1.ceil() as i32,
            y1.ceil() as i32,
        )
    }

    /// `m` (rows first, 2x3) after the current transform
    pub fn then(&mut self, m: [f32; 6]) {
        let [a, b, c, d, e, f] = self.transform;
        let [ma, mb, mc, md, me, mf] = m;
        self.transform = [
            ma * a + mc * b,
            mb * a + md * b,
            ma * c + mc * d,
            mb * c + md * d,
            ma * e + mc * f + me,
            mb * e + md * f + mf,
        ];
    }

    pub fn is_identity_placement(&self) -> bool {
        let [a, b, c, d, e, f] = self.transform;
        a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 && e.fract() == 0.0 && f.fract() == 0.0
    }
}

/// a filter in the gallery: a render node with settings that make it do something
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FilterEntry {
    /// stable id, eg `blur.motion`
    pub id: String,
    pub title: String,
    pub category: String,
    /// the render node
    pub kind: String,
    pub params: BTreeMap<String, ParamValue>,
}

fn entry(
    id: &str,
    title: &str,
    category: &str,
    kind: &str,
    params: &[(&str, ParamValue)],
) -> FilterEntry {
    FilterEntry {
        id: id.into(),
        title: title.into(),
        category: category.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    }
}

/// every filter in the gallery, by category in photoshop's order
pub fn gallery() -> Vec<FilterEntry> {
    let f = ParamValue::Float;
    let c = |s: &str| ParamValue::Choice(s.into());
    vec![
        entry(
            "blur.gaussian",
            "Gaussian Blur",
            "Blur",
            "effect.gaussian_blur",
            &[],
        ),
        entry(
            "blur.motion",
            "Motion Blur",
            "Blur",
            "effect.motion_blur",
            &[],
        ),
        entry(
            "blur.radial",
            "Radial Blur",
            "Blur",
            "effect.radial_blur",
            &[("mode", c("zoom"))],
        ),
        entry("blur.lens", "Lens Blur", "Blur", "effect.lens_blur", &[]),
        entry(
            "blur.surface",
            "Surface Blur",
            "Blur",
            "effect.surface_blur",
            &[],
        ),
        entry(
            "gallery.field",
            "Field Blur",
            "Blur Gallery",
            "effect.focus_blur",
            &[
                ("mode", c("field")),
                ("blur", f(0.0)),
                ("end_blur", f(15.0)),
            ],
        ),
        entry(
            "gallery.iris",
            "Iris Blur",
            "Blur Gallery",
            "effect.focus_blur",
            &[("mode", c("iris"))],
        ),
        entry(
            "gallery.tilt_shift",
            "Tilt-Shift",
            "Blur Gallery",
            "effect.focus_blur",
            &[],
        ),
        entry(
            "gallery.path",
            "Path Blur",
            "Blur Gallery",
            "effect.motion_blur",
            &[("distance", f(25.0))],
        ),
        entry(
            "gallery.spin",
            "Spin Blur",
            "Blur Gallery",
            "effect.radial_blur",
            &[("mode", c("spin")), ("amount", f(15.0))],
        ),
        entry(
            "sharpen.unsharp",
            "Unsharp Mask",
            "Sharpen",
            "effect.sharpen",
            &[],
        ),
        entry(
            "sharpen.smart",
            "Smart Sharpen",
            "Sharpen",
            "effect.smart_sharpen",
            &[],
        ),
        entry(
            "sharpen.high_pass",
            "High Pass",
            "Other",
            "effect.high_pass",
            &[],
        ),
        entry(
            "noise.reduce",
            "Reduce Noise",
            "Noise",
            "effect.reduce_noise",
            &[],
        ),
        entry("noise.add", "Add Noise", "Noise", "effect.add_noise", &[]),
        entry(
            "noise.film_grain",
            "Film Grain",
            "Noise",
            "effect.film_grain",
            &[],
        ),
        entry("distort.wave", "Wave", "Distort", "effect.wave", &[]),
        entry("distort.twirl", "Twirl", "Distort", "effect.twirl", &[]),
        entry(
            "distort.polar",
            "Polar Coordinates",
            "Distort",
            "effect.polar",
            &[],
        ),
        entry("distort.ripple", "Ripple", "Distort", "effect.ripple", &[]),
        entry(
            "distort.displace",
            "Displace",
            "Distort",
            "effect.displace",
            &[],
        ),
        entry(
            "distort.chromatic",
            "Chromatic Aberration",
            "Distort",
            "effect.chromatic_aberration",
            &[],
        ),
        entry("distort.glitch", "Glitch", "Distort", "effect.glitch", &[]),
        entry("stylize.emboss", "Emboss", "Stylize", "effect.emboss", &[]),
        entry(
            "stylize.find_edges",
            "Find Edges",
            "Stylize",
            "effect.find_edges",
            &[],
        ),
        entry(
            "stylize.oil_paint",
            "Oil Paint",
            "Stylize",
            "effect.oil_paint",
            &[],
        ),
        entry(
            "stylize.glowing_edges",
            "Glowing Edges",
            "Stylize",
            "effect.glowing_edges",
            &[],
        ),
        entry("render.clouds", "Clouds", "Render", "effect.clouds", &[]),
        entry(
            "render.lens_flare",
            "Lens Flare",
            "Render",
            "effect.lens_flare",
            &[],
        ),
        entry(
            "render.lighting",
            "Lighting Effects",
            "Render",
            "effect.lighting",
            &[],
        ),
        entry(
            "pixelate.mosaic",
            "Mosaic",
            "Pixelate",
            "effect.mosaic",
            &[],
        ),
        entry(
            "pixelate.halftone",
            "Color Halftone",
            "Pixelate",
            "effect.halftone",
            &[],
        ),
    ]
}

pub fn gallery_entry(id: &str) -> Option<FilterEntry> {
    gallery().into_iter().find(|e| e.id == id)
}

/// a filter's render node, with points that were left at the origin moved to the middle of
/// `area` (the layer or canvas) so a filter applied with its defaults lands sensibly
pub fn filter_node(
    kind: &str,
    params: &BTreeMap<String, ParamValue>,
    area: IRect,
    canvas: IRect,
    registry: &NodeRegistry,
) -> Option<Node> {
    let mut node = registry.create(kind)?;
    let mid = [
        area.x as f32 + area.w as f32 / 2.0,
        area.y as f32 + area.h as f32 / 2.0,
    ];
    for spec in node.kind.info().params {
        if spec.hidden {
            continue;
        }
        if matches!(spec.id, "center" | "position") && !params.contains_key(spec.id) {
            node.params.insert(spec.id.into(), ParamValue::Point(mid));
        }
        if spec.id == "end" && !params.contains_key("end") {
            node.params.insert(
                "end".into(),
                ParamValue::Point([mid[0], area.bottom() as f32]),
            );
        }
    }
    for (k, v) in params {
        node.params.insert(k.clone(), v.clone());
    }
    crate::adjustments::centre_on(&mut node, canvas);
    Some(node)
}

/// how a smart filter lands on what it filtered: through a blend when faded or blended
pub fn blend_filter(
    g: &mut vellum_gpu::Graph,
    before: vellum_gpu::NodeId,
    after: vellum_gpu::NodeId,
    f: &SmartFilter,
) -> vellum_gpu::NodeId {
    if f.opacity >= 1.0 && f.blend == BlendMode::Normal {
        return after;
    }
    if f.blend == BlendMode::Normal {
        return g.add(
            Node::of(vellum_gpu::nodes::style::MixNode)
                .param("amount", f.opacity.clamp(0.0, 1.0) * 100.0)
                .input(0, before)
                .input(1, after),
        );
    }
    g.add(
        Node::of(vellum_gpu::nodes::BlendNode)
            .param("mode", mode_param(f.blend))
            .param("opacity", f.opacity.clamp(0.0, 1.0) * 100.0)
            .input(0, before)
            .input(1, after),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gallery_filter_is_registered_with_real_settings() {
        let reg = NodeRegistry::with_builtins();
        let mut ids = std::collections::HashSet::new();
        for e in gallery() {
            assert!(ids.insert(e.id.clone()), "{} twice", e.id);
            let info = reg
                .get(&e.kind)
                .unwrap_or_else(|| panic!("{}", e.kind))
                .info();
            for k in e.params.keys() {
                assert!(info.params.iter().any(|p| p.id == k), "{} {k}", e.id);
            }
        }
    }

    #[test]
    fn transforms_compose_and_bound() {
        let mut s = SmartObject::new(
            Raster::rgba(crate::raster::Depth::U8),
            100,
            50,
            [10.0, 20.0],
        );
        assert_eq!(s.bounds(), IRect::new(10, 20, 100, 50));
        s.then([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        assert_eq!(s.bounds(), IRect::new(20, 40, 200, 100));
        assert!(!s.is_identity_placement());
    }
}
