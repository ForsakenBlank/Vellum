//! the render graph: nodes, their kinds and the registry.
//!
//! a graph is plain data built by a workspace (the image editor turns its layers into
//! nodes, the timeline turns clips into nodes). nodes are cheap to rebuild because results
//! are cached by content: a node's cache key hashes its kind, its parameters, the keys of
//! its inputs and, for sources, the revision of the pixels it reads. rebuilding a graph
//! that did not change hits the cache; changing one parameter only misses for that node
//! and everything downstream of it.

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;

use crate::color::{ColorTransform, WorkingSpace};
use crate::gpu::{GpuError, GpuExec, GpuImage};
use crate::image::{CpuImage, Precision, SourcePixels};
use crate::params::{ParamMap, ParamSpec, ParamValue, Params};
use crate::rect::{Bounds, IRect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NodeCategory {
    Source,
    Effect,
    Color,
    Blend,
    Mask,
    Transform,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InputSpec {
    pub id: &'static str,
    pub label: &'static str,
}

/// what a node kind is, for the ui and for keyframing
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeInfo {
    /// stable id, eg `effect.gaussian_blur`
    pub id: &'static str,
    pub title: &'static str,
    pub category: NodeCategory,
    pub description: &'static str,
    pub inputs: &'static [InputSpec],
    pub params: &'static [ParamSpec],
}

pub const NO_INPUTS: &[InputSpec] = &[];
pub const ONE_INPUT: &[InputSpec] = &[InputSpec {
    id: "image",
    label: "Image",
}];

#[derive(Debug, thiserror::Error, Clone)]
pub enum RenderError {
    #[error(transparent)]
    Gpu(#[from] GpuError),
    #[error("the graph has no node {0}")]
    MissingNode(u32),
    #[error("the graph loops back on itself at node {0}")]
    Cycle(u32),
    #[error("the render was cancelled")]
    Cancelled,
    #[error("a {0}x{1} area is too big to render in one piece")]
    TooLarge(u32, u32),
}

/// pixels from outside the graph: a layer, a decoded photo, a video frame
pub trait ImageSource: Send + Sync + 'static {
    /// the area with pixels, at level 0
    fn bounds(&self) -> IRect;

    /// feeds whatever identifies the pixels inside `region` (at `level`) into `h`. a layer
    /// hashes the revisions of the tiles it overlaps, so painting one tile only invalidates
    /// renders that touch it.
    fn key(&self, region: IRect, level: u32, h: &mut dyn Hasher);

    /// the pixels inside `region` at `level` (level 1 is half size), or none if empty.
    /// the result may cover less than `region`.
    fn read(&self, region: IRect, level: u32) -> Option<SourcePixels>;

    /// how to bring this source's values into the working space
    fn to_working(&self, working: WorkingSpace) -> ColorTransform;
}

/// data a node carries besides its parameters
#[derive(Clone)]
pub enum Attachment {
    /// the pixels of a source node
    Source(Arc<dyn ImageSource>),
    /// a colour transform that does not fit in parameters, like an icc lut
    Color(Arc<ColorTransform>),
    /// numbers that do not fit in parameters, like a warp mesh
    Data(Arc<NodeData>),
}

/// floats a node reads besides its parameters, with a key that changes when they do
#[derive(Debug, Clone, PartialEq)]
pub struct NodeData {
    pub values: Vec<f32>,
    pub key: u64,
}

impl NodeData {
    pub fn new(values: Vec<f32>) -> Self {
        let mut h = std::hash::DefaultHasher::new();
        for v in &values {
            h.write_u32(v.to_bits());
        }
        Self {
            key: h.finish(),
            values,
        }
    }
}

/// what a node produced
pub enum Out<T> {
    New(T),
    /// the node changes nothing; use this input as its result
    Input(usize),
    /// fully transparent
    Empty,
}

/// what a node sees while it renders
#[derive(Clone, Copy)]
pub struct NodeCx<'a> {
    pub params: Params<'a>,
    pub level: u32,
    pub working: WorkingSpace,
    pub precision: Precision,
    pub attachment: Option<&'a Attachment>,
}

impl NodeCx<'_> {
    /// how many level pixels one document pixel is, 1 at level 0
    pub fn scale(&self) -> f32 {
        1.0 / (1u64 << self.level.min(30)) as f32
    }

    /// colours are kept in [0, 1] in gamma encoded working spaces
    pub fn clamp(&self) -> bool {
        self.working.encoding != crate::color::Transfer::Linear
    }

    pub fn source(&self) -> Option<&Arc<dyn ImageSource>> {
        match self.attachment {
            Some(Attachment::Source(s)) => Some(s),
            _ => None,
        }
    }

    pub fn color_transform(&self) -> Option<&Arc<ColorTransform>> {
        match self.attachment {
            Some(Attachment::Color(t)) => Some(t),
            _ => None,
        }
    }
}

impl<'a> NodeCx<'a> {
    /// the node's data attachment, for as long as the node lives
    pub fn data(&self) -> Option<&'a Arc<NodeData>> {
        match self.attachment {
            Some(Attachment::Data(d)) => Some(d),
            _ => None,
        }
    }
}

