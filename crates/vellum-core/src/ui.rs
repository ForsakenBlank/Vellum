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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppSettings;
    use crate::history::HistoryChanged;
    use crate::jobs::JobEvent;
    use crate::settings::SettingsChanged;

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
            UiReady::NAME,
        ];
        for name in names {
            assert!(
                source.contains(&format!("\"{name}\"")),
                "ui/src/lib/events.ts does not mention \"{name}\""
            );
        }
    }
}
