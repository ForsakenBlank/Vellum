//! what kind of file something is, and which workspace opens it.
//!
//! the table here is the single source for file routing. the ui gets the same table as
//! `ui/src/bindings/fileKinds.json`, written by a test in [`crate::ui`], so a file dropped
//! on the window and a file opened from the cli always land in the same place.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum FileKind {
    Image,
    /// camera raw files, decoded through libraw from phase 2
    Raw,
    /// gifs are animations first, so they get their own kind
    Gif,
    Video,
    Audio,
    Vector,
    Pdf,
    /// a `.vellum` project folder
    Project,
}

/// the workspaces from `PLAN.md` section 4
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Workspace {
    QuickTools,
    Image,
    Video,
    Color,
    Audio,
    Motion,
    Vector,
}

/// one row of the routing table, as the ui sees it
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileKindInfo {
    pub kind: FileKind,
    /// "video", "camera raw photo", used in sentences like "Open 2 video files"
    pub label: &'static str,
    pub workspace: Workspace,
    pub extensions: &'static [&'static str],
}

/// every file type vellum knows, lowercase extensions without the dot
pub const FILE_KINDS: &[FileKindInfo] = &[
    FileKindInfo {
        kind: FileKind::Image,
        label: "image",
        workspace: Workspace::Image,
        extensions: &[
            "png", "jpg", "jpeg", "jfif", "webp", "avif", "heic", "heif", "bmp", "tif", "tiff",
            "psd", "psb", "tga", "exr", "hdr", "ico", "jxl", "qoi",
        ],
    },
    FileKindInfo {
        kind: FileKind::Raw,
        label: "camera raw photo",
        workspace: Workspace::Image,
        extensions: &[
            "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "raf", "srw", "pef",
        ],
    },
    FileKindInfo {
        kind: FileKind::Gif,
        label: "GIF",
        // the gif editor and timeline both treat it as frames over time
        workspace: Workspace::Video,
        extensions: &["gif", "apng"],
    },
    FileKindInfo {
        kind: FileKind::Video,
        label: "video",
        workspace: Workspace::Video,
        extensions: &[
            "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpg", "mpeg", "ts", "mts",
            "m2ts", "3gp", "ogv", "mxf",
        ],
    },
    FileKindInfo {
        kind: FileKind::Audio,
        label: "audio",
        workspace: Workspace::Audio,
        extensions: &[
            "mp3", "wav", "flac", "ogg", "oga", "opus", "m4a", "aac", "aiff", "aif", "wma", "alac",
        ],
    },
    FileKindInfo {
        kind: FileKind::Vector,
        label: "vector",
        workspace: Workspace::Vector,
        extensions: &["svg", "svgz", "eps", "ai"],
    },
    FileKindInfo {
        kind: FileKind::Pdf,
        label: "PDF",
        workspace: Workspace::Vector,
        extensions: &["pdf"],
    },
    FileKindInfo {
        kind: FileKind::Project,
        label: "project",
        // image projects are the only kind so far; timeline projects (phase 8) will read
        // the type from the project's manifest
        workspace: Workspace::Image,
        extensions: &["vellum"],
    },
];

impl FileKind {
    pub fn info(self) -> &'static FileKindInfo {
        // the table has a row for every kind, checked by a test
        FILE_KINDS
            .iter()
            .find(|info| info.kind == self)
            .unwrap_or(&FILE_KINDS[0])
    }

    pub fn workspace(self) -> Workspace {
        self.info().workspace
    }

    /// works out the kind from the file extension. returns none for files vellum cannot open.
    pub fn of_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        FILE_KINDS
            .iter()
            .find(|info| info.extensions.contains(&ext.as_str()))
            .map(|info| info.kind)
    }
}

/// the last part of a path for tabs and lists, falling back to the whole path
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    #[test]
    fn every_kind_has_a_row() {
        let kinds = [
            FileKind::Image,
            FileKind::Raw,
            FileKind::Gif,
            FileKind::Video,
            FileKind::Audio,
            FileKind::Vector,
            FileKind::Pdf,
            FileKind::Project,
        ];
        for kind in kinds {
            assert_eq!(kind.info().kind, kind);
        }
        assert_eq!(FILE_KINDS.len(), kinds.len());
    }

    #[test]
    fn extensions_are_unique_and_lowercase() {
        let mut seen = HashSet::new();
        for info in FILE_KINDS {
            for ext in info.extensions {
                assert_eq!(*ext, ext.to_ascii_lowercase());
                assert!(!ext.starts_with('.'));
                assert!(seen.insert(*ext), "{ext} is listed twice");
            }
        }
    }

    #[test]
    fn routes_by_extension_ignoring_case() {
        let kind = |p: &str| FileKind::of_path(&PathBuf::from(p));
        assert_eq!(kind("holiday.MP4"), Some(FileKind::Video));
        assert_eq!(kind("/a/b/photo.jpeg"), Some(FileKind::Image));
        assert_eq!(kind("IMG_0001.CR3"), Some(FileKind::Raw));
        assert_eq!(kind("meme.gif"), Some(FileKind::Gif));
        assert_eq!(kind("song.flac"), Some(FileKind::Audio));
        assert_eq!(kind("logo.svg"), Some(FileKind::Vector));
        assert_eq!(kind("report.pdf"), Some(FileKind::Pdf));
        assert_eq!(kind("trip.vellum"), Some(FileKind::Project));
        assert_eq!(kind("notes.txt"), None);
        assert_eq!(kind("no_extension"), None);
    }

    #[test]
    fn kinds_open_in_the_expected_workspace() {
        assert_eq!(FileKind::Raw.workspace(), Workspace::Image);
        assert_eq!(FileKind::Gif.workspace(), Workspace::Video);
        assert_eq!(FileKind::Audio.workspace(), Workspace::Audio);
        assert_eq!(FileKind::Pdf.workspace(), Workspace::Vector);
    }

    #[test]
    fn display_name_is_the_file_name() {
        assert_eq!(display_name(Path::new("/x/y/clip.mov")), "clip.mov");
        assert_eq!(display_name(Path::new("/")), "/");
    }
}
