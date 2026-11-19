// the things the shell does on the ui side: opening files, closing tabs, moving tabs
// between windows and changing the dock layout. commands, menus, drops and buttons all
// call these, so each behaviour lives in one place.

import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { AppSettings } from "../bindings/AppSettings";
import type { DockLayout } from "../bindings/DockLayout";
import type { OpenFilesResult } from "../bindings/OpenFilesResult";
import {
  closePanel,
  layoutPresets,
  normalizeLayout,
  showPanel,
  toggleDocks,
  togglePanel,
  type AreaName,
} from "../lib/dock";
import { inDesktopApp, ipc } from "../lib/ipc";
import { fileKinds, fileName } from "../lib/routing";
import { isMainWindow, windowLabel } from "../lib/window";
import { activeDocument, tabsIn, useDocuments } from "../state/documents";
import { useSettings } from "../state/settings";
import { askConfirm, useUi } from "../state/ui";
import { quickTool, type QuickToolId } from "../workspaces/quickTools";
import { panelIds, panels } from "./panels/registry";

function fail(title: string, err: unknown) {
  useUi.getState().toast({
    level: "error",
    title,
    body: `${String(err)}. Try again, or restart Vellum if it keeps happening.`,
  });
}

function reportSkipped(result: OpenFilesResult) {
  const [first, ...rest] = result.skipped;
  if (!first) return;
  useUi.getState().toast({
    level: "warning",
    title:
      rest.length === 0
        ? `${fileName(first.path)} was not opened`
        : `${result.skipped.length} files were not opened`,
    body: first.reason,
  });
}

/** opens files in the right workspaces, or in one quick tool, and shows the first one */
export async function openPaths(paths: string[], tool: QuickToolId | null = null) {
  try {
    const result = await ipc.openFiles(paths, tool, windowLabel);
    // rust sends documents:changed before replying, so the tabs are already there
    const first = result.opened[0];
    if (first) useDocuments.getState().activate(first.id);
    reportSkipped(result);
  } catch (err) {
    fail("The files could not be opened", err);
  }
}

/** asks for files with the system dialog, then opens them */
export async function pickAndOpen(tool: QuickToolId | null = null) {
  if (!inDesktopApp) return;
  const extensions = fileKinds.flatMap((row) => row.extensions);
  try {
    const picked = await open({
      multiple: true,
      title: tool ? `Open files in ${quickTool(tool)?.title ?? "the tool"}` : "Open files",
      filters: [{ name: "Media Vellum can open", extensions }],
    });
    if (picked && picked.length > 0) await openPaths(picked, tool);
  } catch (err) {
    fail("The file picker could not open", err);
  }
}

/** opens a quick tool with no files yet */
export async function openTool(tool: QuickToolId) {
  await openPaths([], tool);
}

/** closes a tab, asking first if it has unsaved changes */
export async function closeTab(id: string | null = useDocuments.getState().activeId) {
  if (!id) return;
  const doc = useDocuments.getState().documents.find((d) => d.id === id);
  if (!doc) return;
  if (doc.dirty) {
    const answer = await askConfirm({
      title: `Close ${doc.title}?`,
      body: "It has changes that have not been saved. Closing throws them away.",
      confirmLabel: "Close without saving",
      altLabel: "Save and close",
      danger: true,
    });
    if (answer === "cancel") return;
    if (answer === "alt") {
      try {
        await ipc.runCommand("file.save", id);
      } catch (err) {
        fail(`${doc.title} could not be saved`, err);
        return;
      }
    }
  }
  try {
    await ipc.closeDocument(id);
  } catch (err) {
    fail(`${doc.title} could not be closed`, err);
  }
}

/** activates the tab to the left or right, wrapping around, with home as the first stop */
export function cycleTab(step: 1 | -1) {
  const state = useDocuments.getState();
  const ids: (string | null)[] = [
    ...(isMainWindow ? [null] : []),
    ...tabsIn(state.documents).map((d) => d.id),
  ];
  if (ids.length === 0) return;
  const at = ids.indexOf(state.activeId);
  const next = ids[(at + step + ids.length) % ids.length] ?? null;
  if (next === null) state.showHome();
  else state.activate(next);
}

export async function tearOff(id: string | null = useDocuments.getState().activeId) {
  if (!id) return;
  try {
    await ipc.tearOffDocument(id);
  } catch (err) {
    fail("The tab could not move to a new window", err);
  }
}

export async function moveToMain(id: string | null = useDocuments.getState().activeId) {
  if (!id || isMainWindow) return;
  try {
    await ipc.moveDocument(id, "main", null);
  } catch (err) {
    fail("The tab could not move", err);
  }
}

/** a torn off window closes itself once its last tab has gone */
export function closeWindowIfEmpty() {
  if (isMainWindow || !inDesktopApp) return;
  const { documents, loaded } = useDocuments.getState();
  if (loaded && tabsIn(documents).length === 0) {
    void getCurrentWindow().close();
  }
}

export function currentLayout(
  settings: AppSettings | null = useSettings.getState().settings,
): DockLayout {
  const preset = layoutPresets[0];
  const fallback = preset ? preset.layout : null;
  const layout = settings?.layout.current ?? fallback;
  if (!layout) throw new Error("no layout presets");
  return normalizeLayout(layout, panelIds);
}

/** saves a new layout. `active` names the preset or saved layout it now matches. */
export function setLayout(layout: DockLayout, active?: string) {
  const update = useSettings.getState().update;
  void update({ layout: { current: layout, ...(active === undefined ? {} : { active }) } });
}

export function homeOf(panel: string): AreaName {
  return panels.find((p) => p.id === panel)?.home ?? "right";
}

export function revealPanel(panel: string) {
  setLayout(showPanel(currentLayout(), panel, homeOf(panel)));
}

export function flipPanel(panel: string) {
  setLayout(togglePanel(currentLayout(), panel, homeOf(panel)));
}

export function hidePanel(panel: string) {
  setLayout(closePanel(currentLayout(), panel));
}

export function flipDocks() {
  setLayout(toggleDocks(currentLayout()));
}

export function applyLayout(name: string) {
  const preset = layoutPresets.find((p) => p.id === name);
  const saved = useSettings.getState().settings?.layout.saved.find((s) => s.name === name);
  const layout = preset?.layout ?? saved?.layout;
  if (layout) setLayout(normalizeLayout(layout, panelIds), name);
}

export function saveLayoutAs(name: string) {
  const settings = useSettings.getState().settings;
  if (!settings) return;
  const clean = name.trim();
  if (!clean) return;
  const saved = [
    ...settings.layout.saved.filter((s) => s.name !== clean),
    { name: clean, layout: currentLayout(settings) },
  ];
  void useSettings.getState().update({ layout: { saved, active: clean } });
  useUi.getState().toast({ level: "success", title: `Saved the layout as ${clean}` });
}

export function deleteLayout(name: string) {
  const settings = useSettings.getState().settings;
  if (!settings) return;
  const saved = settings.layout.saved.filter((s) => s.name !== name);
  void useSettings.getState().update({ layout: { saved } });
}

/** the name to show for the active document, or the window's purpose */
export function activeTitle(): string {
  return activeDocument(useDocuments.getState())?.title ?? "Home";
}
