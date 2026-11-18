import { invoke, isTauri } from "@tauri-apps/api/core";

import type { AppInfo } from "../bindings/AppInfo";
import type { AppSettings } from "../bindings/AppSettings";
import type { JobId } from "../bindings/JobId";
import type { JobPriority } from "../bindings/JobPriority";
import type { JobResource } from "../bindings/JobResource";
import type { JobsOverview } from "../bindings/JobsOverview";

/** every field optional, all the way down, for settings patches */
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K];
};

/** false in a plain browser or in tests, where there is no rust side to talk to */
export const inDesktopApp = isTauri();

// thin typed wrappers around the rust commands in apps/desktop/src/ipc.rs
export const ipc = {
  appInfo: () => invoke<AppInfo>("app_info"),

  getSettings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (patch: DeepPartial<AppSettings>) =>
    invoke<AppSettings>("update_settings", { patch }),
  resetSettings: () => invoke<AppSettings>("reset_settings"),

  listJobs: () => invoke<JobsOverview>("list_jobs"),
  cancelJob: (id: JobId) => invoke<null>("cancel_job", { id }),
  pauseJob: (id: JobId) => invoke<null>("pause_job", { id }),
  resumeJob: (id: JobId) => invoke<null>("resume_job", { id }),
  retryJob: (id: JobId) => invoke<null>("retry_job", { id }),
  removeJob: (id: JobId) => invoke<null>("remove_job", { id }),
  clearFinishedJobs: () => invoke<null>("clear_finished_jobs"),
  setJobPriority: (id: JobId, priority: JobPriority) =>
    invoke<null>("set_job_priority", { id, priority }),
  moveJob: (id: JobId, before: JobId | null) => invoke<null>("move_job", { id, before }),
  startSampleJob: (seconds: number, resource: JobResource, failFirst: boolean) =>
    invoke<JobId>("start_sample_job", { seconds, resource, failFirst }),
};
