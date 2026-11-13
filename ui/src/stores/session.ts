import { useMemo } from "react";
import { create } from "zustand";

import type { DocumentId } from "../bindings/DocumentId";
import type { OpenDocument } from "../bindings/OpenDocument";
import type { OpenPlan } from "../bindings/OpenPlan";
import type { OpenTarget } from "../bindings/OpenTarget";
import type { RecoveryOffer } from "../bindings/RecoveryOffer";
import type { SessionView } from "../bindings/SessionView";
import type { JsonValue } from "../bindings/serde_json/JsonValue";
import type { View } from "../bindings/View";
import { perform, useApp } from "./app";

interface SessionState {
  /** null until the backend has sent it */
  session: SessionView | null;
  /** what a crashed run left behind, until the user answers */
  recovery: RecoveryOffer | null;
  applyView: (session: SessionView) => void;
  setRecovery: (recovery: RecoveryOffer | null) => void;
}

export const useSession = create<SessionState>()((set) => ({
  session: null,
  recovery: null,
  applyView: (session) => {
    // a command reply and its event carry the same view, whichever comes
    // second must not wind things back
    set((state) =>
      state.session && state.session.revision > session.revision ? state : { session },
    );
  },
  setRecovery: (recovery) => {
    set({ recovery });
  },
}));

const HOME: View = { type: "home" };

/** what this window is showing */
export function useView(): View {
  const label = useApp((state) => state.windowLabel);
  return useSession(
    (state) => state.session?.windows.find((window) => window.label === label)?.view ?? HOME,
  );
}

const NO_TABS: OpenDocument[] = [];

/** this window's tabs in order */
export function useTabs(): OpenDocument[] {
  const label = useApp((state) => state.windowLabel);
  const documents = useSession((state) => state.session?.documents ?? NO_TABS);
  return useMemo(() => documents.filter((doc) => doc.window === label), [documents, label]);
}

export function activeDocument(): OpenDocument | null {
  const { session } = useSession.getState();
  const label = useApp.getState().windowLabel;
  const view = session?.windows.find((window) => window.label === label)?.view;
  if (view?.type !== "document") return null;
  return session?.documents.find((doc) => doc.id === view.id) ?? null;
}

export function useActiveDocument(): OpenDocument | null {
  const view = useView();
  return useSession((state) =>
    view.type === "document"
      ? (state.session?.documents.find((doc) => doc.id === view.id) ?? null)
      : null,
  );
}

/** the workspace this window is in, or null on the home screen */
export function workspaceOf(view: View, session: SessionView | null): string | null {
  switch (view.type) {
    case "home":
      return null;
    case "workspace":
      return view.id;
    case "document":
      return session?.documents.find((doc) => doc.id === view.id)?.workspace ?? null;
  }
}

export function useWorkspaceId(): string | null {
  const view = useView();
  return useSession((state) => workspaceOf(view, state.session));
}

export type Params = Record<string, JsonValue>;

/** runs a shell command from the backend registry and applies the result */
export async function runShell(id: string, params: Params = {}): Promise<void> {
  const view = await perform((api) => api.shell.run({ id, params }));
  if (view) useSession.getState().applyView(view);
}

function windowLabel(): string {
  return useApp.getState().windowLabel;
}

export const shell = {
  activate: (id: DocumentId) => runShell("shell.activate", { id }),
  close: (id: DocumentId) => runShell("shell.close", { id }),
  home: () => runShell("shell.home", { window: windowLabel() }),
  workspace: (workspace: string) =>
    runShell("shell.workspace", { window: windowLabel(), workspace }),
  moveTab: (id: DocumentId, index: number) => runShell("shell.move_tab", { id, index }),
  toWindow: (id: DocumentId, window: string | null, position?: { x: number; y: number }) =>
    runShell("shell.move_to_window", { id, window, position: position ?? null }),
};

/** what the user is told about files that could not be opened */
export function skippedNotice(plan: OpenPlan): string | null {
  const [first, ...rest] = plan.skipped;
  if (!first) return null;
  if (rest.length === 0) return `${first.name} was not opened. ${first.reason}`;
  return `${plan.skipped.length} files were not opened. ${first.name}: ${first.reason}`;
}

/** opens files in this window, saying which ones it could not */
export async function openFiles(
  paths: string[],
  target: OpenTarget = { type: "auto" },
): Promise<void> {
  if (paths.length === 0) return;
  const plan = await perform((api) => api.files.plan(paths, target));
  if (!plan) return;
  const notice = skippedNotice(plan);
  if (notice) useApp.getState().setNotice(notice);
  if (plan.opens.length === 0) return;
  await runShell("shell.open", { paths, target, window: windowLabel() });
}

/** asks for files with the system dialog, then opens them */
export async function pickAndOpen(target?: OpenTarget): Promise<void> {
  const paths = await perform((api) => api.files.pick());
  if (paths && paths.length > 0) await openFiles(paths, target);
}

export async function restoreSession(): Promise<void> {
  const view = await perform((api) => api.recovery.restore());
  if (view) useSession.getState().applyView(view);
  useSession.getState().setRecovery(null);
}

export async function discardRecovery(): Promise<void> {
  await perform((api) => api.recovery.discard());
  useSession.getState().setRecovery(null);
}
