//! the media core: reading, probing, decoding and encoding every file vellum opens.
//!
//! - [`ffmpeg`]: finds ffmpeg, runs it with cancel and progress, detects hardware encoders
//!   and encodes with a software fallback
//! - [`picture`]: one decoder api for photos, camera raw, svg and psd
//! - [`probe`]: the unified [`MediaInfo`] for any file
//! - [`seek`]: frame accurate access to video frames
//! - [`thumbs`] and [`waveform`]: previews, filmstrips and audio peaks
//! - [`cache`]: the sqlite backed cache those previews live in
//! - [`metadata`]: writes a copy of a file without its metadata
//! - [`service`]: ties it all to the job queue and the event bus for the app
//!
//! see `ARCHITECTURE.md` section 22 for the design.

pub mod cache;
pub mod control;
pub mod error;
pub mod ffmpeg;
pub mod metadata;
pub mod picture;
pub mod probe;
pub mod seek;
pub mod service;
#[doc(hidden)]
pub mod testmedia;
pub mod thumbs;
pub mod waveform;

pub use control::{Control, Unattended};
pub use error::{MediaError, MediaResult};
pub use picture::{DecodeOptions, Picture, PictureFormat};
pub use probe::{MediaInfo, MediaKind};
pub use service::{MediaAsset, MediaService};
