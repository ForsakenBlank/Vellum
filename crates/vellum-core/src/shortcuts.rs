//! keyboard shortcuts: chord parsing, presets for people coming from other apps, and
//! conflict detection.
//!
//! a binding maps a command kind to the chords that run it. the effective keymap is the
//! preset's bindings with the user's overrides on top. chords are stored in one canonical
//! form (`Ctrl+Alt+Shift+Meta+Key`) so comparing two of them is plain string equality.
//!
//! keys are physical keys, named after the us layout, so `Ctrl+Z` stays in the same place
//! on an azerty keyboard. that is what photoshop and resolve do too.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::Event;

pub type Bindings = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ShortcutPreset {
    #[default]
    Vellum,
    Photoshop,
    Premiere,
    Resolve,
    Audacity,
}

impl ShortcutPreset {
    pub const ALL: [ShortcutPreset; 5] = [
        Self::Vellum,
        Self::Photoshop,
        Self::Premiere,
        Self::Resolve,
        Self::Audacity,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ShortcutSettings {
    pub preset: ShortcutPreset,
    /// the user's changes on top of the preset. an empty list removes every chord.
    pub overrides: Bindings,
}

/// two or more commands that share a chord where both can fire
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ShortcutConflict {
    pub chord: String,
    pub commands: Vec<String>,
}

/// the shortcuts in effect right now, sent to the ui whenever they change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Keymap {
    pub preset: ShortcutPreset,
    pub bindings: Bindings,
    pub conflicts: Vec<ShortcutConflict>,
}

impl Event for Keymap {
    const NAME: &'static str = "keymap:changed";
}

impl Keymap {
    pub fn from_settings(settings: &ShortcutSettings) -> Self {
        let mut bindings = preset_bindings(settings.preset);
        for (kind, chords) in &settings.overrides {
            let chords: Vec<String> = chords.iter().filter_map(|c| normalize_chord(c)).collect();
            bindings.insert(kind.clone(), chords);
        }
        bindings.retain(|_, chords| !chords.is_empty());
        let conflicts = find_conflicts(&bindings);
        Self {
            preset: settings.preset,
            bindings,
            conflicts,
        }
    }

    /// the commands bound to a chord, in any context
    pub fn commands_for(&self, chord: &str) -> Vec<&str> {
        self.bindings
            .iter()
            .filter(|(_, chords)| chords.iter().any(|c| c == chord))
            .map(|(kind, _)| kind.as_str())
            .collect()
    }
}

const MODIFIERS: [&str; 4] = ["Ctrl", "Alt", "Shift", "Meta"];

/// turns user or file input like `ctrl+shift+z` or `Cmd+K` into the canonical form
/// `Ctrl+Shift+Z`. returns none if it is not exactly one key plus any modifiers.
pub fn normalize_chord(text: &str) -> Option<String> {
    let mut mods = [false; 4];
    let mut key: Option<String> = None;
    for part in text.split('+').map(str::trim) {
        if part.is_empty() {
            return None;
        }
        let modifier = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(0),
            "alt" | "option" | "opt" => Some(1),
            "shift" => Some(2),
            "meta" | "cmd" | "command" | "super" | "win" => Some(3),
            _ => None,
        };
        match modifier {
            Some(index) => mods[index] = true,
            None if key.is_none() => key = Some(normalize_key(part)?),
            None => return None,
        }
    }
    let key = key?;
    let mut out: Vec<&str> = MODIFIERS
        .iter()
        .zip(mods)
        .filter(|(_, on)| *on)
        .map(|(name, _)| *name)
        .collect();
    out.push(&key);
    Some(out.join("+"))
}

fn normalize_key(key: &str) -> Option<String> {
    const NAMED: &[(&str, &str)] = &[
        ("enter", "Enter"),
        ("return", "Enter"),
        ("escape", "Escape"),
        ("esc", "Escape"),
        ("tab", "Tab"),
        ("space", "Space"),
        ("backspace", "Backspace"),
        ("delete", "Delete"),
        ("del", "Delete"),
        ("insert", "Insert"),
        ("home", "Home"),
        ("end", "End"),
        ("pageup", "PageUp"),
        ("pagedown", "PageDown"),
        ("up", "Up"),
        ("arrowup", "Up"),
        ("down", "Down"),
        ("arrowdown", "Down"),
        ("left", "Left"),
        ("arrowleft", "Left"),
        ("right", "Right"),
        ("arrowright", "Right"),
    ];
    const PUNCTUATION: &str = ",./;'[]\\-=`";

    let lower = key.to_ascii_lowercase();
    if let Some((_, name)) = NAMED.iter().find(|(alias, _)| *alias == lower) {
        return Some((*name).to_owned());
    }
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
        if PUNCTUATION.contains(c) {
            return Some(c.to_string());
        }
        return None;
    }
    // f1 to f24
    let number = lower.strip_prefix('f')?.parse::<u8>().ok()?;
    (1..=24).contains(&number).then(|| format!("F{number}"))
}

