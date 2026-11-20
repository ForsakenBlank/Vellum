use std::io;
use std::path::{Path, PathBuf};

use vellum_core::JobError;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error(
        "FFmpeg was not found. Install FFmpeg, or set the folder it is in under Settings > Media."
    )]
    FfmpegMissing,
    #[error("could not read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// the file is a kind vellum cannot read (yet). the message says what to do instead.
    #[error("{0}")]
    Unsupported(String),
    /// the file looks like something vellum reads, but it is damaged or unusual
    #[error("{0}")]
    Decode(String),
    /// an external tool (ffmpeg, ffprobe, heif-dec) failed
    #[error("{tool} failed: {message}")]
    Tool { tool: String, message: String },
    #[error("the media cache failed: {0}")]
    Cache(String),
    #[error("cancelled")]
    Cancelled,
}

impl MediaError {
    pub fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }

    pub fn decode(message: impl Into<String>) -> Self {
        Self::Decode(message.into())
    }

    pub(crate) fn tool(tool: &str, message: impl Into<String>) -> Self {
        Self::Tool {
            tool: tool.to_owned(),
            message: message.into(),
        }
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
            // a busy disk or a locked file can clear up, so these are worth another try
            MediaError::Io { .. } | MediaError::Cache(_) => JobError::failed(err),
            // the same file and the same tool give the same answer next time
            MediaError::FfmpegMissing
            | MediaError::Unsupported(_)
            | MediaError::Decode(_)
            | MediaError::Tool { .. } => JobError::fatal(err),
        }
    }
}

pub type MediaResult<T> = Result<T, MediaError>;
