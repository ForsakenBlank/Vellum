//! the media core: reading, probing and caching every kind of file vellum opens.
//!
//! - [`ffmpeg`]: finds and runs ffmpeg and ffprobe, probes files, checks hardware
//!   encoders and pulls exact frames out of video
//! - [`info`]: [`MediaInfo`], one description for images, video and audio
//! - [`images`]: decodes and describes every still and animated image format
//! - [`sniff`]: tells formats apart by their first bytes
//!
//! see `ARCHITECTURE.md` section 22 for how the pieces fit and why.

pub mod animation;
pub mod cache;
pub mod error;
pub mod exif_tags;
pub mod ffmpeg;
pub mod images;
pub mod info;
pub mod sniff;
pub mod strip;
pub mod thumbnail;
pub mod waveform;

pub use error::{MediaError, MediaResult};
pub use info::{MediaInfo, MediaKind};
