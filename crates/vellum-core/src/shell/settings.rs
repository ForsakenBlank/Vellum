//! settings sections for the app shell: general behaviour, keyboard
//! shortcuts and panel layouts.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::settings::SettingsSection;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct GeneralSettings {
    /// seconds between autosaves. 0 turns autosave off
    pub autosave_seconds: u32,
    /// how many recent files the home screen remembers, not counting pinned ones
    pub recent_limit: u32,
    /// show a notification when a job finishes
    pub notify_jobs: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            autosave_seconds: 30,
            recent_limit: 30,
            notify_jobs: true,
        }
    }
}

impl SettingsSection for GeneralSettings {
    const KEY: &'static str = "general";

    fn sanitize(&mut self) {
        if self.autosave_seconds != 0 {
            self.autosave_seconds = self.autosave_seconds.clamp(5, 3600);
        }
        self.recent_limit = self.recent_limit.min(200);
    }
}

/// a starting set of shortcuts for people coming from another app
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ShortcutPreset {
    #[default]
    Vellum,
    Photoshop,
    Premiere,
    Resolve,
    Audacity,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ShortcutSettings {
    pub preset: ShortcutPreset,
    /// the user's own changes on top of the preset, by command id. an empty
    /// list means the user removed every shortcut from that command
    pub bindings: BTreeMap<String, Vec<String>>,
}

impl SettingsSection for ShortcutSettings {
    const KEY: &'static str = "shortcuts";

    fn sanitize(&mut self) {
        for keys in self.bindings.values_mut() {
            let mut seen = BTreeSet::new();
            keys.retain(|key| !key.trim().is_empty() && seen.insert(key.clone()));
        }
        self.bindings.retain(|id, _| !id.is_empty());
    }
}

/// one tabbed group of panels inside a dock zone
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PanelStack {
    pub panels: Vec<String>,
    pub active: Option<String>,
    /// share of the zone's length, relative to the other stacks
    #[serde(default = "one")]
    pub weight: f32,
}

fn one() -> f32 {
    1.0
}

/// one edge of the workspace that panels can dock to
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DockZone {
    /// width for the side zones and height for the bottom one, in rem
    pub size: f32,
    pub open: bool,
    pub stacks: Vec<PanelStack>,
}

impl DockZone {
    fn sanitize(&mut self, min: f32, max: f32, seen: &mut BTreeSet<String>) {
        self.size = if self.size.is_finite() {
            self.size.clamp(min, max)
        } else {
            min
        };
        for stack in &mut self.stacks {
            // a panel can only be in one place at a time, the first one wins
            stack.panels.retain(|panel| seen.insert(panel.clone()));
            if stack
                .active
                .as_ref()
                .is_none_or(|active| !stack.panels.contains(active))
            {
                stack.active = stack.panels.first().cloned();
            }
            if !stack.weight.is_finite() || stack.weight <= 0.0 {
                stack.weight = 1.0;
            }
        }
        self.stacks.retain(|stack| !stack.panels.is_empty());
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DockLayout {
    pub left: DockZone,
    pub right: DockZone,
    pub bottom: DockZone,
}

impl DockLayout {
    pub fn sanitize(&mut self) {
        let mut seen = BTreeSet::new();
        self.left.sanitize(10.0, 40.0, &mut seen);
        self.right.sanitize(10.0, 40.0, &mut seen);
        self.bottom.sanitize(6.0, 40.0, &mut seen);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SavedLayout {
    pub name: String,
    pub workspace: String,
    pub layout: DockLayout,
}

/// panel layouts. built in layouts live in the ui, this holds what the user changed
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct LayoutSettings {
    /// the layout each workspace is in right now. missing means its default
    pub current: BTreeMap<String, DockLayout>,
    /// layouts the user saved by name
    pub saved: Vec<SavedLayout>,
}

const MAX_SAVED_LAYOUTS: usize = 50;

impl SettingsSection for LayoutSettings {
    const KEY: &'static str = "layouts";

    fn sanitize(&mut self) {
        for layout in self.current.values_mut() {
            layout.sanitize();
        }
        let mut names = BTreeSet::new();
        for saved in &mut self.saved {
            saved.name = saved.name.trim().chars().take(60).collect();
            saved.layout.sanitize();
        }
        // a name is unique within its workspace. the later one wins, it is the newer save
        self.saved.reverse();
        self.saved.retain(|saved| {
            !saved.name.is_empty() && names.insert((saved.workspace.clone(), saved.name.clone()))
        });
        self.saved.reverse();
        let excess = self.saved.len().saturating_sub(MAX_SAVED_LAYOUTS);
        self.saved.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::settings::SettingsStore;

    fn zone(size: f32, stacks: &[&[&str]]) -> DockZone {
        DockZone {
            size,
            open: true,
            stacks: stacks
                .iter()
                .map(|panels| PanelStack {
                    panels: panels.iter().map(|p| (*p).to_owned()).collect(),
                    active: Some("missing".into()),
                    weight: -2.0,
                })
                .collect(),
        }
    }

    #[test]
    fn autosave_is_off_or_in_a_sane_range() {
        let store = SettingsStore::in_memory(None);
        let saved = store
            .update::<GeneralSettings>(|g| g.autosave_seconds = 1)
            .unwrap();
        assert_eq!(saved.autosave_seconds, 5);
        let saved = store
            .update::<GeneralSettings>(|g| g.autosave_seconds = 0)
            .unwrap();
        assert_eq!(saved.autosave_seconds, 0);
    }

    #[test]
    fn shortcut_lists_lose_blanks_and_repeats() {
        let store = SettingsStore::in_memory(None);
        store.register::<ShortcutSettings>();
        let saved = store
            .set_raw(
                "shortcuts",
                json!({
                    "preset": "photoshop",
                    "bindings": { "shell.close": ["Mod+W", "", "Mod+W", "Mod+F4"], "shell.home": [] }
                }),
            )
            .unwrap();
        assert_eq!(
            saved["bindings"],
            json!({ "shell.close": ["Mod+W", "Mod+F4"], "shell.home": [] })
        );
    }

    #[test]
    fn layouts_are_cleaned_up() {
        let mut layout = DockLayout {
            left: zone(500.0, &[&["layers", "history"], &[]]),
            right: zone(f32::NAN, &[&["history", "properties"]]),
            bottom: zone(1.0, &[]),
        };
        layout.sanitize();

        assert_eq!(layout.left.size, 40.0);
        assert_eq!(layout.left.stacks.len(), 1);
        assert_eq!(layout.left.stacks[0].active.as_deref(), Some("layers"));
        assert_eq!(layout.left.stacks[0].weight, 1.0);
        assert_eq!(layout.right.size, 10.0);
        assert_eq!(layout.right.stacks[0].panels, ["properties"]);
        assert_eq!(layout.bottom.size, 6.0);
    }

    #[test]
    fn saving_a_layout_under_a_taken_name_replaces_it() {
        let layout = DockLayout {
            left: zone(16.0, &[&["layers"]]),
            right: zone(16.0, &[]),
            bottom: zone(12.0, &[]),
        };
        let saved = |name: &str, workspace: &str| SavedLayout {
            name: name.into(),
            workspace: workspace.into(),
            layout: layout.clone(),
        };
        let mut settings = LayoutSettings {
            current: BTreeMap::new(),
            saved: vec![
                saved("Editing", "image"),
                saved("Editing", "video"),
                saved(" Editing ", "image"),
                saved("   ", "image"),
            ],
        };
        settings.sanitize();
        let names: Vec<_> = settings
            .saved
            .iter()
            .map(|s| (s.name.as_str(), s.workspace.as_str()))
            .collect();
        assert_eq!(names, [("Editing", "video"), ("Editing", "image")]);
    }
}
