//! turns a document into a render graph for the phase 4 engine.
//!
//! layers composite bottom to top the way photoshop does:
//!
//! - a layer is its pixels, hidden by its mask if it has one, blended onto what is below
//!   with its blend mode, opacity and fill.
//! - a clipping group is a base layer and the clipped layers right above it. each clipped
//!   layer is blended onto the base, limited to where the base has pixels, and the result
//!   is blended down with the base's blend mode and opacity. the base's fill only fades
//!   the base's own pixels.
//! - a pass through group (photoshop's default) blends its children straight into what is
//!   below; its opacity and mask fade between the result and the backdrop. any other blend
//!   mode isolates the group: children composite on their own first.
//!
//! blending happens on the document's own values, as photoshop does, and one colour node
//! at the end takes the result into the working space. that also means a render of the
//! graph without that node ([`build_raw`]) gives back exact document values, which merging
//! layers and "sample all layers" rely on.
//!
//! the graph is plain data and cheap to rebuild after every change. sources hash the tile
//! revisions they read, so the engine only redraws what changed.

use std::hash::Hasher;
use std::sync::{Arc, OnceLock};

use vellum_gpu::graph::{Graph, ImageSource, Node, NodeId};
use vellum_gpu::image::SourcePixels;
use vellum_gpu::nodes::{self, blend::mode_param};
use vellum_gpu::{BlendMode, ColorTransform, IRect, NodeRegistry, WorkingSpace};

use crate::document::{Content, ImageDocument, Layer, LayerId};
use crate::raster::{Channels, Raster};

/// what stands in for part of a document while something is being drawn
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Pixels(LayerId),
    Mask(LayerId),
    Selection,
    /// a smart object's filter mask
    FilterMask(LayerId),
}

/// temporary changes shown while a tool is in use, never stored in the document
#[derive(Clone, Default)]
pub struct Preview {
    /// pixels that replace a part of the document, eg a brush stroke in progress
    pub replace: Option<(Part, Raster)>,
    /// layers drawn shifted by whole pixels while they are dragged
    pub offset: Option<(Vec<LayerId>, i32, i32)>,
    /// shows one layer's mask on its own in black and white
    pub show_mask: Option<LayerId>,
    /// a free transform in progress
    pub transform: Option<TransformPreview>,
    /// liquify or puppet warp in progress: a layer drawn through a mesh
    pub mesh: Option<(LayerId, Arc<crate::warp::Mesh>)>,
}

/// what a free transform shows before it is committed
#[derive(Clone, Debug)]
pub struct TransformPreview {
    pub spec: crate::transform::TransformSpec,
    pub plan: crate::transform::Plan,
}

/// a raster in the graph, as document values, with an optional whole pixel shift
pub struct RasterSource {
    raster: Raster,
    dx: i32,
    dy: i32,
}

impl RasterSource {
    pub fn new(raster: Raster) -> Self {
        Self {
            raster,
            dx: 0,
            dy: 0,
        }
    }

    pub fn shifted(mut self, dx: i32, dy: i32) -> Self {
        self.dx = dx;
        self.dy = dy;
        self
    }

    /// the shift at a mip level, rounded down. at a coarse level a shift that is not a
    /// multiple of the level's scale lands up to half a screen pixel off, only while
    /// dragging.
    fn shift_at(&self, level: u32) -> (i32, i32) {
        let s = 1i32 << level.min(30);
        (self.dx.div_euclid(s), self.dy.div_euclid(s))
    }
}

impl ImageSource for RasterSource {
    fn bounds(&self) -> IRect {
        self.raster.tile_extent().translate(self.dx, self.dy)
    }

    fn key(&self, region: IRect, level: u32, h: &mut dyn Hasher) {
        h.write_i32(self.dx);
        h.write_i32(self.dy);
        let r0 = region.to_level0(level).translate(-self.dx, -self.dy);
        self.raster.hash_region(r0, h);
    }

    fn read(&self, region: IRect, level: u32) -> Option<SourcePixels> {
        let (sx, sy) = self.shift_at(level);
        let mut px = self.raster.read_level(region.translate(-sx, -sy), level)?;
        px.rect = px.rect.translate(sx, sy);
        Some(px)
    }

    fn to_working(&self, _working: WorkingSpace) -> ColorTransform {
        ColorTransform::identity()
    }
}

/// every render node, for adjustments and filters named by id
pub fn registry() -> &'static NodeRegistry {
    static REGISTRY: OnceLock<NodeRegistry> = OnceLock::new();
    REGISTRY.get_or_init(NodeRegistry::with_builtins)
}

struct Builder<'a> {
    g: Graph,
    preview: &'a Preview,
    /// the selection, which floats the pixels a transform moves
    selection: Option<&'a Raster>,
    doc: &'a ImageDocument,
}

