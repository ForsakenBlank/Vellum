//! the render engine shared by the image editor, the video timeline, the colour page and
//! motion graphics.
//!
//! - [`graph`]: nodes, typed parameters ([`params`]) and the registry
//! - [`engine`]: renders a graph on the gpu or the cpu, caches results, recovers from a
//!   lost device
//! - [`nodes`]: the built in sources, blends, masks, transforms, colour nodes and effects
//! - [`blend`]: every photoshop blend mode, mirrored in `shaders/prelude.wgsl`
//! - [`color`]: colour spaces, transfer functions, icc profiles
//! - [`tiles`]: tiled sources and rendering outputs bigger than a gpu texture
//! - [`view`]: the viewport, which streams display tiles to the window
//! - [`hub`]: every open viewport, for the desktop shell
//!
//! see `ARCHITECTURE.md` section 24 for why it is shaped this way.

pub mod blend;
pub mod cache;
pub mod color;
pub mod engine;
pub mod gpu;
pub mod graph;
pub mod hub;
pub mod image;
pub mod nodes;
pub mod params;
pub mod rect;
pub mod tiles;
pub mod view;

pub use blend::BlendMode;
pub use color::{ColorSpace, ColorTransform, IccProfile, Primaries, Transfer, WorkingSpace};
pub use engine::{Backend, BackendChoice, Engine, EngineOptions, EngineStatus, RenderRequest};
pub use gpu::{GpuContext, GpuError, GpuOptions};
pub use graph::{
    Graph, ImageSource, Node, NodeData, NodeId, NodeInfo, NodeKind, NodeRegistry, RenderError,
};
pub use hub::{HubError, ViewHub, ViewInfo};
pub use image::{AlphaMode, CpuImage, Precision, SourceFormat, SourcePixels};
pub use params::{ParamSpec, ParamValue};
pub use rect::{Bounds, IRect};
pub use tiles::{ProceduralSource, SourceColor, TiledImage, render_tiled};
