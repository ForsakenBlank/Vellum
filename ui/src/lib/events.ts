import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { AppSettings } from "../bindings/AppSettings";
import type { HistoryChanged } from "../bindings/HistoryChanged";
import type { JobEvent } from "../bindings/JobEvent";
import type { UiReady } from "../bindings/UiReady";

// event names and payload types. the payload types are generated from rust, the names are
// kept here by hand and a rust test (apps/desktop/src/bridge.rs) checks they still match.

/** events rust sends to the ui */
export interface RustEvents {
  "job:changed": JobEvent;
  "history:changed": HistoryChanged;
  "settings:changed": { settings: AppSettings };
}

/** events the ui sends to rust. rust only accepts the ones it registered. */
export interface UiEvents {
  "ui:ready": UiReady;
}

export function onRustEvent<K extends keyof RustEvents>(
  name: K,
  handler: (payload: RustEvents[K]) => void,
): Promise<UnlistenFn> {
  return listen<RustEvents[K]>(name, (event) => {
    handler(event.payload);
  });
}

export function sendUiEvent<K extends keyof UiEvents>(name: K, payload: UiEvents[K]) {
  return invoke<null>("ui_event", { name, payload });
}