/// the part of the app a command belongs to. commands in different workspaces may share a
/// chord, and a workspace's own command may take a global command's chord: inside that
/// workspace it wins, which is how ctrl+e merges layers in the image editor and exports to
/// a size everywhere else.
pub fn context_of(kind: &str) -> &str {
    const WORKSPACES: [&str; 7] = [
        "quick_tools",
        "image",
        "video",
        "color",
        "audio",
        "motion",
        "vector",
    ];
    let first = kind.split('.').next().unwrap_or_default();
    if WORKSPACES.contains(&first) {
        first
    } else {
        "global"
    }
}

/// true when two commands would fight over the same keys
fn contexts_overlap(a: &str, b: &str) -> bool {
    a == b
}

/// every chord used by two or more commands that can fire at the same time
pub fn find_conflicts(bindings: &Bindings) -> Vec<ShortcutConflict> {
    let mut by_chord: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (kind, chords) in bindings {
        for chord in chords {
            let list = by_chord.entry(chord.as_str()).or_default();
            if !list.contains(&kind.as_str()) {
                list.push(kind.as_str());
            }
        }
    }
    let mut conflicts = Vec::new();
    for (chord, kinds) in by_chord {
        let clashing: Vec<String> = kinds
            .iter()
            .filter(|kind| {
                kinds.iter().any(|other| {
                    other != *kind && contexts_overlap(context_of(kind), context_of(other))
                })
            })
            .map(|kind| (*kind).to_owned())
            .collect();
        if clashing.len() > 1 {
            conflicts.push(ShortcutConflict {
                chord: chord.to_owned(),
                commands: clashing,
            });
        }
    }
    conflicts
}

/// normalises every chord in the settings and drops the ones that cannot be parsed
pub fn sanitize(settings: &mut ShortcutSettings) {
    for chords in settings.overrides.values_mut() {
        let mut clean: Vec<String> = Vec::new();
        for chord in chords.iter().filter_map(|c| normalize_chord(c)) {
            if !clean.contains(&chord) {
                clean.push(chord);
            }
        }
        *chords = clean;
    }
    settings.overrides.retain(|kind, _| !kind.trim().is_empty());
}

type Table = &'static [(&'static str, &'static [&'static str])];

