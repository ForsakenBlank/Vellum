//! what kind of thing a file is, and where it should open.
//!
//! phase 1 decides by extension only. phase 2 can swap `classify` for a real
//! probe without anything else changing, because everything that routes files
//! (the drop overlay, the open command, the recent list) goes through here.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Vector,
    /// pdfs and other page based files
    Document,
    /// a `.vellum` project folder
    Project,
    Unknown,
}

const IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "webp", "avif", "heic", "heif", "tif", "tiff", "gif",
    "bmp", "ico", "psd", "psb", "exr", "hdr", "tga", "dds", "jxl", "qoi", // camera raw
    "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "raf", "srw", "pef",
];
const VIDEO: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpg", "mpeg", "ts", "mts", "m2ts",
    "3gp", "ogv", "mxf",
];
const AUDIO: &[&str] = &[
    "mp3", "wav", "flac", "ogg", "oga", "opus", "m4a", "aac", "aif", "aiff", "wma", "caf",
];
const VECTOR: &[&str] = &["svg", "svgz", "ai", "eps"];
const DOCUMENT: &[&str] = &["pdf"];

pub const PROJECT_EXTENSION: &str = "vellum";

pub fn classify(path: &Path) -> MediaKind {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return MediaKind::Unknown;
    };
    let extension = extension.to_ascii_lowercase();
    let is = |list: &[&str]| list.contains(&extension.as_str());
    if extension == PROJECT_EXTENSION {
        MediaKind::Project
    } else if is(IMAGE) {
        MediaKind::Image
    } else if is(VIDEO) {
        MediaKind::Video
    } else if is(AUDIO) {
        MediaKind::Audio
    } else if is(VECTOR) {
        MediaKind::Vector
    } else if is(DOCUMENT) {
        MediaKind::Document
    } else {
        MediaKind::Unknown
    }
}

/// the workspace a file of this kind opens in when nothing else is asked for
pub fn workspace_for(kind: MediaKind) -> Option<&'static str> {
    match kind {
        MediaKind::Image => Some("image"),
        MediaKind::Video => Some("video"),
        MediaKind::Audio => Some("audio"),
        MediaKind::Vector | MediaKind::Document => Some("vector"),
        MediaKind::Project | MediaKind::Unknown => None,
    }
}

/// the quick tools that take files, and what they take. the cards themselves
/// (titles, icons) live in the ui, these ids must match theirs
pub fn quick_tool_accepts(tool: &str) -> Option<&'static [MediaKind]> {
    use MediaKind::*;
    match tool {
        "export-to-size" => Some(&[Video, Image, Audio]),
        "video-to-gif" => Some(&[Video]),
        "remove-background" => Some(&[Image]),
        "convert" => Some(&[Image, Video, Audio, Vector]),
        _ => None,
    }
}

/// where something was dropped or asked to open
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum OpenTarget {
    /// pick the workspace from the file type
    #[default]
    Auto,
    /// a specific workspace, such as a file dropped on the video workspace
    Workspace { id: String },
    /// a quick tool card
    Tool { id: String },
}

