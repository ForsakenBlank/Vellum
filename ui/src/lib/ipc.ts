import { invoke, isTauri } from "@tauri-apps/api/core";

import type { AnimEstimate } from "../bindings/AnimEstimate";
import type { AnimOptions } from "../bindings/AnimOptions";
import type { AppInfo } from "../bindings/AppInfo";
import type { AppSettings } from "../bindings/AppSettings";
import type { BrushSettings } from "../bindings/BrushSettings";
import type { AutosaveStatus } from "../bindings/AutosaveStatus";
import type { CacheStats } from "../bindings/CacheStats";
import type { CommandInfo } from "../bindings/CommandInfo";
import type { CommandOutcome } from "../bindings/CommandOutcome";
import type { ConvertTarget } from "../bindings/ConvertTarget";
import type { DocumentInfo } from "../bindings/DocumentInfo";
import type { ExportOptions } from "../bindings/ExportOptions";
import type { DocumentState } from "../bindings/DocumentState";
import type { EngineStatus } from "../bindings/EngineStatus";
import type { GifFramesView } from "../bindings/GifFramesView";
import type { HistoryView } from "../bindings/HistoryView";
import type { JobId } from "../bindings/JobId";
import type { JobPriority } from "../bindings/JobPriority";
import type { JobResource } from "../bindings/JobResource";
import type { JobsOverview } from "../bindings/JobsOverview";
import type { Keymap } from "../bindings/Keymap";
import type { MediaAsset } from "../bindings/MediaAsset";
import type { MediaCapabilities } from "../bindings/MediaCapabilities";
import type { MediaInfo } from "../bindings/MediaInfo";
import type { NewImage } from "../bindings/NewImage";
import type { PreviewRequest } from "../bindings/PreviewRequest";
import type { TransformRequest } from "../bindings/TransformRequest";
import type { TransformTarget } from "../bindings/TransformTarget";
import type { StrokePoint } from "../bindings/StrokePoint";
import type { NodeId } from "../bindings/NodeId";
import type { NotificationsChanged } from "../bindings/NotificationsChanged";
import type { OpenFilesResult } from "../bindings/OpenFilesResult";
import type { RecentFiles } from "../bindings/RecentFiles";
import type { RecoveryOffer } from "../bindings/RecoveryOffer";
import type { RenamePattern } from "../bindings/RenamePattern";
import type { RenamePlan } from "../bindings/RenamePlan";
import type { SizeOptions } from "../bindings/SizeOptions";
import type { SizePlan } from "../bindings/SizePlan";
import type { SnapshotId } from "../bindings/SnapshotId";
import type { TileId } from "../bindings/TileId";
import type { ViewInfo } from "../bindings/ViewInfo";
import type { ViewState } from "../bindings/ViewState";
import type { WaveformOverview } from "../bindings/WaveformOverview";

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

  mediaCapabilities: () => invoke<MediaCapabilities>("media_capabilities"),
  probeMedia: (path: string) => invoke<MediaInfo>("probe_media", { path }),
  mediaThumbnail: (path: string) => invoke<MediaAsset>("media_thumbnail", { path }),
  mediaFilmstrip: (path: string, count: number, height: number) =>
    invoke<MediaAsset>("media_filmstrip", { path, count, height }),
  mediaWaveform: (path: string) => invoke<MediaAsset>("media_waveform", { path }),
  waveformOverview: (path: string, columns: number) =>
    invoke<WaveformOverview | null>("waveform_overview", { path, columns }),
  stripMetadata: (path: string) => invoke<JobId>("strip_metadata", { path }),
  mediaCacheStats: () => invoke<CacheStats>("media_cache_stats"),
  cleanMediaCache: () => invoke<CacheStats>("clean_media_cache"),

  runQuickTool: (document: string) => invoke<JobId[]>("run_quick_tool", { document }),
  sizePlan: (path: string, options: SizeOptions) =>
    invoke<SizePlan>("size_plan", { path, options }),
  animEstimate: (path: string, options: AnimOptions) =>
    invoke<AnimEstimate>("anim_estimate", { path, options }),
  gifFrames: (path: string) => invoke<GifFramesView>("gif_frames", { path }),
  convertTargets: (path: string) => invoke<ConvertTarget[]>("convert_targets", { path }),
  renamePlan: (document: string, pattern: RenamePattern) =>
    invoke<RenamePlan>("rename_plan", { document, pattern }),
  mediaFrame: (path: string, seconds: number, height: number) =>
    invoke<string>("media_frame", { path, seconds, height }),
  previewFile: (path: string) => invoke<null>("preview_file", { path }),
  copyFiles: (paths: string[]) => invoke<null>("copy_files", { paths }),

  // the gpu viewport, apps/desktop/src/render.rs
  openView: (document: string) => invoke<ViewInfo>("open_view", { document }),
  viewFrame: (document: string, view: ViewState, evicted: TileId[], reset: boolean) =>
    invoke<ArrayBuffer>("view_frame", { document, view, evicted, reset }),
  closeView: (document: string) => invoke<null>("close_view", { document }),
  renderStatus: () => invoke<EngineStatus>("render_status"),

  // the image editor, apps/desktop/src/image.rs
  imageNew: (spec: NewImage, window: string) => invoke<DocumentInfo>("image_new", { spec, window }),
  imageCommand: (document: string, kind: string, params: unknown = {}) =>
    invoke<null>("image_command", { document, record: { kind, params } }),
  imageStrokeBegin: (document: string, settings: BrushSettings) =>
    invoke<null>("image_stroke_begin", { document, settings }),
  imageStroke: (document: string, points: StrokePoint[]) =>
    invoke<null>("image_stroke", { document, points }),
  imageStrokeEnd: (document: string) => invoke<null>("image_stroke_end", { document }),
  imageStrokeCancel: (document: string) => invoke<null>("image_stroke_cancel", { document }),
  imagePreview: (document: string, request: PreviewRequest) =>
    invoke<null>("image_preview", { document, request }),
  imageOutline: (document: string, level: number) =>
    invoke<[number, number][][]>("image_outline", { document, level }),
  imageMagneticBegin: (document: string, sampleAll: boolean, contrast: number) =>
    invoke<null>("image_magnetic_begin", { document, sampleAll, contrast }),
  imageMagnetic: (document: string, from: [number, number], to: [number, number], width: number) =>
    invoke<[number, number][]>("image_magnetic", { document, from, to, width }),
  imageMagneticEnd: (document: string) => invoke<null>("image_magnetic_end", { document }),
  imagePickColor: (document: string, x: number, y: number, size: number, all: boolean) =>
    invoke<[number, number, number, number]>("image_pick_color", { document, x, y, size, all }),
  imageThumbnails: (document: string, layers: [number, boolean][], size: number) =>
    invoke<ArrayBuffer>("image_thumbnails", { document, layers, size }),
  imageSaveAs: (document: string, path: string) =>
    invoke<JobId>("image_save_as", { document, path }),
  imageTransformBox: (document: string, target: TransformTarget) =>
    invoke<[number, number, number, number]>("image_transform_box", { document, target }),
  imageTransform: (document: string, request: TransformRequest) =>
    invoke<null>("image_transform", { document, request }),
  imageExport: (document: string, path: string, options: ExportOptions) =>
    invoke<JobId>("image_export", { document, path, options }),
};
