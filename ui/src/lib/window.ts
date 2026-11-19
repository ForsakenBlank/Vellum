import { getCurrentWindow } from "@tauri-apps/api/window";

import { inDesktopApp } from "./ipc";

function currentLabel(): string {
  if (!inDesktopApp) return "main";
  try {
    return getCurrentWindow().label;
  } catch {
    // only happens when the tauri globals are mocked, as in unit tests
    return "main";
  }
}

/** the label of the window this ui runs in. "main" is the first window, torn off document
 * windows are "doc-1", "doc-2" and so on. */
export const windowLabel: string = currentLabel();

export const isMainWindow = windowLabel === "main";
