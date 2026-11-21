//! batch rename: new names for many files from one pattern, shown before anything moves.
//!
//! a pattern like `holiday {n}` names the files `holiday 01.jpg`, `holiday 02.jpg` and so
//! on. only the name changes: files stay in their folder and keep their extension. a name
//! that would clash with another file, or with another new name in the batch, is flagged
//! in the preview and left alone, so a rename never replaces anything.
//!
//! the renaming itself is [`RenameFiles`], a quick tool command, so it lands on the undo
//! stack and ctrl+z puts the old names back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::command::{Command, CommandError, params_of};
use crate::quick_tool::QuickToolDocument;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NameCase {
    #[default]
    Keep,
    Lower,
    Upper,
}

/// how to build the new names, stored in the convert tool's options
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct RenamePattern {
    /// the new name without its extension. tags: {name} {n} {date} {folder} {ext}
    pub pattern: String,
    /// the number {n} starts at
    pub start: u32,
    /// {n} is padded with zeros to at least this many digits
    pub digits: u32,
    /// replaced in the old name before it goes into {name}
    pub find: String,
    pub replace: String,
    pub case: NameCase,
}

impl Default for RenamePattern {
    fn default() -> Self {
        Self {
            pattern: "{name}".to_owned(),
            start: 1,
            digits: 2,
            find: String::new(),
            replace: String::new(),
            case: NameCase::Keep,
        }
    }
}

pub const TAGS: [&str; 5] = ["name", "n", "date", "folder", "ext"];

/// one file in the preview
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameRow {
    pub from: String,
    pub to: String,
    /// why this file will be skipped, if it will
    pub problem: Option<String>,
}

impl RenameRow {
    pub fn changes(&self) -> bool {
        self.problem.is_none() && self.from != self.to
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenamePlan {
    pub rows: Vec<RenameRow>,
    /// set when the pattern itself is wrong, eg an unknown tag. nothing can run then.
    pub error: Option<String>,
}

/// works out the new name of every file without touching any of them
pub fn plan(inputs: &[PathBuf], pattern: &RenamePattern) -> RenamePlan {
    if let Err(error) = check_pattern(&pattern.pattern) {
        return RenamePlan {
            rows: inputs
                .iter()
                .map(|p| RenameRow {
                    from: p.display().to_string(),
                    to: p.display().to_string(),
                    problem: None,
                })
                .collect(),
            error: Some(error),
        };
    }
    let mut rows: Vec<RenameRow> = inputs
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let number = u64::from(pattern.start) + i as u64;
            match new_name(path, number, pattern) {
                Ok(to) => RenameRow {
                    from: path.display().to_string(),
                    to: to.display().to_string(),
                    problem: None,
                },
                Err(problem) => RenameRow {
                    from: path.display().to_string(),
                    to: path.display().to_string(),
                    problem: Some(problem),
                },
            }
        })
        .collect();

    // two files may not end up with one name. names are compared ignoring case because
    // windows and macos treat "A.jpg" and "a.jpg" as the same file.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for row in &rows {
        if row.problem.is_none() {
            *seen.entry(row.to.to_lowercase()).or_default() += 1;
        }
    }
    let sources: Vec<String> = inputs
        .iter()
        .map(|p| p.display().to_string().to_lowercase())
        .collect();
    for row in &mut rows {
        if !row.changes() {
            continue;
        }
        let key = row.to.to_lowercase();
        let name = file_name(Path::new(&row.to));
        if seen.get(&key).copied().unwrap_or(0) > 1 {
            row.problem = Some(format!(
                "Another file in the list would also be called {name}."
            ));
        } else if Path::new(&row.to).exists() && !sources.contains(&key) {
            row.problem = Some(format!("A file called {name} is already there."));
        }
    }
    RenamePlan { rows, error: None }
}

