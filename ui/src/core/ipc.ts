import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { JobInfo } from "../bindings/JobInfo";
import type { Settings } from "../bindings/Settings";
import type { VellumEvent } from "../bindings/VellumEvent";

// must match UI_EVENT_CHANNEL in crates/vellum-core/src/events.rs
const EVENT_CHANNEL = "vellum://event";

export interface AppInfo {
  version: string;
  settingsPath: string;
  settingsNotice: string | null;
  settingsReadOnly: boolean;
}

// a partial settings object, deep merged on the rust side
export type SettingsPatch = {
  [K in keyof Settings]?: Settings[K] extends object ? Partial<Settings[K]> : Settings[K];
};

export const inDesktopApp = isTauri;

export const ipc = {
  appInfo: () => invoke<AppInfo>("app_info"),
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: SettingsPatch) => invoke<Settings>("update_settings", { patch }),
  listJobs: () => invoke<JobInfo[]>("list_jobs"),
  cancelJob: (id: number) => invoke<void>("cancel_job", { id }),
  pauseJob: (id: number) => invoke<void>("pause_job", { id }),
  resumeJob: (id: number) => invoke<void>("resume_job", { id }),
  retryJob: (id: number) => invoke<void>("retry_job", { id }),
  removeJob: (id: number) => invoke<void>("remove_job", { id }),
  clearFinishedJobs: () => invoke<void>("clear_finished_jobs"),
  startSampleJob: (seconds?: number) => invoke<number>("start_sample_job", { seconds }),
};

export function onCoreEvent(handler: (event: VellumEvent) => void): Promise<UnlistenFn> {
  return listen<VellumEvent>(EVENT_CHANNEL, (e) => handler(e.payload));
}
