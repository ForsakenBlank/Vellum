//! the image editor's core: documents, layers, selections, painting and files.
//!
//! - [`raster`]: sparse tiled pixels with copy on write tiles and mip levels
//! - [`document`]: the canvas, layers, groups, masks and the selection
//! - [`compose`]: the document as a render graph for `vellum-gpu`
//!
//! see `ARCHITECTURE.md` section 25 for why it is shaped this way.

pub mod adjustments;
pub mod canvas;
pub mod commands;
pub mod compose;
pub mod document;
pub mod export;
pub mod fills;
pub mod flatten;
pub mod fonts;
pub mod format;
pub mod heal;
pub mod layer_ops;
pub mod paint;
pub mod psd;
pub mod raster;
pub mod retouch;
pub mod selection;
pub mod service;
pub mod smart;
pub mod styles;
pub mod text;
pub mod transform;
pub mod vector;
pub mod vector_ops;
pub mod warp;

pub use compose::{Part, Preview, build};
pub use document::{Content, EditTarget, ImageDocument, Label, Layer, LayerId, LayerMask, Locks};
pub use raster::{Channels, Depth, Raster, TILE};
