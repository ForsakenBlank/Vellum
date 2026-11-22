//! layer styles: drop shadow, inner shadow, outer and inner glow, bevel and emboss, satin,
//! colour, gradient and pattern overlays, and stroke.
//!
//! a style is settings on a layer, drawn by the render engine from the layer's shape every
//! time, so it follows painting, moving and transforming. effects outside the layer (drop
//! shadow, outer glow) blend straight onto what is below with their own modes. the layer's
//! pixels, faded by its fill, then get the effects inside it in photoshop's order, and that
//! lands with the layer's own blend mode and opacity. fill fades only the pixels, which is
//! how a style can show on a layer whose fill is zero.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use vellum_gpu::graph::{Graph, Node, NodeId};
use vellum_gpu::nodes::blend::mode_param;
use vellum_gpu::nodes::{self, GaussianBlur};
use vellum_gpu::{BlendMode, IRect};

use crate::fills::{GradientFill, Pattern, gradient_node, pattern_node};

/// what every effect has: on or off, how it blends and how strong it is
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Blend {
    pub enabled: bool,
    pub mode: BlendMode,
    /// 0 to 1
    pub opacity: f32,
}

impl Blend {
    pub const fn new(mode: BlendMode, opacity: f32) -> Self {
        Self {
            enabled: true,
            mode,
            opacity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Shadow {
    pub blend: Blend,
    pub color: [f32; 4],
    /// degrees, where the light comes from
    pub angle: f32,
    pub distance: f32,
    /// percent the edge is hardened before blurring (choke for inner shadows)
    pub spread: f32,
    pub size: f32,
    /// the layer hides its own drop shadow where it is see through, photoshop's default
    #[serde(default = "yes")]
    pub knockout: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Glow {
    pub blend: Blend,
    pub color: [f32; 4],
    pub spread: f32,
    pub size: f32,
    /// inner glows only: glow from the middle instead of the edge
    #[serde(default)]
    pub from_center: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Bevel {
    pub highlight: Blend,
    pub highlight_color: [f32; 4],
    pub shadow: Blend,
    pub shadow_color: [f32; 4],
    /// percent
    pub depth: f32,
    pub size: f32,
    pub soften: f32,
    pub angle: f32,
    pub altitude: f32,
    /// pressed in rather than raised
    pub down: bool,
    /// emboss raises the layer from its surroundings, not only inside its edge
    #[serde(default)]
    pub emboss: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Satin {
    pub blend: Blend,
    pub color: [f32; 4],
    pub angle: f32,
    pub distance: f32,
    pub size: f32,
    pub invert: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColorOverlay {
    pub blend: Blend,
    pub color: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GradientOverlay {
    pub blend: Blend,
    pub gradient: GradientFill,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PatternOverlay {
    pub blend: Blend,
    pub pattern: Pattern,
    pub scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StrokePosition {
    #[default]
    Outside,
    Inside,
    Center,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StrokeEffect {
    pub blend: Blend,
    pub color: [f32; 4],
    pub size: f32,
    pub position: StrokePosition,
}

/// a layer's style. effects are kept while switched off, like photoshop's.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct LayerStyle {
    /// hides every effect at once
    pub hidden: bool,
    pub drop_shadow: Option<Shadow>,
    pub inner_shadow: Option<Shadow>,
    pub outer_glow: Option<Glow>,
    pub inner_glow: Option<Glow>,
    pub bevel: Option<Bevel>,
    pub satin: Option<Satin>,
    pub color_overlay: Option<ColorOverlay>,
    pub gradient_overlay: Option<GradientOverlay>,
    pub pattern_overlay: Option<PatternOverlay>,
    pub stroke: Option<StrokeEffect>,
}

fn on<T>(e: &Option<T>, blend: impl Fn(&T) -> &Blend) -> Option<&T> {
    e.as_ref()
        .filter(|v| blend(v).enabled && blend(v).opacity > 0.0)
}

impl LayerStyle {
    /// true when any effect would draw
    pub fn shows(&self) -> bool {
        !self.hidden
            && (on(&self.drop_shadow, |e| &e.blend).is_some()
                || on(&self.inner_shadow, |e| &e.blend).is_some()
                || on(&self.outer_glow, |e| &e.blend).is_some()
                || on(&self.inner_glow, |e| &e.blend).is_some()
                || self
                    .bevel
                    .as_ref()
                    .is_some_and(|b| b.highlight.enabled || b.shadow.enabled)
                || on(&self.satin, |e| &e.blend).is_some()
                || on(&self.color_overlay, |e| &e.blend).is_some()
                || on(&self.gradient_overlay, |e| &e.blend).is_some()
                || on(&self.pattern_overlay, |e| &e.blend).is_some()
                || on(&self.stroke, |e| &e.blend).is_some())
    }
}

/// the default settings of each effect, as photoshop starts them
pub fn default_shadow() -> Shadow {
    Shadow {
        blend: Blend::new(BlendMode::Multiply, 0.75),
        color: [0.0, 0.0, 0.0, 1.0],
        angle: 120.0,
        distance: 5.0,
        spread: 0.0,
        size: 5.0,
        knockout: true,
    }
}

pub fn default_glow() -> Glow {
    Glow {
        blend: Blend::new(BlendMode::Screen, 0.75),
        color: [1.0, 1.0, 0.745, 1.0],
        spread: 0.0,
        size: 5.0,
        from_center: false,
    }
}

pub fn default_bevel() -> Bevel {
    Bevel {
        highlight: Blend::new(BlendMode::Screen, 0.75),
        highlight_color: [1.0, 1.0, 1.0, 1.0],
        shadow: Blend::new(BlendMode::Multiply, 0.75),
        shadow_color: [0.0, 0.0, 0.0, 1.0],
        depth: 100.0,
        size: 5.0,
        soften: 0.0,
        angle: 120.0,
        altitude: 30.0,
        down: false,
        emboss: false,
    }
}

pub fn default_stroke() -> StrokeEffect {
    StrokeEffect {
        blend: Blend::new(BlendMode::Normal, 1.0),
        color: [0.0, 0.0, 0.0, 1.0],
        size: 3.0,
        position: StrokePosition::Outside,
    }
}

/// named styles for the styles panel
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StylePreset {
    pub name: String,
    pub style: LayerStyle,
}

pub fn presets() -> Vec<StylePreset> {
    let shadow = default_shadow();
    let p = |name: &str, style: LayerStyle| StylePreset {
        name: name.into(),
        style,
    };
    vec![
        p(
            "Soft shadow",
            LayerStyle {
                drop_shadow: Some(Shadow {
                    blend: Blend::new(BlendMode::Multiply, 0.45),
                    distance: 8.0,
                    size: 18.0,
                    ..shadow.clone()
                }),
                ..Default::default()
            },
        ),
        p(
            "Hard shadow",
            LayerStyle {
                drop_shadow: Some(Shadow {
                    blend: Blend::new(BlendMode::Normal, 1.0),
                    distance: 6.0,
                    size: 0.0,
                    ..shadow.clone()
                }),
                ..Default::default()
            },
        ),
        p(
            "Neon",
            LayerStyle {
                outer_glow: Some(Glow {
                    color: [0.1, 0.9, 1.0, 1.0],
                    size: 14.0,
                    spread: 10.0,
                    ..default_glow()
                }),
                inner_glow: Some(Glow {
                    color: [0.8, 1.0, 1.0, 1.0],
                    size: 4.0,
                    ..default_glow()
                }),
                color_overlay: Some(ColorOverlay {
                    blend: Blend::new(BlendMode::Normal, 1.0),
                    color: [0.2, 0.95, 1.0, 1.0],
                }),
                ..Default::default()
            },
        ),
        p(
            "Outline",
            LayerStyle {
                stroke: Some(default_stroke()),
                ..Default::default()
            },
        ),
        p(
            "Embossed",
            LayerStyle {
                bevel: Some(Bevel {
                    emboss: true,
                    ..default_bevel()
                }),
                ..Default::default()
            },
        ),
        p(
            "Gold",
            LayerStyle {
                gradient_overlay: Some(GradientOverlay {
                    blend: Blend::new(BlendMode::Normal, 1.0),
                    gradient: GradientFill {
                        stops: vec![
                            crate::paint::ColorStop {
                                at: 0.0,
                                color: [0.55, 0.38, 0.1, 1.0],
                            },
                            crate::paint::ColorStop {
                                at: 0.5,
                                color: [1.0, 0.86, 0.45, 1.0],
                            },
                            crate::paint::ColorStop {
                                at: 1.0,
                                color: [0.62, 0.45, 0.15, 1.0],
                            },
                        ],
                        ..Default::default()
                    },
                }),
                bevel: Some(Bevel {
                    size: 4.0,
                    ..default_bevel()
                }),
                drop_shadow: Some(Shadow {
                    blend: Blend::new(BlendMode::Multiply, 0.5),
                    ..shadow
                }),
                ..Default::default()
            },
        ),
    ]
}

/// the graph pieces for one layer's style
pub struct Built {
    /// what is below with the outside effects on it
    pub backdrop: Option<NodeId>,
    /// the layer's pixels faded by fill with the inside effects on top, ready to blend
    /// down with the layer's mode and opacity
    pub body: NodeId,
}

fn blur(g: &mut Graph, input: NodeId, size: f32) -> NodeId {
    // photoshop's size is roughly the reach of the blur; a sigma of a third reaches it
    g.add(
        Node::of(GaussianBlur)
            .param("radius", (size / 3.0).max(0.0))
            .param("edges", "transparent")
            .input(0, input),
    )
}

fn offset(angle: f32, distance: f32) -> [f32; 2] {
    // the light comes from `angle`, so the shadow falls the other way. y points down.
    let (s, c) = angle.to_radians().sin_cos();
    [-c * distance, s * distance]
}

fn effect(g: &mut Graph, kind: &str, color: [f32; 4], shape: NodeId, layer: NodeId) -> Node {
    let _ = g;
    Node::of(nodes::style::LayerEffect)
        .param("kind", kind)
        .param("color", color)
        .input(0, shape)
        .input(1, layer)
}

fn blend_on(g: &mut Graph, under: Option<NodeId>, over: NodeId, b: &Blend, scale: f32) -> NodeId {
    let mut n = Node::of(nodes::BlendNode)
        .param("mode", mode_param(b.mode))
        .param("opacity", (b.opacity * scale).clamp(0.0, 1.0) * 100.0)
        .input(1, over);
    if let Some(u) = under {
        n = n.input(0, u);
    }
    g.add(n)
}

/// keeps `node` only where `layer` has pixels
fn inside(g: &mut Graph, node: NodeId, layer: NodeId) -> NodeId {
    g.add(
        Node::of(nodes::MaskNode)
            .param("channel", "alpha")
            .input(0, node)
            .input(1, layer),
    )
}

/// adds a style's nodes for a layer whose (masked) pixels are `layer`. `opacity` is the
/// layer's, which the outside effects use because they skip the final blend.
pub fn build(
    g: &mut Graph,
    style: &LayerStyle,
    layer: NodeId,
    fill: f32,
    opacity: f32,
    backdrop: Option<NodeId>,
    canvas: IRect,
) -> Built {
    let mut back = backdrop;
    if let Some(s) = on(&style.drop_shadow, |e| &e.blend) {
        let b = blur(g, layer, s.size);
        let n = effect(g, "shadow", s.color, b, layer)
            .param("offset", offset(s.angle, s.distance))
            .param("spread", s.spread)
            .param("knockout", s.knockout);
        let n = g.add(n);
        back = Some(blend_on(g, back, n, &s.blend, opacity));
    }
    if let Some(s) = on(&style.outer_glow, |e| &e.blend) {
        let b = blur(g, layer, s.size);
        let n = effect(g, "outer_glow", s.color, b, layer).param("spread", s.spread);
        let n = g.add(n);
        back = Some(blend_on(g, back, n, &s.blend, opacity));
    }
    let mut body = if fill >= 1.0 {
        layer
    } else {
        g.add(
            Node::of(nodes::MaskNode)
                .param("outside", fill.clamp(0.0, 1.0))
                .input(0, layer),
        )
    };
    if let Some(s) = on(&style.inner_shadow, |e| &e.blend) {
        let b = blur(g, layer, s.size);
        let n = effect(g, "inner_shadow", s.color, b, layer)
            .param("offset", offset(s.angle, s.distance))
            .param("spread", s.spread);
        let n = g.add(n);
        body = blend_on(g, Some(body), n, &s.blend, 1.0);
    }
    if let Some(s) = on(&style.inner_glow, |e| &e.blend) {
        let b = blur(g, layer, s.size);
        let kind = if s.from_center {
            "inner_glow_center"
        } else {
            "inner_glow_edge"
        };
        let n = effect(g, kind, s.color, b, layer).param("spread", s.spread);
        let n = g.add(n);
        body = blend_on(g, Some(body), n, &s.blend, 1.0);
    }
    if let Some(s) = on(&style.satin, |e| &e.blend) {
        let b = blur(g, layer, s.size);
        let n = effect(g, "satin", s.color, b, layer)
            .param("offset", offset(s.angle, s.distance))
            .param("invert", s.invert);
        let n = g.add(n);
        body = blend_on(g, Some(body), n, &s.blend, 1.0);
    }
    if let Some(s) = on(&style.color_overlay, |e| &e.blend) {
        let solid = g.add(Node::of(nodes::SolidNode).param("color", s.color));
        let n = inside(g, solid, layer);
        body = blend_on(g, Some(body), n, &s.blend, 1.0);
    }
    if let Some(s) = on(&style.gradient_overlay, |e| &e.blend) {
        let grad = g.add(gradient_node(&s.gradient, canvas));
        let n = inside(g, grad, layer);
        body = blend_on(g, Some(body), n, &s.blend, 1.0);
    }
    if let Some(s) = on(&style.pattern_overlay, |e| &e.blend) {
        let pat = g.add(pattern_node(&s.pattern, s.scale, [0.0, 0.0]));
        let n = inside(g, pat, layer);
        body = blend_on(g, Some(body), n, &s.blend, 1.0);
    }
    if let Some(s) = on(&style.stroke, |e| &e.blend) {
        let morph = |g: &mut Graph, r: f32, mode: &str| {
            g.add(
                Node::of(nodes::style::Morph)
                    .param("radius", r)
                    .param("mode", mode)
                    .input(0, layer),
            )
        };
        let (outer, inner) = match s.position {
            StrokePosition::Outside => (morph(g, s.size, "grow"), layer),
            StrokePosition::Inside => (layer, morph(g, s.size, "shrink")),
            StrokePosition::Center => (
                morph(g, s.size / 2.0, "grow"),
                morph(g, s.size / 2.0, "shrink"),
            ),
        };
        let n = g.add(
            Node::of(nodes::style::LayerEffect)
                .param("kind", "stroke")
                .param("color", s.color)
                .input(0, outer)
                .input(1, layer)
                .input(2, inner),
        );
        body = blend_on(g, Some(body), n, &s.blend, 1.0);
    }
    if let Some(bv) = &style.bevel
        && !style.hidden
    {
        let soft = blur(g, layer, bv.size + bv.soften);
        // emboss lights the outside of the edge too, so it reads against any backdrop
        let lit = if bv.emboss {
            g.add(
                Node::of(nodes::style::Morph)
                    .param("radius", bv.size / 2.0)
                    .input(0, layer),
            )
        } else {
            layer
        };
        let common = |kind: &str, color: [f32; 4]| {
            Node::of(nodes::style::LayerEffect)
                .param("kind", kind)
                .param("color", color)
                .param("angle", bv.angle)
                .param("altitude", bv.altitude)
                .param("depth", bv.depth * (bv.size / 5.0).max(0.2))
                .param("down", bv.down)
                .input(0, soft)
                .input(1, lit)
        };
        if bv.shadow.enabled && bv.shadow.opacity > 0.0 {
            let n = g.add(common("bevel_shadow", bv.shadow_color));
            body = blend_on(g, Some(body), n, &bv.shadow, 1.0);
        }
        if bv.highlight.enabled && bv.highlight.opacity > 0.0 {
            let n = g.add(common("bevel_highlight", bv.highlight_color));
            body = blend_on(g, Some(body), n, &bv.highlight, 1.0);
        }
    }
    Built {
        backdrop: back,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_know_when_they_show() {
        assert!(!LayerStyle::default().shows());
        let mut s = LayerStyle {
            drop_shadow: Some(default_shadow()),
            ..Default::default()
        };
        assert!(s.shows());
        s.hidden = true;
        assert!(!s.shows());
        s.hidden = false;
        if let Some(d) = &mut s.drop_shadow {
            d.blend.enabled = false;
        }
        assert!(!s.shows());
        assert!(presets().iter().all(|p| p.style.shows()));
    }

    #[test]
    fn shadows_fall_away_from_the_light() {
        // light from the top left (135 degrees) throws the shadow down and right
        let o = offset(135.0, 10.0);
        assert!(o[0] > 0.0 && o[1] > 0.0, "{o:?}");
    }
}
