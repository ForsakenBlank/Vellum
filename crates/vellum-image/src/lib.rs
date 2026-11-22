//! the image editor's core: documents, layers, selections, painting and files.
//!
//! - [`raster`]: sparse tiled pixels with copy on write tiles and mip levels
//! - [`document`]: the canvas, layers, groups, masks and the selection
//! - [`compose`]: the document as a render graph for `vellum-gpu`
//!
//! see `ARCHITECTURE.md` section 25 for why it is shaped this way.

pub mod canvas;
pub mod commands;
pub mod compose;
pub mod document;
pub mod export;
pub mod flatten;
pub mod format;
pub mod paint;
pub mod psd;
pub mod raster;
pub mod selection;
pub mod service;
pub mod transform;

pub use compose::{Part, Preview, build};
pub use document::{Content, EditTarget, ImageDocument, Label, Layer, LayerId, LayerMask, Locks};
pub use raster::{Channels, Depth, Raster, TILE};
