import { onRustEvent, sendUiEvent } from "./lib/events";
import { inDesktopApp, ipc } from "./lib/ipc";
import { useJobs } from "./state/jobs";
import { applyAppearance, useSettings } from "./state/settings";

/**
 * subscribes to rust events first, then loads the current state, so nothing that happens
 * in between is missed. older events that arrive after the load are dropped by revision.
 */
export async function connectToRust() {
  if (!inDesktopApp) {
    return;
  }

  await Promise.all([
    onRustEvent("job:changed", (event) => {
      useJobs.getState().apply(event);
    }),
    onRustEvent("settings:changed", ({ settings }) => {
      useSettings.getState().replace(settings);
      applyAppearance(settings);
    }),
  ]);

  const [settings, jobs] = await Promise.all([ipc.getSettings(), ipc.listJobs()]);
  useSettings.getState().replace(settings);
  applyAppearance(settings);
  useJobs.getState().merge(jobs);

  await sendUiEvent("ui:ready", { uiVersion: __APP_VERSION__ });
}