/// vellum's own shortcuts. the other presets start from these and change what differs.
const VELLUM: Table = &[
    ("app.command_palette", &["Ctrl+K"]),
    ("app.settings", &["Ctrl+,"]),
    ("app.shortcuts", &["Ctrl+Alt+K"]),
    ("app.home", &["Ctrl+Shift+H"]),
    ("file.open", &["Ctrl+O"]),
    ("file.save", &["Ctrl+S"]),
    ("file.close", &["Ctrl+W"]),
    ("edit.undo", &["Ctrl+Z"]),
    ("edit.redo", &["Ctrl+Shift+Z", "Ctrl+Y"]),
    ("window.next_tab", &["Ctrl+Tab"]),
    ("window.previous_tab", &["Ctrl+Shift+Tab"]),
    ("view.docks.toggle", &["Ctrl+\\"]),
    ("view.panel.jobs", &["Ctrl+Shift+J"]),
    ("view.zoom.in", &["Ctrl+="]),
    ("view.zoom.out", &["Ctrl+-"]),
    ("view.zoom.reset", &["Ctrl+0"]),
    ("workspace.quick_tools", &["Ctrl+1"]),
    ("workspace.image", &["Ctrl+2"]),
    ("workspace.video", &["Ctrl+3"]),
    ("workspace.color", &["Ctrl+4"]),
    ("workspace.audio", &["Ctrl+5"]),
    ("workspace.motion", &["Ctrl+6"]),
    ("workspace.vector", &["Ctrl+7"]),
    ("tool.export_to_size", &["Ctrl+E"]),
    ("tool.screen_record", &["Ctrl+Shift+R"]),
    ("file.new", &["Ctrl+N"]),
    ("file.save_as", &["Ctrl+Shift+S"]),
    // the image editor follows photoshop out of the box
    ("image.export", &["Ctrl+Alt+Shift+W"]),
    ("image.tool.move", &["V"]),
    ("image.tool.marquee", &["M"]),
    ("image.tool.marquee_cycle", &["Shift+M"]),
    ("image.tool.lasso", &["L"]),
    ("image.tool.lasso_cycle", &["Shift+L"]),
    ("image.tool.wand", &["W"]),
    ("image.tool.wand_cycle", &["Shift+W"]),
    ("image.tool.crop", &["C"]),
    ("image.tool.crop_cycle", &["Shift+C"]),
    ("image.tool.eyedropper", &["I"]),
    ("image.tool.brush", &["B"]),
    ("image.tool.brush_cycle", &["Shift+B"]),
    ("image.tool.eraser", &["E"]),
    ("image.tool.gradient", &["G"]),
    ("image.tool.gradient_cycle", &["Shift+G"]),
    ("image.tool.hand", &["H"]),
    ("image.tool.zoom", &["Z"]),
    ("image.transform", &["Ctrl+T"]),
    ("image.select.all", &["Ctrl+A"]),
    ("image.select.deselect", &["Ctrl+D"]),
    ("image.select.reselect", &["Ctrl+Shift+D"]),
    ("image.select.invert", &["Ctrl+Shift+I"]),
    ("image.select.feather", &["Shift+F6"]),
    ("image.quick_mask", &["Q"]),
    ("image.colors.default", &["D"]),
    ("image.colors.swap", &["X"]),
    ("image.brush.smaller", &["["]),
    ("image.brush.bigger", &["]"]),
    ("image.brush.softer", &["Shift+["]),
    ("image.brush.harder", &["Shift+]"]),
    ("image.edit.clear", &["Delete", "Backspace"]),
    ("image.edit.fill_foreground", &["Alt+Backspace"]),
    ("image.edit.fill_background", &["Ctrl+Backspace"]),
    ("image.layer.new", &["Ctrl+Shift+N"]),
    ("image.layer.via_copy", &["Ctrl+J"]),
    ("image.layer.via_cut", &["Ctrl+Shift+J"]),
    ("image.layer.group", &["Ctrl+G"]),
    ("image.layer.ungroup", &["Ctrl+Shift+G"]),
    ("image.layer.clip", &["Ctrl+Alt+G"]),
    ("image.layer.merge_down", &["Ctrl+E"]),
    ("image.layer.merge_visible", &["Ctrl+Shift+E"]),
    ("image.view.rulers", &["Ctrl+R"]),
    ("image.view.grid", &["Ctrl+'"]),
    ("image.view.guides", &["Ctrl+;"]),
    ("image.view.snap", &["Ctrl+Shift+;"]),
    ("image.view.lock_guides", &["Ctrl+Alt+;"]),
    ("image.view.fit", &["Ctrl+0"]),
    ("image.view.actual_pixels", &["Ctrl+1"]),
    ("image.view.zoom_in", &["Ctrl+="]),
    ("image.view.zoom_out", &["Ctrl+-"]),
    ("image.canvas_size", &["Ctrl+Alt+C"]),
    ("image.image_size", &["Ctrl+Alt+I"]),
];

/// photoshop opens preferences with ctrl+k, exports with ctrl+alt+shift+w and hides
/// panels with tab
const PHOTOSHOP: Table = &[
    ("app.settings", &["Ctrl+K"]),
    ("app.command_palette", &["Ctrl+Shift+P"]),
    ("tool.export_to_size", &["Ctrl+Alt+Shift+W"]),
    ("view.docks.toggle", &["Tab"]),
    ("edit.redo", &["Ctrl+Shift+Z"]),
];