impl Builder<'_> {
    fn source(&mut self, source: RasterSource) -> NodeId {
        self.g
            .add(Node::of(nodes::ImageNode).source(Arc::new(source)))
    }

    fn shift(&self, id: LayerId) -> (i32, i32) {
        match &self.preview.offset {
            Some((ids, dx, dy)) if ids.contains(&id) => (*dx, *dy),
            _ => (0, 0),
        }
    }

    /// the transform previewed on a part, if any
    fn transforming(&self, part: &Part) -> Option<&TransformPreview> {
        self.preview
            .transform
            .as_ref()
            .filter(|t| t.plan.parts.contains(part))
    }

    fn replaced(&self, part: &Part) -> Option<&Raster> {
        match &self.preview.replace {
            Some((p, r)) if p == part => Some(r),
            _ => None,
        }
    }

    /// the layer's own pixels, or its isolated group composite
    fn content(&mut self, layer: &Layer) -> Option<NodeId> {
        let node = match &layer.content {
            Content::Pixels(raster) => {
                let raster = self
                    .replaced(&Part::Pixels(layer.id))
                    .unwrap_or(raster)
                    .clone();
                if raster.is_empty() {
                    return None;
                }
                let (dx, dy) = self.shift(layer.id);
                self.source(RasterSource::new(raster).shifted(dx, dy))
            }
            Content::Group { children, .. } => {
                let inside = self.stack(children, None);
                let Some(board) = layer.artboard else {
                    return inside;
                };
                // an artboard: its background under its contents, both cut to its box
                let node = match (board.background, inside) {
                    (Some(bg), inside) => {
                        let bg = crate::vector_ops::to_doc(self.doc)(bg);
                        let solid = self.g.add(Node::of(nodes::SolidNode).param("color", bg));
                        match inside {
                            Some(i) => {
                                Some(self.blend(Some(solid), i, BlendMode::Normal, 1.0, 1.0, None))
                            }
                            None => Some(solid),
                        }
                    }
                    (None, inside) => inside,
                };
                let node = node?;
                let mut rect = Raster::gray(crate::raster::Depth::U8, 0.0);
                rect.update(board.rect, |_, _, v| v[0] = 1.0);
                let m = self.source(RasterSource::new(rect));
                return Some(
                    self.g
                        .add(Node::of(nodes::MaskNode).input(0, node).input(1, m)),
                );
            }
            Content::Adjustment(_) => return None,
            Content::Fill(fill) => {
                let n = crate::fills::node(fill, self.doc.canvas());
                self.g.add(n)
            }
            Content::Smart(s) => self.smart(layer.id, s),
            Content::Text { pixels, .. } | Content::Shape { pixels, .. } => {
                if pixels.is_empty() {
                    return None;
                }
                let (dx, dy) = self.shift(layer.id);
                self.source(RasterSource::new(pixels.clone()).shifted(dx, dy))
            }
        };
        let node = match &self.preview.mesh {
            Some((id, mesh)) if *id == layer.id => {
                let n = mesh.node().input(0, node);
                self.g.add(n)
            }
            _ => node,
        };
        if let Some(t) = self.transforming(&Part::Pixels(layer.id)).cloned() {
            let sel = match (t.plan.lift, self.selection) {
                (true, Some(s)) => Some(self.source(RasterSource::new(s.clone()))),
                _ => None,
            };
            if let Ok(moved) = crate::transform::pixels_node(&mut self.g, node, sel, &t.spec) {
                return Some(moved);
            }
        }
        Some(node)
    }

    /// a smart object: its pixels placed by its transform, then each smart filter, then
    /// the filter mask fading between the two
    fn smart(&mut self, id: LayerId, s: &crate::smart::SmartObject) -> NodeId {
        let (dx, dy) = self.shift(id);
        let mut node = if s.is_identity_placement() {
            let [.., e, f] = s.transform;
            self.source(RasterSource::new(s.source.clone()).shifted(e as i32 + dx, f as i32 + dy))
        } else {
            let src = self.source(RasterSource::new(s.source.clone()));
            let mut m = s.matrix();
            m[2] += dx as f32;
            m[5] += dy as f32;
            self.g.add(
                Node::of(nodes::TransformNode)
                    .param("matrix", m)
                    .input(0, src),
            )
        };
        if !s.filters_visible || s.filters.iter().all(|f| !f.visible) {
            return node;
        }
        let before = node;
        let area = s.bounds().translate(dx, dy);
        let canvas = self.doc.canvas();
        for f in s.filters.iter().filter(|f| f.visible) {
            let Some(n) = crate::smart::filter_node(&f.kind, &f.params, area, canvas, registry())
            else {
                continue;
            };
            let after = self.g.add(n.input(0, node));
            node = crate::smart::blend_filter(&mut self.g, node, after, f);
        }
        if let Some(mask) = &s.filter_mask {
            let mask = self.replaced(&Part::FilterMask(id)).unwrap_or(mask).clone();
            let fill = mask.fill();
            let m = self.source(RasterSource::new(mask));
            node = self.g.add(
                Node::of(nodes::style::MixNode)
                    .param("masked", true)
                    .param("outside", fill)
                    .input(0, before)
                    .input(1, node)
                    .input(2, m),
            );
        }
        node
    }

    /// lands an adjustment's result: faded by opacity and fill through the layer's mask,
    /// or blended with its mode
    fn land(&mut self, layer: &Layer, before: NodeId, after: NodeId) -> NodeId {
        let amount = (layer.opacity * layer.fill).clamp(0.0, 1.0) * 100.0;
        if layer.blend != BlendMode::Normal {
            let after = self.masked(layer, after);
            return self.blend(
                Some(before),
                after,
                layer.blend,
                layer.opacity,
                layer.fill,
                None,
            );
        }
        let mut mix = Node::of(nodes::style::MixNode)
            .param("amount", amount)
            .input(0, before)
            .input(1, after);
        if let Some(mask) = layer.mask.as_ref().filter(|m| m.enabled) {
            let raster = self
                .replaced(&Part::Mask(layer.id))
                .unwrap_or(&mask.raster)
                .clone();
            let fill = raster.fill();
            let m = self.source(RasterSource::new(raster));
            mix = mix.param("masked", true).param("outside", fill).input(2, m);
        }
        self.g.add(mix)
    }

    /// an adjustment layer applied to `backdrop`
    fn adjust(&mut self, layer: &Layer, backdrop: Option<NodeId>) -> Option<NodeId> {
        let Content::Adjustment(adj) = &layer.content else {
            return backdrop;
        };
        let back = backdrop?;
        let Some(node) = crate::adjustments::node(adj, self.doc, registry()) else {
            return backdrop;
        };
        let after = self.g.add(node.input(0, back));
        Some(self.land(layer, back, after))
    }

    /// hides parts of `node` with the layer's mask
    fn masked(&mut self, layer: &Layer, node: NodeId) -> NodeId {
        let node = match layer.vector_mask.as_ref().filter(|v| v.mask.enabled) {
            Some(v) => {
                let fill = v.raster.fill();
                let (dx, dy) = self.shift(layer.id);
                let m = self.source(RasterSource::new(v.raster.clone()).shifted(dx, dy));
                self.g.add(
                    Node::of(nodes::MaskNode)
                        .param("outside", fill)
                        .input(0, node)
                        .input(1, m),
                )
            }
            None => node,
        };
        let Some(mask) = layer.mask.as_ref().filter(|m| m.enabled) else {
            return node;
        };
        let raster = self
            .replaced(&Part::Mask(layer.id))
            .unwrap_or(&mask.raster)
            .clone();
        let fill = raster.fill();
        let (dx, dy) = if mask.linked {
            self.shift(layer.id)
        } else {
            (0, 0)
        };
        let mut m = self.source(RasterSource::new(raster).shifted(dx, dy));
        if let Some(t) = self.transforming(&Part::Mask(layer.id)).cloned()
            && let Ok(moved) = crate::transform::gray_node(&mut self.g, m, fill, &t.spec)
        {
            m = moved;
        }
        self.g.add(
            Node::of(nodes::MaskNode)
                .param("outside", fill)
                .input(0, node)
                .input(1, m),
        )
    }

    fn blend(
        &mut self,
        backdrop: Option<NodeId>,
        layer: NodeId,
        mode: BlendMode,
        opacity: f32,
        fill: f32,
        clip: Option<NodeId>,
    ) -> NodeId {
        let mut node = Node::of(nodes::BlendNode)
            .param("mode", mode_param(mode))
            .param("opacity", opacity.clamp(0.0, 1.0) * 100.0)
            .param("fill", fill.clamp(0.0, 1.0) * 100.0)
            .input(1, layer);
        if let Some(b) = backdrop {
            node = node.input(0, b);
        }
        if let Some(c) = clip {
            node = node.param("mask_channel", "alpha").input(2, c);
        }
        self.g.add(node)
    }

    fn stack(&mut self, layers: &[Layer], backdrop: Option<NodeId>) -> Option<NodeId> {
        let mut acc = backdrop;
        let mut i = 0;
        while i < layers.len() {
            let base = &layers[i];
            let mut j = i + 1;
            while j < layers.len() && layers[j].clipped {
                j += 1;
            }
            if base.visible {
                acc = self.layer(base, &layers[i + 1..j], acc);
            }
            i = j;
        }
        acc
    }

    fn layer(&mut self, base: &Layer, clips: &[Layer], backdrop: Option<NodeId>) -> Option<NodeId> {
        if matches!(base.content, Content::Adjustment(_)) {
            return self.adjust(base, backdrop);
        }
        if let Content::Group {
            children,
            pass_through: true,
            ..
        } = &base.content
            && base.artboard.is_none()
            && clips.is_empty()
            && base.blend == BlendMode::Normal
        {
            let after = self.stack(children, backdrop)?;
            let faded = base.opacity < 1.0;
            let masked = base.mask.as_ref().is_some_and(|m| m.enabled);
            if !faded && !masked {
                return Some(after);
            }
            let after = self.masked(base, after);
            return Some(self.blend(backdrop, after, BlendMode::Normal, base.opacity, 1.0, None));
        }

        let Some(content) = self.content(base) else {
            // an empty base still clips its layers to nothing
            return backdrop;
        };
        let raw = self.masked(base, content);
        let mut result = if clips.is_empty() || base.fill >= 1.0 {
            raw
        } else {
            self.g.add(
                Node::of(nodes::MaskNode)
                    .param("outside", base.fill.clamp(0.0, 1.0))
                    .input(0, raw),
            )
        };
        let base_fill = if clips.is_empty() { base.fill } else { 1.0 };
        for clip in clips.iter().filter(|c| c.visible) {
            if matches!(clip.content, Content::Adjustment(_)) {
                if let Some(r) = self.adjust(clip, Some(result)) {
                    result = r;
                }
                continue;
            }
            let Some(c) = self.content(clip) else {
                continue;
            };
            let c = self.masked(clip, c);
            result = self.blend(
                Some(result),
                c,
                clip.blend,
                clip.opacity,
                clip.fill,
                Some(raw),
            );
        }
        if let Some(style) = base.style.as_ref().filter(|s| s.shows()) {
            let built = crate::styles::build(
                &mut self.g,
                style,
                result,
                base_fill,
                base.opacity,
                backdrop,
                self.doc.canvas(),
            );
            return Some(self.blend(
                built.backdrop,
                built.body,
                base.blend,
                base.opacity,
                1.0,
                None,
            ));
        }
        Some(self.blend(backdrop, result, base.blend, base.opacity, base_fill, None))
    }
}