fn check_pattern(pattern: &str) -> Result<(), String> {
    if pattern.trim().is_empty() {
        return Err("Type a pattern for the new names, like {name} or holiday {n}.".to_owned());
    }
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return Err("A tag is missing its closing }.".to_owned());
        };
        let tag = &after[..close];
        if !TAGS.contains(&tag) {
            return Err(format!(
                "{{{tag}}} is not a tag. Use {{name}}, {{n}}, {{date}}, {{folder}} or {{ext}}."
            ));
        }
        rest = &after[close + 1..];
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn new_name(path: &Path, number: u64, pattern: &RenamePattern) -> Result<PathBuf, String> {
    let parent = path.parent().unwrap_or(Path::new(""));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = if pattern.find.is_empty() {
        stem
    } else {
        stem.replace(&pattern.find, &pattern.replace)
    };
    let digits = pattern.digits.clamp(1, 9) as usize;
    let folder = parent
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let date = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(date_label)
        .unwrap_or_default();

    let rendered = pattern
        .pattern
        .replace("{name}", &name)
        .replace("{n}", &format!("{number:0digits$}"))
        .replace("{date}", &date)
        .replace("{folder}", &folder)
        .replace("{ext}", &ext);
    let rendered = match pattern.case {
        NameCase::Keep => rendered,
        NameCase::Lower => rendered.to_lowercase(),
        NameCase::Upper => rendered.to_uppercase(),
    };
    let clean = clean_name(&rendered)?;
    let full = if ext.is_empty() {
        clean
    } else {
        format!("{clean}.{ext}")
    };
    Ok(parent.join(full))
}

/// swaps characters no file system allows for an underscore and refuses names windows
/// keeps for itself
fn clean_name(name: &str) -> Result<String, String> {
    let swapped: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // windows drops trailing dots and spaces on its own, which would surprise people
    let clean = swapped.trim().trim_end_matches('.').trim_end().to_owned();
    if clean.is_empty() {
        return Err("The new name would be empty.".to_owned());
    }
    let upper = clean.to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        return Err(format!("Windows does not allow a file called {clean}."));
    }
    if clean.chars().count() > 240 {
        return Err("The new name would be too long.".to_owned());
    }
    Ok(clean)
}