/// a kind of node. one implementation per file under `nodes/`, registered in
/// [`NodeRegistry::with_builtins`].
///
/// every node has a cpu path and a gpu path that must agree; the golden tests run both.
pub trait NodeKind: Send + Sync + 'static {
    fn info(&self) -> &'static NodeInfo;

    /// where this node can produce pixels at `cx.level`, given its inputs' bounds there
    /// (none for an input that is not connected). the default suits colour effects:
    /// exactly where the first input has pixels.
    fn bounds(&self, _cx: &NodeCx, inputs: &[Option<Bounds>]) -> Bounds {
        input_bounds(inputs, 0)
    }

    /// the part of input `input` needed to produce `region`. spatial effects grow it.
    fn input_region(&self, _cx: &NodeCx, _input: usize, region: IRect) -> IRect {
        region
    }

    /// extra cache key material beyond the parameters, for sources
    fn key(&self, _cx: &NodeCx, _region: IRect, _h: &mut dyn Hasher) {}

    /// renders `region` on the cpu. inputs are none where they have no pixels. `region` is
    /// already clipped to this node's bounds and never empty.
    fn cpu(&self, cx: &NodeCx, inputs: &[Option<&CpuImage>], region: IRect) -> Out<CpuImage>;

    /// renders `region` on the gpu
    fn gpu(
        &self,
        cx: &NodeCx,
        gx: &mut GpuExec,
        inputs: &[Option<&GpuImage>],
        region: IRect,
    ) -> Result<Out<GpuImage>, GpuError>;
}

/// the bounds of input `i`, empty if it is not connected
pub fn input_bounds(inputs: &[Option<Bounds>], i: usize) -> Bounds {
    inputs.get(i).copied().flatten().unwrap_or(Bounds::Empty)
}

/// a node's index in its graph. not exported to typescript, where `NodeId` already names
/// a history node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct NodeId(pub u32);

#[derive(Clone)]
pub struct Node {
    pub kind: Arc<dyn NodeKind>,
    pub params: ParamMap,
    pub inputs: Vec<Option<NodeId>>,
    pub attachment: Option<Attachment>,
    /// skip this node and pass its first input through, like a disabled effect
    pub bypass: bool,
}

impl Node {
    pub fn new(kind: Arc<dyn NodeKind>) -> Self {
        let n = kind.info().inputs.len();
        Self {
            kind,
            params: ParamMap::new(),
            inputs: vec![None; n],
            attachment: None,
            bypass: false,
        }
    }

    pub fn of(kind: impl NodeKind) -> Self {
        Self::new(Arc::new(kind))
    }

    pub fn param(mut self, id: &str, value: impl Into<ParamValue>) -> Self {
        self.params.insert(id.to_owned(), value.into());
        self
    }

    pub fn input(mut self, index: usize, node: NodeId) -> Self {
        if self.inputs.len() <= index {
            self.inputs.resize(index + 1, None);
        }
        self.inputs[index] = Some(node);
        self
    }

    pub fn source(mut self, source: Arc<dyn ImageSource>) -> Self {
        self.attachment = Some(Attachment::Source(source));
        self
    }

    pub fn transform(mut self, transform: ColorTransform) -> Self {
        self.attachment = Some(Attachment::Color(Arc::new(transform)));
        self
    }

    pub fn data(mut self, data: NodeData) -> Self {
        self.attachment = Some(Attachment::Data(Arc::new(data)));
        self
    }

    pub fn id(&self) -> &'static str {
        self.kind.info().id
    }
}

/// a render graph. nodes refer to their inputs by id; ids are indexes into the graph.
#[derive(Clone, Default)]
pub struct Graph {
    nodes: Vec<Node>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, node: Node) -> NodeId {
        self.nodes.push(node);
        NodeId(self.nodes.len() as u32 - 1)
    }

    pub fn node(&self, id: NodeId) -> Result<&Node, RenderError> {
        self.nodes
            .get(id.0 as usize)
            .ok_or(RenderError::MissingNode(id.0))
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0 as usize)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// checks every input exists and nothing loops, from `output` down
    pub fn validate(&self, output: NodeId) -> Result<(), RenderError> {
        fn visit(
            g: &Graph,
            id: NodeId,
            state: &mut HashMap<NodeId, bool>,
        ) -> Result<(), RenderError> {
            match state.get(&id) {
                Some(true) => return Ok(()),
                Some(false) => return Err(RenderError::Cycle(id.0)),
                None => {}
            }
            state.insert(id, false);
            for input in g.node(id)?.inputs.iter().flatten() {
                visit(g, *input, state)?;
            }
            state.insert(id, true);
            Ok(())
        }
        visit(self, output, &mut HashMap::new())
    }
}

/// every node kind by id, so graphs can be built from saved data and the ui can list them
#[derive(Clone, Default)]
pub struct NodeRegistry {
    kinds: HashMap<&'static str, Arc<dyn NodeKind>>,
}

impl NodeRegistry {
    pub fn register(&mut self, kind: impl NodeKind) {
        let id = kind.info().id;
        self.kinds.insert(id, Arc::new(kind));
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn NodeKind>> {
        self.kinds.get(id).cloned()
    }

    pub fn infos(&self) -> Vec<&'static NodeInfo> {
        let mut infos: Vec<_> = self.kinds.values().map(|k| k.info()).collect();
        infos.sort_by_key(|i| i.id);
        infos
    }

    /// a node of the given kind with default parameters
    pub fn create(&self, id: &str) -> Option<Node> {
        self.get(id).map(Node::new)
    }

    /// every node vellum ships
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        crate::nodes::register_builtins(&mut r);
        r
    }
}
