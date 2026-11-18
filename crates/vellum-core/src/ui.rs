//! small types the ui and the desktop shell share that belong to no other module.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::Event;

/// sent by the ui once it has loaded and subscribed to events
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UiReady {
    pub ui_version: String,
}

impl Event for UiReady {
    const NAME: &'static str = "ui:ready";
}

/// basic facts about the running app, for the about box and bug reports
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub version: String,
    pub settings_path: String,
    pub os: String,
}

/// writes a generated file only when its contents change, so file times stay put
#[cfg(test)]
fn write_generated(relative: &str, value: &serde_json::Value) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppSettings;
    use crate::actions;
    use crate::app::AutosaveStatus;
    use crate::documents::{DocumentModel, DocumentsChanged, PlaceholderDocument};
    use crate::files::FILE_KINDS;
    use crate::history::HistoryChanged;
    use crate::jobs::JobEvent;
    use crate::notifications::NotificationsChanged;
    use crate::recent::RecentFiles;
    use crate::settings::{SettingsChanged, SettingsSchema};
    use crate::shortcuts::{Keymap, ShortcutPreset, ShortcutSettings};
    use serde_json::json;

    /// the ui keeps its own map from event names to types in `ui/src/lib/events.ts`. this
    /// fails if a name changes on the rust side without the ui following.
    #[test]
    fn ui_event_map_lists_every_event() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/src/lib/events.ts");
        let source = std::fs::read_to_string(path).unwrap();
        let names = [
            JobEvent::NAME,
            HistoryChanged::NAME,
            <SettingsChanged<AppSettings> as Event>::NAME,
            <SettingsChanged<RecentFiles> as Event>::NAME,
            UiReady::NAME,
            DocumentsChanged::NAME,
            NotificationsChanged::NAME,
            Keymap::NAME,
            AutosaveStatus::NAME,
        ];
        for name in names {
            assert!(
                source.contains(&format!("\"{name}\"")),
                "ui/src/lib/events.ts does not mention \"{name}\""
            );
        }
    }

    /// the file routing table for the ui, next to the generated types
    #[test]
    fn export_file_kinds() {
        write_generated(
            "../../ui/src/bindings/fileKinds.json",
            &serde_json::to_value(FILE_KINDS).unwrap(),
        );
    }

    /// real data for the fake backend the playwright tests run against, so the e2e tests
    /// see the same commands, shortcuts and defaults as the app
    #[test]
    fn export_e2e_fixture() {
        let commands: Vec<_> = actions::registry().unwrap().infos().cloned().collect();
        let keymaps: serde_json::Map<_, _> = ShortcutPreset::ALL
            .iter()
            .map(|preset| {
                let keymap = Keymap::from_settings(&ShortcutSettings {
                    preset: *preset,
                    overrides: Default::default(),
                });
                let name = serde_json::to_value(preset).unwrap();
                (
                    name.as_str().unwrap().to_owned(),
                    serde_json::to_value(keymap).unwrap(),
                )
            })
            .collect();
        let documents: Vec<_> = PlaceholderDocument::commands().infos().cloned().collect();
        write_generated(
            "../../ui/e2e/fixtures/backend.json",
            &json!({
                "commands": commands,
                "documentCommands": documents,
                "keymaps": keymaps,
                "settings": AppSettings::default(),
                "settingsVersion": AppSettings::VERSION,
            }),
        );
    }
}