/// premiere exports with ctrl+m and switches workspaces with alt+shift+number
const PREMIERE: Table = &[
    ("tool.export_to_size", &["Ctrl+M"]),
    ("edit.redo", &["Ctrl+Shift+Z"]),
    ("workspace.video", &["Alt+Shift+1"]),
    ("workspace.color", &["Alt+Shift+2"]),
    ("workspace.audio", &["Alt+Shift+3"]),
    ("workspace.motion", &["Alt+Shift+4"]),
    ("workspace.quick_tools", &["Alt+Shift+5"]),
    ("workspace.image", &["Alt+Shift+6"]),
    ("workspace.vector", &["Alt+Shift+7"]),
];

/// resolve switches pages with shift+number: media 2, cut 3, edit 4, fusion 5, color 6,
/// fairlight 7, deliver 8
const RESOLVE: Table = &[
    ("edit.redo", &["Ctrl+Shift+Z"]),
    ("workspace.image", &["Shift+2"]),
    ("workspace.vector", &["Shift+3"]),
    ("workspace.video", &["Shift+4"]),
    ("workspace.motion", &["Shift+5"]),
    ("workspace.color", &["Shift+6"]),
    ("workspace.audio", &["Shift+7"]),
    ("tool.export_to_size", &["Shift+8"]),
];

/// audacity redoes with ctrl+y, opens preferences with ctrl+p and exports with ctrl+shift+e
const AUDACITY: Table = &[
    ("edit.redo", &["Ctrl+Y", "Ctrl+Shift+Z"]),
    ("app.settings", &["Ctrl+P"]),
    ("tool.export_to_size", &["Ctrl+Shift+E"]),
];

pub fn preset_bindings(preset: ShortcutPreset) -> Bindings {
    let mut bindings = table_to_bindings(VELLUM);
    let changes = match preset {
        ShortcutPreset::Vellum => return bindings,
        ShortcutPreset::Photoshop => PHOTOSHOP,
        ShortcutPreset::Premiere => PREMIERE,
        ShortcutPreset::Resolve => RESOLVE,
        ShortcutPreset::Audacity => AUDACITY,
    };
    for (kind, chords) in table_to_bindings(changes) {
        // a chord the preset gives to a command is taken away from whatever had it before
        // in the same context
        for (other, existing) in bindings.iter_mut() {
            if *other != kind && contexts_overlap(context_of(other), context_of(&kind)) {
                existing.retain(|c| !chords.contains(c));
            }
        }
        bindings.insert(kind, chords);
    }
    bindings.retain(|_, chords| !chords.is_empty());
    bindings
}

fn table_to_bindings(table: Table) -> Bindings {
    table
        .iter()
        .map(|(kind, chords)| {
            let chords = chords.iter().filter_map(|c| normalize_chord(c)).collect();
            ((*kind).to_owned(), chords)
        })
        .collect()
}

