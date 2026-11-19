import { onRustEvent, sendUiEvent } from "./lib/events";
import { inDesktopApp, ipc } from "./lib/ipc";
import { isMainWindow } from "./lib/window";
import { closeWindowIfEmpty } from "./shell/actions";
import { listenForFileDrops } from "./shell/fileDrop";
import { performAction } from "./shell/notify";
import { useCommands } from "./state/commands";
import { useDocuments } from "./state/documents";
import { resultPath, useJobs } from "./state/jobs";
import { useNotifications } from "./state/notifications";
import { useRecent } from "./state/recent";
import { applyAppearance, useSettings } from "./state/settings";
import { useUi } from "./state/ui";

/**
 * subscribes to rust events first, then loads the current state, so nothing that happens
 * in between is missed. older job events that arrive after the load are dropped by revision.
 */
export async function connectToRust() {
  if (!inDesktopApp) {
    useRecent.getState().replace([]);
    return;
  }

  await Promise.all([
    onRustEvent("job:changed", (event) => {
      useJobs.getState().apply(event);
      if (event.type === "updated" && event.job.state === "completed") openIfAsked(event.job.id);
    }),
    onRustEvent("settings:changed", ({ settings }) => {
      useSettings.getState().replace(settings);
      applyAppearance(settings);
    }),
    onRustEvent("recent:changed", ({ settings }) => {
      useRecent.getState().replace(settings.files);
    }),
    onRustEvent("documents:changed", ({ documents }) => {
      useDocuments.getState().setDocuments(documents);
      closeWindowIfEmpty();
    }),
    onRustEvent("history:changed", ({ document, summary }) => {
      useDocuments.getState().setSummary(document, summary);
    }),
    onRustEvent("notifications:changed", (change) => {
      const fresh = useNotifications.getState().replace(change);
      if (isMainWindow) toastNotifications(fresh);
    }),
    onRustEvent("keymap:changed", (keymap) => {
      useCommands.getState().setKeymap(keymap);
    }),
    onRustEvent("autosave:changed", (status) => {
      useUi.getState().setAutosave(status);
    }),
    listenForFileDrops(),
  ]);

  const [settings, jobs, commands, keymap, documents, recent, notifications, autosave] =
    await Promise.all([
      ipc.getSettings(),
      ipc.listJobs(),
      ipc.listCommands(),
      ipc.getKeymap(),
      ipc.listDocuments(),
      ipc.listRecent(),
      ipc.listNotifications(),
      ipc.autosaveStatus(),
    ]);
  useSettings.getState().replace(settings);
  applyAppearance(settings);
  useJobs.getState().merge(jobs);
  useCommands.getState().setInfos(commands);
  useCommands.getState().setKeymap(keymap);
  useRecent.getState().replace(recent.files);
  // notifications from before this window opened are already known, so no toasts for them
  useNotifications.getState().replace(notifications);
  useUi.getState().setAutosave(autosave);
  setInitialDocuments(documents);

  if (isMainWindow) {
    useUi.getState().setRecovery(await ipc.recoveryOffer());
  }

  await sendUiEvent("ui:ready", { uiVersion: __APP_VERSION__ });
}

function setInitialDocuments(
  documents: Parameters<ReturnType<typeof useDocuments.getState>["setDocuments"]>[0],
) {
  const store = useDocuments.getState();
  store.setDocuments(documents);
  // a torn off window opens on its own document
  if (!isMainWindow) {
    const own = documents.find((d) => d.window !== "main" && d.window === windowOf(documents));
    if (own) store.activate(own.id);
  }
}

function windowOf(documents: { window: string }[]): string | undefined {
  return documents.find((d) => d.window !== "main")?.window;
}

function openIfAsked(id: number) {
  const { openWhenDone, jobs, toggleOpenWhenDone } = useJobs.getState();
  if (!openWhenDone[id]) return;
  toggleOpenWhenDone(id);
  const job = jobs[id];
  const path = job ? resultPath(job) : null;
  if (path) {
    void ipc.openResult(path, false).catch((err: unknown) => {
      useUi
        .getState()
        .toast({ level: "error", title: "The result could not be opened", body: String(err) });
    });
  }
}

function toastNotifications(
  fresh: ReturnType<ReturnType<typeof useNotifications.getState>["replace"]>,
) {
  for (const note of fresh) {
    const action = note.action;
    useUi.getState().toast({
      level: note.level,
      title: note.title,
      body: note.body ?? undefined,
      action:
        action && note.actionLabel
          ? {
              label: note.actionLabel,
              run: () => {
                performAction(action);
              },
            }
          : undefined,
    });
  }
}
