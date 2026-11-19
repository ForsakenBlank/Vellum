import { invoke, isTauri } from "@tauri-apps/api/core";

import type { AppInfo } from "../bindings/AppInfo";
import type { AppSettings } from "../bindings/AppSettings";
import type { AutosaveStatus } from "../bindings/AutosaveStatus";
import type { CommandInfo } from "../bindings/CommandInfo";
import type { CommandOutcome } from "../bindings/CommandOutcome";
import type { DocumentInfo } from "../bindings/DocumentInfo";
import type { DocumentState } from "../bindings/DocumentState";
import type { HistoryView } from "../bindings/HistoryView";
import type { JobId } from "../bindings/JobId";
import type { JobPriority } from "../bindings/JobPriority";
import type { JobResource } from "../bindings/JobResource";
import type { JobsOverview } from "../bindings/JobsOverview";
import type { Keymap } from "../bindings/Keymap";
import type { NodeId } from "../bindings/NodeId";
import type { NotificationsChanged } from "../bindings/NotificationsChanged";
import type { OpenFilesResult } from "../bindings/OpenFilesResult";
import type { RecentFiles } from "../bindings/RecentFiles";
import type { RecoveryOffer } from "../bindings/RecoveryOffer";
import type { SnapshotId } from "../bindings/SnapshotId";

/** every field optional, all the way down, for settings patches. arrays are replaced whole. */
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends unknown[] ? T[K] : T[K] extends object ? DeepPartial<T[K]> : T[K];
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

  listCommands: () => invoke<CommandInfo[]>("list_commands"),
  runCommand: (kind: string, document: string | null, params: unknown = {}) =>
    invoke<CommandOutcome>("run_command", { record: { kind, params }, document }),
  getKeymap: () => invoke<Keymap>("get_keymap"),

  listDocuments: () => invoke<DocumentInfo[]>("list_documents"),
  openFiles: (paths: string[], tool: string | null, window: string) =>
    invoke<OpenFilesResult>("open_files", { paths, tool, window }),
  closeDocument: (id: string) => invoke<null>("close_document", { id }),
  moveDocument: (id: string, window: string, index: number | null) =>
    invoke<null>("move_document", { id, window, index }),
  tearOffDocument: (id: string) => invoke<string>("tear_off_document", { id }),
  documentCommand: (id: string, kind: string, params: unknown) =>
    invoke<null>("document_command", { id, record: { kind, params } }),
  sealDocument: (id: string) => invoke<null>("seal_document", { id }),
  getDocument: (id: string) => invoke<DocumentState>("get_document", { id }),
  documentHistory: (id: string) => invoke<HistoryView>("document_history", { id }),
  checkoutHistory: (id: string, node: NodeId) => invoke<null>("checkout_history", { id, node }),
  createSnapshot: (id: string, name: string) => invoke<SnapshotId>("create_snapshot", { id, name }),

  listRecent: () => invoke<RecentFiles>("list_recent"),
  pinRecent: (path: string, pinned: boolean) => invoke<null>("pin_recent", { path, pinned }),
  removeRecent: (path: string) => invoke<null>("remove_recent", { path }),

  recoveryOffer: () => invoke<RecoveryOffer | null>("recovery_offer"),
  restoreSession: () => invoke<DocumentInfo[]>("restore_session"),
  discardRecovery: () => invoke<null>("discard_recovery"),
  autosaveStatus: () => invoke<AutosaveStatus>("autosave_status"),

  listNotifications: () => invoke<NotificationsChanged>("list_notifications"),
  dismissNotification: (id: number) => invoke<null>("dismiss_notification", { id }),
  openResult: (path: string, reveal: boolean) => invoke<null>("open_result", { path, reveal }),

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