/// what opening a set of files will do, before it happens. the drop overlay
/// shows it while the files hover, and the open command carries it out
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OpenPlan {
    pub opens: Vec<PlannedOpen>,
    pub skipped: Vec<SkippedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlannedOpen {
    pub path: String,
    pub name: String,
    pub kind: MediaKind,
    pub workspace: String,
    pub tool: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkippedFile {
    pub path: String,
    pub name: String,
    /// a sentence for the user
    pub reason: String,
}

pub fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

pub fn plan_open(paths: &[String], target: &OpenTarget) -> OpenPlan {
    let mut plan = OpenPlan::default();
    for path in paths {
        let kind = classify(Path::new(path));
        let name = file_name(path);
        match route(kind, target) {
            Ok((workspace, tool)) => plan.opens.push(PlannedOpen {
                path: path.clone(),
                name,
                kind,
                workspace: workspace.to_owned(),
                tool,
            }),
            Err(reason) => plan.skipped.push(SkippedFile {
                path: path.clone(),
                name,
                reason,
            }),
        }
    }
    plan
}

fn route(kind: MediaKind, target: &OpenTarget) -> Result<(String, Option<String>), String> {
    match target {
        OpenTarget::Auto => match workspace_for(kind) {
            Some(workspace) => Ok((workspace.to_owned(), None)),
            None => Err(unopenable(kind)),
        },
        OpenTarget::Workspace { id } => {
            if kind == MediaKind::Unknown || kind == MediaKind::Project {
                return Err(unopenable(kind));
            }
            Ok((id.clone(), None))
        }
        OpenTarget::Tool { id } => {
            let accepts = quick_tool_accepts(id)
                .ok_or_else(|| "That tool does not take files.".to_owned())?;
            if accepts.contains(&kind) {
                Ok(("quick-tools".to_owned(), Some(id.clone())))
            } else {
                Err(format!("This tool only takes {}.", describe_kinds(accepts)))
            }
        }
    }
}

fn unopenable(kind: MediaKind) -> String {
    match kind {
        MediaKind::Project => {
            "Projects can be opened once project files arrive in a later update.".to_owned()
        }
        _ => "Vellum does not know this file type yet.".to_owned(),
    }
}

fn describe_kinds(kinds: &[MediaKind]) -> String {
    let words: Vec<&str> = kinds
        .iter()
        .map(|kind| match kind {
            MediaKind::Image => "images",
            MediaKind::Video => "videos",
            MediaKind::Audio => "audio",
            MediaKind::Vector => "vector files",
            MediaKind::Document => "pdfs",
            MediaKind::Project => "projects",
            MediaKind::Unknown => "files",
        })
        .collect();
    match words.as_slice() {
        [] => "files".to_owned(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn extensions_are_matched_without_caring_about_case() {
        assert_eq!(classify(Path::new("/a/Holiday.JPG")), MediaKind::Image);
        assert_eq!(classify(Path::new("clip.mOv")), MediaKind::Video);
        assert_eq!(classify(Path::new("voice.flac")), MediaKind::Audio);
        assert_eq!(classify(Path::new("logo.svg")), MediaKind::Vector);
        assert_eq!(classify(Path::new("scan.pdf")), MediaKind::Document);
        assert_eq!(classify(Path::new("trip.vellum")), MediaKind::Project);
        assert_eq!(classify(Path::new("raw.CR3")), MediaKind::Image);
        assert_eq!(classify(Path::new("notes.txt")), MediaKind::Unknown);
        assert_eq!(classify(Path::new("Makefile")), MediaKind::Unknown);
    }

    #[test]
    fn auto_sends_each_file_to_its_own_workspace() {
        let plan = plan_open(
            &paths(&["/m/a.png", "/m/b.mp4", "/m/c.wav", "/m/d.pdf", "/m/e.txt"]),
            &OpenTarget::Auto,
        );
        let routed: Vec<_> = plan
            .opens
            .iter()
            .map(|open| (open.name.as_str(), open.workspace.as_str()))
            .collect();
        assert_eq!(
            routed,
            [
                ("a.png", "image"),
                ("b.mp4", "video"),
                ("c.wav", "audio"),
                ("d.pdf", "vector")
            ]
        );
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.skipped[0].name, "e.txt");
    }

    #[test]
    fn a_workspace_target_wins_over_the_file_type() {
        let plan = plan_open(
            &paths(&["frame.png"]),
            &OpenTarget::Workspace { id: "video".into() },
        );
        assert_eq!(plan.opens[0].workspace, "video");
    }

    #[test]
    fn a_tool_only_takes_what_it_can_use() {
        let gif = OpenTarget::Tool {
            id: "video-to-gif".into(),
        };
        let plan = plan_open(&paths(&["clip.mp4", "photo.jpg"]), &gif);
        assert_eq!(plan.opens.len(), 1);
        assert_eq!(plan.opens[0].workspace, "quick-tools");
        assert_eq!(plan.opens[0].tool.as_deref(), Some("video-to-gif"));
        assert_eq!(plan.skipped[0].reason, "This tool only takes videos.");

        let size = OpenTarget::Tool {
            id: "export-to-size".into(),
        };
        let plan = plan_open(&paths(&["doc.pdf"]), &size);
        assert_eq!(
            plan.skipped[0].reason,
            "This tool only takes videos, images and audio."
        );

        let record = OpenTarget::Tool {
            id: "screen-record".into(),
        };
        assert!(plan_open(&paths(&["a.png"]), &record).opens.is_empty());
    }
}
