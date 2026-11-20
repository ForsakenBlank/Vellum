// files dragged onto the window. tauri reports them with real paths (html drag and drop
// only gives names), along with the pointer position in physical pixels. the plan for the
// drop is worked out as the pointer moves, so the overlay can say what will happen.

import { getCurrentWebview } from "@tauri-apps/api/webview";

import { inDesktopApp, ipc } from "../lib/ipc";
import { planDrop, type DropTarget } from "../lib/routing";
import { useUi } from "../state/ui";
import type { QuickToolId } from "../workspaces/quickTools";
import { openPaths } from "./actions";

/** the quick tool card or open session under a point, if any */
export function targetAt(x: number, y: number): DropTarget {
  for (const el of document.elementsFromPoint(x, y)) {
    const session =
      el instanceof HTMLElement ? el.closest<HTMLElement>("[data-drop-session]") : null;
    if (session?.dataset.dropSession && session.dataset.dropSessionTool) {
      return {
        type: "session",
        document: session.dataset.dropSession,
        tool: session.dataset.dropSessionTool as QuickToolId,
      };
    }
    const tool = el instanceof HTMLElement ? el.closest<HTMLElement>("[data-drop-tool]") : null;
    if (tool?.dataset.dropTool) return { type: "tool", tool: tool.dataset.dropTool as QuickToolId };
  }
  return { type: "window" };
}

let dragged: string[] = [];

function update(physical: { x: number; y: number }) {
  const scale = window.devicePixelRatio || 1;
  const target = targetAt(physical.x / scale, physical.y / scale);
  useUi
    .getState()
    .setDrop(planDrop(dragged, target), target.type === "window" ? null : target.tool);
}

export async function performDrop(paths: string[], target: DropTarget) {
  const plan = planDrop(paths, target);
  useUi.getState().setDrop(null, null);
  dragged = [];
  switch (plan.action) {
    case "open":
      await openPaths(plan.files);
      break;
    case "tool":
      await openPaths(plan.files, plan.tool);
      break;
    case "add":
      await ipc
        .documentCommand(plan.document, "tool.add_inputs", { paths: plan.files })
        .catch((err: unknown) => {
          useUi
            .getState()
            .toast({ level: "error", title: "The files were not added", body: String(err) });
        });
      break;
    case "reject":
      useUi.getState().toast({ level: "warning", title: plan.title, body: plan.detail });
      break;
  }
}

/** listens for files dragged onto this window. returns a cleanup. */
export async function listenForFileDrops(): Promise<() => void> {
  if (!inDesktopApp) return () => undefined;
  return getCurrentWebview().onDragDropEvent(({ payload }) => {
    switch (payload.type) {
      case "enter":
        dragged = payload.paths;
        update(payload.position);
        break;
      case "over":
        update(payload.position);
        break;
      case "drop": {
        const scale = window.devicePixelRatio || 1;
        const target = targetAt(payload.position.x / scale, payload.position.y / scale);
        void performDrop(payload.paths, target);
        break;
      }
      case "leave":
        dragged = [];
        useUi.getState().setDrop(null, null);
        break;
    }
  });
}
