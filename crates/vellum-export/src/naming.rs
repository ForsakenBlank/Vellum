//! where results go and what they are called. nothing a quick tool writes ever replaces an
//! existing file: `clip.mp4` gives `clip (10 MB).mp4`, then `clip (10 MB 2).mp4`.
//!
//! a batch can run several jobs at once into one folder, and two of them might pick the
//! same free name before either has written anything. so a chosen name is reserved until
//! its [`Reserved`] guard is dropped, and other jobs skip it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;

fn reserved() -> &'static Mutex<HashSet<PathBuf>> {
    static RESERVED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    RESERVED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// a free output path, held for this job until dropped
#[derive(Debug)]
pub struct Reserved {
    path: PathBuf,
}

impl Reserved {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        reserved().lock().remove(&self.path);
    }
}

/// picks a free path for a result of `source`: in `folder` if given, otherwise next to the
/// source, named after it with `label` in brackets and the extension `ext`
pub fn output_for(
    source: &Path,
    folder: Option<&Path>,
    label: Option<&str>,
    ext: &str,
) -> Reserved {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let dir = folder
        .map(Path::to_path_buf)
        .or_else(|| source.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let ext = ext.trim_start_matches('.');
    let name = |n: u32| match (label, n) {
        (Some(label), 1) => format!("{stem} ({label}).{ext}"),
        (Some(label), n) => format!("{stem} ({label} {n}).{ext}"),
        (None, 1) => format!("{stem}.{ext}"),
        (None, n) => format!("{stem} ({n}).{ext}"),
    };
    let mut taken = reserved().lock();
    let mut n = 1;
    loop {
        let candidate = dir.join(name(n));
        if candidate != source && !candidate.exists() && !taken.contains(&candidate) {
            taken.insert(candidate.clone());
            return Reserved { path: candidate };
        }
        n += 1;
    }
}

/// a free folder for many results, like extracted frames: `clip frames`, `clip frames 2`
pub fn folder_for(source: &Path, folder: Option<&Path>, label: &str) -> Reserved {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let dir = folder
        .map(Path::to_path_buf)
        .or_else(|| source.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let mut taken = reserved().lock();
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem} {label}")
        } else {
            format!("{stem} {label} {n}")
        };
        let candidate = dir.join(name);
        if !candidate.exists() && !taken.contains(&candidate) {
            taken.insert(candidate.clone());
            return Reserved { path: candidate };
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("clip.mp4");
        std::fs::write(&source, b"x").unwrap();

        let first = output_for(&source, None, Some("10 MB"), "mp4");
        assert_eq!(first.path(), dir.path().join("clip (10 MB).mp4"));
        // reserved but not written yet, a second job must not take it
        let second = output_for(&source, None, Some("10 MB"), "mp4");
        assert_eq!(second.path(), dir.path().join("clip (10 MB 2).mp4"));
        drop(first);
        let again = output_for(&source, None, Some("10 MB"), "mp4");
        assert_eq!(again.path(), dir.path().join("clip (10 MB).mp4"));
    }

    #[test]
    fn a_plain_conversion_never_points_at_its_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.png");
        std::fs::write(&source, b"x").unwrap();
        let out = output_for(&source, None, None, "png");
        assert_eq!(out.path(), dir.path().join("photo (2).png"));
        let jpg = output_for(&source, None, None, ".jpg");
        assert_eq!(jpg.path(), dir.path().join("photo.jpg"));
    }

    #[test]
    fn results_can_go_to_another_folder() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let source = dir.path().join("song.wav");
        let result = output_for(&source, Some(out.path()), None, "mp3");
        assert_eq!(result.path(), out.path().join("song.mp3"));
        let frames = folder_for(&source, None, "frames");
        assert_eq!(frames.path(), dir.path().join("song frames"));
    }
}
