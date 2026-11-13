import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Envelope } from "../bindings/Envelope";

// must match EVENT_CHANNEL in apps/desktop/src/lib.rs
const EVENT_CHANNEL = "vellum:event";

/** the two things the ui needs from the rust side. tests swap in a fake */
export interface Backend {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  /** resolves to a function that stops listening */
  listen(onEvent: (envelope: Envelope) => void): Promise<() => void>;
}

/** the real backend, or null when the ui is open in a plain browser */
export function desktopBackend(): Backend | null {
  if (!isTauri()) return null;
  return {
    invoke: (command, args) => invoke(command, args),
    listen: (onEvent) =>
      listen<Envelope>(EVENT_CHANNEL, (event) => {
        onEvent(event.payload);
      }),
  };
}