/// "2026-11-21". this is the date in utc, which can be a day off from the clock on the
/// wall late at night; good enough to sort holiday photos by.
fn date_label(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_date((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// days since 1970-01-01 to a calendar date, after howard hinnant's `civil_from_days`
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// one file to rename, from and to the full path
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileRename {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// renames files in a quick tool's list on disk and points the list at the new names.
/// only files in the list can be renamed and they stay in their folder, so a command sent
/// from anywhere cannot move or rename other files.
#[derive(Debug, Serialize, Deserialize)]
pub struct RenameFiles {
    pub renames: Vec<FileRename>,
}

impl RenameFiles {
    pub const KIND: &'static str = "tool.rename_files";

    fn check(&self, doc: &QuickToolDocument) -> Result<(), CommandError> {
        if self.renames.is_empty() {
            return Err(CommandError::failed("no file needs a new name"));
        }
        let sources: Vec<&PathBuf> = self.renames.iter().map(|r| &r.from).collect();
        for rename in &self.renames {
            if !doc.inputs.contains(&rename.from) {
                return Err(CommandError::failed(format!(
                    "{} is not in the list",
                    rename.from.display()
                )));
            }
            let name = rename.to.file_name().map(|n| n.to_string_lossy());
            let same_folder = rename.to.parent() == rename.from.parent();
            if !same_folder || name.is_none_or(|n| clean_name(&n).as_deref() != Ok(&*n)) {
                return Err(CommandError::failed(format!(
                    "{} is not a new name in the same folder",
                    rename.to.display()
                )));
            }
            // a case only change finds the file itself on windows and macos
            let taken = rename.to.exists()
                && !sources.iter().any(|s| {
                    s.display()
                        .to_string()
                        .eq_ignore_ascii_case(&rename.to.display().to_string())
                });
            if taken {
                return Err(CommandError::failed(format!(
                    "{} is already there",
                    rename.to.display()
                )));
            }
        }
        Ok(())
    }
}

/// renames every pair, by way of a temporary name so files can swap names and case only
/// changes work everywhere. if any step fails, every file gets its old name back.
fn rename_all(pairs: &[(PathBuf, PathBuf)]) -> Result<(), CommandError> {
    let mut parked: Vec<(PathBuf, PathBuf)> = Vec::new();
    let undo_parked = |parked: &[(PathBuf, PathBuf)]| {
        for (from, temp) in parked.iter().rev() {
            let _ = std::fs::rename(temp, from);
        }
    };
    for (i, (from, _)) in pairs.iter().enumerate() {
        let temp = temp_name(from, i);
        if let Err(err) = std::fs::rename(from, &temp) {
            undo_parked(&parked);
            return Err(CommandError::failed(format!(
                "{} could not be renamed: {err}",
                from.display()
            )));
        }
        parked.push((from.clone(), temp));
    }
    let mut placed: Vec<(PathBuf, PathBuf)> = Vec::new();
    for ((from, temp), (_, to)) in parked.iter().zip(pairs) {
        // checked again here: something else may have made the file since the preview
        let result = if to.exists() {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "a file with that name appeared",
            ))
        } else {
            std::fs::rename(temp, to)
        };
        if let Err(err) = result {
            for (to, temp) in placed.iter().rev() {
                let _ = std::fs::rename(to, temp);
            }
            undo_parked(&parked);
            return Err(CommandError::failed(format!(
                "{} could not be renamed: {err}",
                from.display()
            )));
        }
        placed.push((to.clone(), temp.clone()));
    }
    Ok(())
}

fn temp_name(path: &Path, index: usize) -> PathBuf {
    let name = file_name(path);
    let parent = path.parent().unwrap_or(Path::new(""));
    let mut n = 0;
    loop {
        let candidate = parent.join(format!(".{name}.{index}.{n}.vellum-rename"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn swap_inputs(doc: &mut QuickToolDocument, pairs: &[(PathBuf, PathBuf)]) {
    for input in &mut doc.inputs {
        if let Some((_, to)) = pairs.iter().find(|(from, _)| from == input) {
            input.clone_from(to);
        }
    }
}

impl Command<QuickToolDocument> for RenameFiles {
    fn kind(&self) -> &'static str {
        Self::KIND
    }

    fn name(&self) -> String {
        match self.renames.len() {
            1 => "Rename a file".to_owned(),
            n => format!("Rename {n} files"),
        }
    }

    fn apply(&mut self, doc: &mut QuickToolDocument) -> Result<(), CommandError> {
        self.check(doc)?;
        let pairs: Vec<(PathBuf, PathBuf)> = self
            .renames
            .iter()
            .map(|r| (r.from.clone(), r.to.clone()))
            .collect();
        rename_all(&pairs)?;
        swap_inputs(doc, &pairs);
        Ok(())
    }

    fn revert(&mut self, doc: &mut QuickToolDocument) -> Result<(), CommandError> {
        let pairs: Vec<(PathBuf, PathBuf)> = self
            .renames
            .iter()
            .map(|r| (r.to.clone(), r.from.clone()))
            .collect();
        rename_all(&pairs)?;
        swap_inputs(doc, &pairs);
        Ok(())
    }

    fn params(&self) -> Result<Value, CommandError> {
        params_of(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandRecord;
    use crate::documents::DocumentModel;
    use crate::history::{History, HistoryLimits};

    fn files(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|n| {
                let path = dir.join(n);
                std::fs::write(&path, n.as_bytes()).unwrap();
                path
            })
            .collect()
    }

    fn names(plan: &RenamePlan) -> Vec<String> {
        plan.rows
            .iter()
            .map(|r| file_name(Path::new(&r.to)))
            .collect()
    }

    #[test]
    fn patterns_number_and_keep_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = files(dir.path(), &["IMG_1.JPG", "IMG_2.jpg", "notes"]);
        let pattern = RenamePattern {
            pattern: "holiday {n} ({name})".into(),
            start: 9,
            ..RenamePattern::default()
        };
        let plan = plan(&inputs, &pattern);
        assert_eq!(plan.error, None);
        assert_eq!(
            names(&plan),
            [
                "holiday 09 (IMG_1).JPG",
                "holiday 10 (IMG_2).jpg",
                "holiday 11 (notes)"
            ]
        );
        assert!(plan.rows.iter().all(RenameRow::changes));
    }

    #[test]
    fn find_replace_case_and_bad_characters() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = files(dir.path(), &["My Clip.mp4"]);
        let pattern = RenamePattern {
            pattern: "{name}: {ext}?".into(),
            find: " ".into(),
            replace: "_".into(),
            case: NameCase::Lower,
            ..RenamePattern::default()
        };
        assert_eq!(names(&plan(&inputs, &pattern)), ["my_clip_ mp4_.mp4"]);
    }

    #[test]
    fn clashes_are_flagged_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = files(dir.path(), &["a.png", "b.png", "c.jpg"]);
        files(dir.path(), &["taken.jpg"]);
        let same = plan(
            &inputs[..2],
            &RenamePattern {
                pattern: "same".into(),
                ..RenamePattern::default()
            },
        );
        assert!(same.rows.iter().all(|r| r.problem.is_some()));
        let taken = plan(
            &inputs[2..],
            &RenamePattern {
                pattern: "taken".into(),
                ..RenamePattern::default()
            },
        );
        assert!(
            taken.rows[0]
                .problem
                .as_deref()
                .unwrap()
                .contains("already there")
        );
        // swapping names inside the batch is fine
        let swap = plan(
            &[inputs[1].clone(), inputs[0].clone()],
            &RenamePattern {
                pattern: "{n}".into(),
                digits: 1,
                ..RenamePattern::default()
            },
        );
        assert!(swap.rows.iter().all(RenameRow::changes));
    }

    #[test]
    fn unknown_tags_and_empty_names_are_explained() {
        let inputs = vec![PathBuf::from("x.png")];
        let bad = plan(
            &inputs,
            &RenamePattern {
                pattern: "{nmae}".into(),
                ..RenamePattern::default()
            },
        );
        assert!(bad.error.unwrap().contains("{nmae} is not a tag"));
        assert!(
            plan(
                &inputs,
                &RenamePattern {
                    pattern: "{x".into(),
                    ..RenamePattern::default()
                }
            )
            .error
            .is_some()
        );
        assert!(clean_name(" .. ").is_err());
        assert!(clean_name("nul").is_err());
        assert!(clean_name("com1").is_err());
        assert_eq!(clean_name("com").unwrap(), "com");
    }

    #[test]
    fn dates_are_calendar_dates() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(20_413), (2025, 11, 21));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(date_label(UNIX_EPOCH), "1970-01-01");
    }

    fn rename_command(renames: Vec<FileRename>) -> Box<dyn Command<QuickToolDocument>> {
        QuickToolDocument::commands()
            .instantiate(&CommandRecord {
                kind: RenameFiles::KIND.into(),
                params: serde_json::json!({ "renames": renames }),
            })
            .unwrap()
    }

    #[test]
    fn renaming_moves_files_and_undo_puts_them_back() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = files(dir.path(), &["a.png", "b.png"]);
        let mut doc = QuickToolDocument::new("Convert", inputs.clone());
        let mut history = History::new(HistoryLimits::default());
        // a and b swap names, which needs the temporary step
        let renames = vec![
            FileRename {
                from: inputs[0].clone(),
                to: inputs[1].clone(),
            },
            FileRename {
                from: inputs[1].clone(),
                to: inputs[0].clone(),
            },
        ];
        history.execute(rename_command(renames), &mut doc).unwrap();
        assert_eq!(std::fs::read(&inputs[0]).unwrap(), b"b.png");
        assert_eq!(std::fs::read(&inputs[1]).unwrap(), b"a.png");
        assert_eq!(doc.inputs, [inputs[1].clone(), inputs[0].clone()]);
        assert_eq!(
            history.summary().undo_name.as_deref(),
            Some("Rename 2 files")
        );

        history.undo(&mut doc).unwrap();
        assert_eq!(std::fs::read(&inputs[0]).unwrap(), b"a.png");
        assert_eq!(doc.inputs, inputs);
        // nothing is left behind
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn files_outside_the_list_or_folder_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let inputs = files(dir.path(), &["a.png", "b.png"]);
        let outsider = files(dir.path(), &["secret.txt"]);
        let mut doc = QuickToolDocument::new("Convert", inputs.clone());
        let mut history = History::new(HistoryLimits::default());
        for rename in [
            FileRename {
                from: outsider[0].clone(),
                to: dir.path().join("x.txt"),
            },
            FileRename {
                from: inputs[0].clone(),
                to: other.path().join("a.png"),
            },
            FileRename {
                from: inputs[0].clone(),
                to: inputs[1].clone(),
            },
            FileRename {
                from: inputs[0].clone(),
                to: dir.path().join("bad?.png"),
            },
        ] {
            assert!(
                history
                    .execute(rename_command(vec![rename]), &mut doc)
                    .is_err()
            );
        }
        assert!(inputs.iter().all(|p| p.exists()));
        assert_eq!(doc.inputs, inputs);
    }

    #[test]
    fn a_failed_step_puts_every_name_back() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = files(dir.path(), &["a.png", "b.png"]);
        let missing = dir.path().join("gone.png");
        // the second source is not there, so its rename fails after the first one parked
        let pairs = vec![
            (inputs[0].clone(), dir.path().join("x.png")),
            (missing, dir.path().join("y.png")),
        ];
        assert!(rename_all(&pairs).is_err());
        assert!(inputs[0].exists());
        assert!(!dir.path().join("x.png").exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
