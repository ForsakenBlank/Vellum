import type { AppInfo } from "../bindings/AppInfo";
import type { CommandRecord } from "../bindings/CommandRecord";
import type { CommandSpec } from "../bindings/CommandSpec";
import type { JobId } from "../bindings/JobId";
import type { JobsOverview } from "../bindings/JobsOverview";
import type { OpenPlan } from "../bindings/OpenPlan";
import type { OpenTarget } from "../bindings/OpenTarget";
import type { Priority } from "../bindings/Priority";
import type { RecentEntry } from "../bindings/RecentEntry";
import type { RecoveryOffer } from "../bindings/RecoveryOffer";
import type { SessionView } from "../bindings/SessionView";
import type { SettingsMap } from "../bindings/SettingsMap";
import type { Backend } from "./backend";

/**
 * every command the rust side exposes, with its types. the payload types are
 * generated from rust. the command names and argument lists are kept in step
 * with apps/desktop/src/commands.rs by hand
 */
export function createApi(backend: Backend) {
  return {
    appInfo: () => backend.invoke<AppInfo>("app_info"),
    uiReady: () => backend.invoke<null>("ui_ready"),
    commands: () => backend.invoke<CommandSpec[]>("commands_list"),

    settings: {
      all: () => backend.invoke<SettingsMap>("settings_all"),
      set: <K extends keyof SettingsMap>(section: K, value: SettingsMap[K]) =>
        backend.invoke<SettingsMap[K]>("settings_set", { section, value }),
    },

    jobs: {
      overview: () => backend.invoke<JobsOverview>("jobs_overview"),
      pause: (id: JobId) => backend.invoke<null>("job_pause", { id }),
      resume: (id: JobId) => backend.invoke<null>("job_resume", { id }),
      cancel: (id: JobId) => backend.invoke<null>("job_cancel", { id }),
      retry: (id: JobId) => backend.invoke<null>("job_retry", { id }),
      remove: (id: JobId) => backend.invoke<null>("job_remove", { id }),
      setPriority: (id: JobId, priority: Priority) =>
        backend.invoke<null>("job_set_priority", { id, priority }),
      moveTo: (id: JobId, index: number) => backend.invoke<null>("job_move", { id, index }),
      clearFinished: () => backend.invoke<null>("jobs_clear_finished"),
      runSample: (seconds: number, fail: boolean) =>
        backend.invoke<JobId>("dev_sample_job", { seconds, fail }),
    },

    shell: {
      view: () => backend.invoke<SessionView>("shell_view"),
      run: (record: CommandRecord) => backend.invoke<SessionView>("shell_run", { record }),
    },

    files: {
      plan: (paths: string[], target: OpenTarget) =>
        backend.invoke<OpenPlan>("files_plan", { paths, target }),
      pick: () => backend.invoke<string[]>("files_pick"),
      open: (path: string) => backend.invoke<null>("path_open", { path }),
      reveal: (path: string) => backend.invoke<null>("path_reveal", { path }),
    },

    recent: {
      list: () => backend.invoke<RecentEntry[]>("recent_list"),
      pin: (path: string, pinned: boolean) => backend.invoke<null>("recent_pin", { path, pinned }),
      remove: (path: string) => backend.invoke<null>("recent_remove", { path }),
      clear: () => backend.invoke<null>("recent_clear"),
    },

    recovery: {
      offer: () => backend.invoke<RecoveryOffer | null>("recovery_offer"),
      restore: () => backend.invoke<SessionView>("recovery_restore"),
      discard: () => backend.invoke<null>("recovery_discard"),
    },
  };
}

export type Api = ReturnType<typeof createApi>;

/** turns whatever a failed command threw into a sentence for the user */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (typeof error === "object" && error !== null && "message" in error) {
    const { message } = error;
    if (typeof message === "string") return message;
  }
  return "Something went wrong. Try again, and restart Vellum if it keeps happening.";
}