/// the kinds every preset mentions, so a test can check they are all registered
pub fn kinds_in_presets() -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = [VELLUM, PHOTOSHOP, PREMIERE, RESOLVE, AUDACITY]
        .iter()
        .flat_map(|table| table.iter().map(|(kind, _)| *kind))
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_are_normalised() {
        let n = |s: &str| normalize_chord(s);
        assert_eq!(n("ctrl+shift+z").as_deref(), Some("Ctrl+Shift+Z"));
        assert_eq!(n("Shift + Ctrl + Z").as_deref(), Some("Ctrl+Shift+Z"));
        assert_eq!(n("cmd+k").as_deref(), Some("Meta+K"));
        assert_eq!(n("Option+Esc").as_deref(), Some("Alt+Escape"));
        assert_eq!(n("ctrl+,").as_deref(), Some("Ctrl+,"));
        assert_eq!(n("f12").as_deref(), Some("F12"));
        assert_eq!(n("ArrowUp").as_deref(), Some("Up"));
        assert_eq!(n("tab").as_deref(), Some("Tab"));
    }

    #[test]
    fn bad_chords_are_rejected() {
        for bad in [
            "",
            "ctrl",
            "ctrl+shift",
            "a+b",
            "ctrl++",
            "f25",
            "f0",
            "ctrl+é",
            "hyper+x",
        ] {
            assert_eq!(normalize_chord(bad), None, "{bad} should be rejected");
        }
    }

    #[test]
    fn every_built_in_chord_is_canonical() {
        for table in [VELLUM, PHOTOSHOP, PREMIERE, RESOLVE, AUDACITY] {
            for (kind, chords) in table {
                for chord in *chords {
                    assert_eq!(
                        normalize_chord(chord).as_deref(),
                        Some(*chord),
                        "{kind} has a chord in a non canonical form"
                    );
                }
            }
        }
    }

    #[test]
    fn presets_have_no_conflicts() {
        for preset in ShortcutPreset::ALL {
            let keymap = Keymap::from_settings(&ShortcutSettings {
                preset,
                overrides: Bindings::new(),
            });
            assert!(
                keymap.conflicts.is_empty(),
                "{preset:?} has conflicts: {:?}",
                keymap.conflicts
            );
        }
    }

    #[test]
    fn presets_change_what_differs() {
        let ps = preset_bindings(ShortcutPreset::Photoshop);
        assert_eq!(ps["app.settings"], ["Ctrl+K"]);
        assert_eq!(ps["app.command_palette"], ["Ctrl+Shift+P"]);
        assert_eq!(ps["edit.undo"], ["Ctrl+Z"]);

        let resolve = preset_bindings(ShortcutPreset::Resolve);
        assert_eq!(resolve["workspace.color"], ["Shift+6"]);
        assert_eq!(resolve["workspace.quick_tools"], ["Ctrl+1"]);
        assert_eq!(resolve["tool.export_to_size"], ["Shift+8"]);

        let audacity = preset_bindings(ShortcutPreset::Audacity);
        assert_eq!(audacity["edit.redo"], ["Ctrl+Y", "Ctrl+Shift+Z"]);
    }

    #[test]
    fn overrides_win_and_can_unbind() {
        let mut overrides = Bindings::new();
        overrides.insert("file.save".into(), vec!["ctrl+alt+s".into()]);
        overrides.insert("edit.undo".into(), vec![]);
        let keymap = Keymap::from_settings(&ShortcutSettings {
            preset: ShortcutPreset::Vellum,
            overrides,
        });
        assert_eq!(keymap.bindings["file.save"], ["Ctrl+Alt+S"]);
        assert!(!keymap.bindings.contains_key("edit.undo"));
        assert_eq!(keymap.commands_for("Ctrl+Alt+S"), ["file.save"]);
    }

    #[test]
    fn conflicts_are_found() {
        let mut overrides = Bindings::new();
        overrides.insert("file.save".into(), vec!["Ctrl+K".into()]);
        let keymap = Keymap::from_settings(&ShortcutSettings {
            preset: ShortcutPreset::Vellum,
            overrides,
        });
        assert_eq!(
            keymap.conflicts,
            [ShortcutConflict {
                chord: "Ctrl+K".into(),
                commands: vec!["app.command_palette".into(), "file.save".into()],
            }]
        );
    }

    #[test]
    fn workspace_commands_only_clash_inside_their_workspace() {
        let mut bindings = Bindings::new();
        bindings.insert("image.brush".into(), vec!["B".into()]);
        bindings.insert("video.blade".into(), vec!["B".into()]);
        assert!(find_conflicts(&bindings).is_empty());

        bindings.insert("image.blur".into(), vec!["B".into()]);
        let conflicts = find_conflicts(&bindings);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].commands, ["image.blur", "image.brush"]);

        // a global command on the same keys is shadowed inside the workspace, not a clash
        bindings.insert("app.home".into(), vec!["B".into()]);
        assert_eq!(find_conflicts(&bindings).len(), 1);
        bindings.insert("file.open".into(), vec!["B".into()]);
        let conflicts = find_conflicts(&bindings);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            conflicts[0].commands,
            ["app.home", "file.open", "image.blur", "image.brush"]
        );
    }

    #[test]
    fn contexts_come_from_the_kind() {
        assert_eq!(context_of("image.layer.new"), "image");
        assert_eq!(context_of("edit.undo"), "global");
        assert_eq!(context_of("workspace.image"), "global");
    }

    #[test]
    fn sanitize_cleans_overrides() {
        let mut settings = ShortcutSettings::default();
        settings.overrides.insert(
            "file.save".into(),
            vec!["ctrl+s".into(), "Ctrl+S".into(), "nonsense+".into()],
        );
        settings.overrides.insert(" ".into(), vec!["Ctrl+Q".into()]);
        sanitize(&mut settings);
        assert_eq!(settings.overrides.len(), 1);
        assert_eq!(settings.overrides["file.save"], ["Ctrl+S"]);
    }
}