/// the graph that draws `doc` in the working space, and the node to render. `preview`
/// adds whatever a tool is showing without changing the document.
pub fn build(doc: &ImageDocument, preview: &Preview) -> (Graph, NodeId) {
    let (mut g, top) = build_raw(doc, preview);
    let to_working = doc.color.to_working(doc.working());
    if to_working.is_identity() {
        return (g, top);
    }
    let out = g.add(Node::of(nodes::LutNode).transform(to_working).input(0, top));
    (g, out)
}

/// like [`build`], but leaves values as the document stores them (premultiplied)
pub fn build_raw(doc: &ImageDocument, preview: &Preview) -> (Graph, NodeId) {
    let mut b = Builder {
        g: Graph::new(),
        preview,
        selection: doc.selection.as_ref(),
        doc,
    };
    if let Some(id) = preview.show_mask
        && let Some(mask) = doc.layer(id).and_then(|l| l.mask.as_ref())
    {
        let raster = b.replaced(&Part::Mask(id)).unwrap_or(&mask.raster).clone();
        let fill = raster.fill();
        let white =
            b.g.add(Node::of(nodes::SolidNode).param("color", [fill, fill, fill, 1.0]));
        let m = b.source(RasterSource::new(raster));
        let top = b.blend(Some(white), m, BlendMode::Normal, 1.0, 1.0, None);
        return (b.g, top);
    }
    let mut top = b.stack(&doc.layers, None);
    if doc.quick_mask {
        let selection = b
            .replaced(&Part::Selection)
            .cloned()
            .or_else(|| doc.selection.clone())
            .unwrap_or_else(|| Raster::gray(crate::raster::Depth::U8, 0.0));
        let red =
            b.g.add(Node::of(nodes::SolidNode).param("color", [1.0, 0.0, 0.0, 1.0]));
        let fill = selection.fill();
        let s = b.source(RasterSource::new(selection));
        let overlay = b.g.add(
            Node::of(nodes::MaskNode)
                .param("invert", true)
                .param("outside", fill)
                .input(0, red)
                .input(1, s),
        );
        top = Some(b.blend(top, overlay, BlendMode::Normal, 0.5, 1.0, None));
    }
    let top = top.unwrap_or_else(|| {
        b.source(RasterSource::new(Raster::new(
            Channels::Rgba,
            doc.depth,
            0.0,
        )))
    });
    (b.g, top)
}
