//! fill layers (solid colour, gradient, pattern) and the patterns they and pattern
//! overlays use. a fill covers the whole canvas and is drawn by the render engine, so it
//! never runs out at the edges when the canvas grows.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::graph::Node;
use vellum_gpu::nodes::style::{PatternNode, pattern_data};
use vellum_gpu::nodes::{GradientNode, SolidNode};
use vellum_gpu::params::GradientStop;
use vellum_gpu::{IRect, ParamValue};

use crate::paint::{ColorStop, GradientKind};

/// a small picture that repeats, straight srgb
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Pattern {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GradientFill {
    pub kind: GradientKind,
    pub stops: Vec<ColorStop>,
    /// degrees, 0 runs left to right
    pub angle: f32,
    /// percent of the canvas the gradient spans
    pub scale: f32,
    pub reverse: bool,
    /// moves the centre, in document pixels
    #[serde(default)]
    pub offset: [f32; 2],
}

impl Default for GradientFill {
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
            angle: 90.0,
            scale: 100.0,
            reverse: false,
            offset: [0.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "type")]
#[ts(export)]
pub enum FillLayer {
    Solid { color: [f32; 4] },
    Gradient { gradient: GradientFill },
    Pattern { pattern: Pattern, scale: f32 },
}

impl FillLayer {
    pub fn title(&self) -> &'static str {
        match self {
            FillLayer::Solid { .. } => "Color Fill",
            FillLayer::Gradient { .. } => "Gradient Fill",
            FillLayer::Pattern { .. } => "Pattern Fill",
        }
    }
}

/// the gradient node for a gradient filling `area`
pub fn gradient_node(g: &GradientFill, area: IRect) -> Node {
    let c = [
        area.x as f32 + area.w as f32 / 2.0 + g.offset[0],
        area.y as f32 + area.h as f32 / 2.0 + g.offset[1],
    ];
    let (s, co) = g.angle.to_radians().sin_cos();
    // photoshop's angle turns anticlockwise with y pointing up
    let dir = [co, -s];
    let (w, h) = (area.w as f32, area.h as f32);
    let k = g.scale.max(1.0) / 100.0;
    let shape = match g.kind {
        GradientKind::Linear => "linear",
        GradientKind::Radial => "radial",
        GradientKind::Angle => "angle",
        GradientKind::Reflected => "reflected",
        GradientKind::Diamond => "diamond",
    };
    let (start, end) = match g.kind {
        GradientKind::Linear => {
            let half = 0.5 * k * (w * co.abs() + h * s.abs());
            (
                [c[0] - dir[0] * half, c[1] - dir[1] * half],
                [c[0] + dir[0] * half, c[1] + dir[1] * half],
            )
        }
        _ => {
            let r = 0.5 * k * w.hypot(h);
            (c, [c[0] + dir[0] * r, c[1] + dir[1] * r])
        }
    };
    let stops: Vec<GradientStop> = g
        .stops
        .iter()
        .map(|s| GradientStop {
            offset: s.at,
            color: s.color,
        })
        .collect();
    Node::of(GradientNode)
        .param("shape", shape)
        .param("start", start)
        .param("end", end)
        .param("stops", ParamValue::Gradient(stops))
        .param("reverse", g.reverse)
}

pub fn pattern_node(p: &Pattern, scale: f32, offset: [f32; 2]) -> Node {
    Node::of(PatternNode)
        .param("scale", scale)
        .param("offset", offset)
        .data(pattern_data(p.width, p.height, &p.pixels))
}

/// the node that draws a fill layer over the canvas
pub fn node(fill: &FillLayer, canvas: IRect) -> Node {
    match fill {
        FillLayer::Solid { color } => Node::of(SolidNode).param("color", *color),
        FillLayer::Gradient { gradient } => gradient_node(gradient, canvas),
        FillLayer::Pattern { pattern, scale } => pattern_node(pattern, *scale, [0.0, 0.0]),
    }
}

fn make(name: &str, w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> Pattern {
    Pattern {
        name: name.into(),
        width: w,
        height: h,
        pixels: (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| f(x, y))
            .collect(),
    }
}

/// the patterns vellum ships
pub fn patterns() -> Vec<Pattern> {
    let ink = [0.15, 0.15, 0.17, 1.0];
    let paper = [0.95, 0.94, 0.9, 1.0];
    vec![
        make("Checkerboard", 16, 16, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 { ink } else { paper }
        }),
        make("Dots", 12, 12, |x, y| {
            let d = (x as f32 - 5.5).hypot(y as f32 - 5.5);
            if d < 3.5 { ink } else { paper }
        }),
        make("Stripes", 12, 12, |x, y| {
            if (x + y) % 12 < 4 { ink } else { paper }
        }),
        make(
            "Grid",
            16,
            16,
            |x, y| {
                if x == 0 || y == 0 { ink } else { paper }
            },
        ),
        make("Bricks", 32, 16, |x, y| {
            let row = y / 8;
            let shift = if row % 2 == 0 { 0 } else { 16 };
            let mortar = y % 8 == 0 || (x + shift) % 32 == 0;
            if mortar {
                [0.85, 0.83, 0.78, 1.0]
            } else {
                [0.66, 0.3, 0.22, 1.0]
            }
        }),
        make("Paper grain", 32, 32, |x, y| {
            let n = vellum_gpu::blend::pixel_noise(x as i32, y as i32) * 0.08;
            [0.9 - n, 0.89 - n, 0.85 - n, 1.0]
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_gpu::Engine;
    use vellum_gpu::engine::RenderRequest;
    use vellum_gpu::graph::Graph;

    #[test]
    fn fills_cover_the_canvas() {
        let canvas = IRect::of_size(40, 20);
        let engine = Engine::cpu_only();
        for fill in [
            FillLayer::Solid {
                color: [1.0, 0.0, 0.0, 1.0],
            },
            FillLayer::Gradient {
                gradient: GradientFill::default(),
            },
            FillLayer::Pattern {
                pattern: patterns().remove(0),
                scale: 100.0,
            },
        ] {
            let mut g = Graph::new();
            let n = g.add(node(&fill, canvas));
            let img = engine
                .render_image(&g, &RenderRequest::new(n, canvas))
                .expect("render");
            assert!(img.data.iter().all(|p| p[3] > 0.99), "{}", fill.title());
        }
    }

    #[test]
    fn an_upward_gradient_is_dark_at_the_bottom() {
        let canvas = IRect::of_size(10, 100);
        let mut g = Graph::new();
        let n = g.add(gradient_node(&GradientFill::default(), canvas));
        let img = Engine::cpu_only()
            .render_image(&g, &RenderRequest::new(n, canvas))
            .expect("render");
        assert!(img.get(5, 95)[0] < 0.1 && img.get(5, 4)[0] > 0.9);
    }
}
