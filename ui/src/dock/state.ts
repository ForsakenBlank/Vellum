import { useMemo } from "react";
import { create } from "zustand";

import type { DockLayout } from "../bindings/DockLayout";
import type { LayoutSettings } from "../bindings/LayoutSettings";
import { useApp } from "../stores/app";
import { updateSettings, useSettings } from "../stores/settings";
import type { Workspace } from "../workspaces/types";
import { anyZoneOpen, keepKnown, setAllZones, togglePanel, type DropSpot } from "./layout";

/** the panel being dragged and where it would land, while a drag is on */
export interface PanelDrag {
  panel: string;
  title: string;
  x: number;
  y: number;
  spot: DropSpot | null;
}

interface DockState {
  /** a layout being changed by a gesture, shown until the change is saved */
  preview: { workspace: string; layout: DockLayout } | null;
  drag: PanelDrag | null;
  setPreview: (preview: DockState["preview"]) => void;
  setDrag: (drag: PanelDrag | null) => void;
}

export const useDock = create<DockState>()((set) => ({
  preview: null,
  drag: null,
  setPreview: (preview) => {
    set({ preview });
  },
  setDrag: (drag) => {
    set({ drag });
  },
}));

/** the layout a workspace is in: what the user left it as, or its default */
export function layoutFor(workspace: Workspace, layouts: LayoutSettings | undefined): DockLayout {
  const saved = layouts?.current[workspace.id];
  if (!saved) return workspace.layout;
  return keepKnown(saved, new Set(workspace.panels.map((panel) => panel.id)));
}

export function useLayout(workspace: Workspace): DockLayout {
  const layouts = useSettings((state) => state.sections?.layouts);
  const preview = useDock((state) => state.preview);
  return useMemo(
    () => (preview?.workspace === workspace.id ? preview.layout : layoutFor(workspace, layouts)),
    [preview, workspace, layouts],
  );
}

function layoutSettings(): LayoutSettings {
  return useSettings.getState().sections?.layouts ?? { current: {}, saved: [] };
}

/** saves a workspace's layout. the preview stays up until the save comes back */
export async function saveLayout(workspace: Workspace, layout: DockLayout): Promise<void> {
  const settings = layoutSettings();
  if (useApp.getState().status !== "ready") {
    // without a backend there is nowhere to save to, so keep showing the change
    useDock.getState().setPreview({ workspace: workspace.id, layout });
    return;
  }
  useDock.getState().setPreview({ workspace: workspace.id, layout });
  await updateSettings("layouts", { current: { ...settings.current, [workspace.id]: layout } });
  useDock.getState().setPreview(null);
}

export function changeLayout(
  workspace: Workspace,
  change: (layout: DockLayout) => DockLayout,
): Promise<void> {
  const preview = useDock.getState().preview;
  const current =
    preview?.workspace === workspace.id ? preview.layout : layoutFor(workspace, layoutSettings());
  return saveLayout(workspace, change(current));
}

export function resetLayout(workspace: Workspace): Promise<void> {
  return saveLayout(workspace, workspace.layout);
}

export function togglePanelIn(workspace: Workspace, panel: string): Promise<void> {
  return changeLayout(workspace, (layout) => togglePanel(layout, panel, workspace.layout));
}

/** hides every dock zone, or brings them all back */
export function toggleAllPanels(workspace: Workspace): Promise<void> {
  return changeLayout(workspace, (layout) => setAllZones(layout, !anyZoneOpen(layout)));
}

export function savedLayouts(workspace: Workspace, layouts: LayoutSettings | undefined) {
  return (layouts?.saved ?? []).filter((saved) => saved.workspace === workspace.id);
}

export async function saveNamedLayout(workspace: Workspace, name: string): Promise<void> {
  const settings = layoutSettings();
  const layout = layoutFor(workspace, settings);
  const others = settings.saved.filter(
    (saved) => !(saved.workspace === workspace.id && saved.name === name.trim()),
  );
  await updateSettings("layouts", {
    saved: [...others, { name: name.trim(), workspace: workspace.id, layout }],
  });
}

export async function deleteNamedLayout(workspace: Workspace, name: string): Promise<void> {
  const settings = layoutSettings();
  await updateSettings("layouts", {
    saved: settings.saved.filter(
      (saved) => !(saved.workspace === workspace.id && saved.name === name),
    ),
  });
}
