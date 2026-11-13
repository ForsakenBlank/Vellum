import { convertFileSrc, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import type { Envelope } from "../bindings/Envelope";

// must match EVENT_CHANNEL in apps/desktop/src/lib.rs
const EVENT_CHANNEL = "vellum:event";

/** files being dragged over the window from outside. x and y are css pixels */
export type FileDropEvent =
  | { type: "enter"; paths: string[]; x: number; y: number }
  | { type: "over"; x: number; y: number }
  | { type: "drop"; paths: string[]; x: number; y: number }
  | { type: "leave" };

/** the things the ui needs from the rust side. tests swap in a fake */
export interface Backend {
  /** label of the window this ui runs in, "main" or a torn off one */
  readonly windowLabel: string;
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  /** resolves to a function that stops listening */
  listen(onEvent: (envelope: Envelope) => void): Promise<() => void>;
  /** files dragged in from the desktop. resolves to a function that stops listening */
  onFileDrop(onEvent: (event: FileDropEvent) => void): Promise<() => void>;
  /** a url an <img> can load a local file from */
  previewUrl(path: string): string;
}

/** the real backend, or null when the ui is open in a plain browser */
export function desktopBackend(): Backend | null {
  if (!isTauri()) return null;
  const webview = getCurrentWebview();
  return {
    windowLabel: webview.label,
    invoke: (command, args) => invoke(command, args),
    listen: (onEvent) =>
      listen<Envelope>(EVENT_CHANNEL, (event) => {
        onEvent(event.payload);
      }),
    onFileDrop: (onEvent) =>
      webview.onDragDropEvent(({ payload }) => {
        // tauri reports physical pixels, the dom works in css pixels
        const scale = window.devicePixelRatio || 1;
        switch (payload.type) {
          case "enter":
          case "drop":
            onEvent({
              type: payload.type,
              paths: payload.paths,
              x: payload.position.x / scale,
              y: payload.position.y / scale,
            });
            break;
          case "over":
            onEvent({
              type: "over",
              x: payload.position.x / scale,
              y: payload.position.y / scale,
            });
            break;
          case "leave":
            onEvent({ type: "leave" });
            break;
        }
      }),
    previewUrl: (path) => convertFileSrc(path),
  };
}
