use std::io;
use std::path::{Path, PathBuf};

use vellum_core::JobError;

/// everything that can go wrong in the media crate. messages are written for people: they
/// say what happened and, where it helps, what to do next.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("cancelled")]
    Cancelled,
    #[error(
        "Vellum could not find FFmpeg, which it needs for video and audio. Install FFmpeg or set VELLUM_FFMPEG_DIR to the folder that holds it."
    )]
    FfmpegMissing,
    #[error("could not read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// a plain sentence, eg "Vellum cannot strip metadata from TIFF files yet."
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Decode(String),
    #[error("FFmpeg could not finish: {0}")]
    Ffmpeg(String),
    #[error("the media cache could not be used: {0}")]
    Cache(String),
}

impl MediaError {
    pub fn io(path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    pub fn decode(path: &Path, reason: impl std::fmt::Display) -> Self {
        Self::Decode(format!(
            "Vellum could not read {}: {reason}",
            vellum_core::files::display_name(path)
        ))
    }
}

impl From<rusqlite::Error> for MediaError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Cache(err.to_string())
    }
}

impl From<MediaError> for JobError {
    fn from(err: MediaError) -> Self {
        match err {
            MediaError::Cancelled => JobError::Cancelled,
            // a busy disk or a locked cache can clear up, a missing file will not
            MediaError::Io { ref source, .. } if source.kind() != io::ErrorKind::NotFound => {
                JobError::failed(err)
            }
            MediaError::Cache(_) => JobError::failed(err),
            other => JobError::fatal(other),
        }
    }
}

pub type MediaResult<T> = Result<T, MediaError>;
