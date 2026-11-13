import { create } from "zustand";

import type { OpenPlan } from "../bindings/OpenPlan";
import type { OpenTarget } from "../bindings/OpenTarget";
import type { Backend, FileDropEvent } from "../lib/backend";
import { perform } from "../stores/app";
import { openFiles } from "../stores/session";
import { workspaceById } from "../workspaces";
import { quickToolById } from "./quickTools";

/**
 * files dragged in from the desktop. anything with a `data-drop` attribute
 * can be a target: "tool:video-to-gif" for a quick tool card, or
 * "workspace:video" for a workspace. anywhere else opens each file in the
 * workspace its type belongs to.
 */

interface DropState {
  /** the files being dragged, or null when nothing is */
  paths: string[] | null;
  /** the `data-drop` value under the pointer, or null for "anywhere" */
  targetKey: string | null;
  /** what dropping right here would do, once the backend has said */
  plan: OpenPlan | null;
}

export const useDrop = create<DropState>()(() => ({
  paths: null,
  targetKey: null,
  plan: null,
}));

export function targetFromKey(key: string | null): OpenTarget {
  const [kind, id] = key?.split(":", 2) ?? [];
  if (kind === "tool" && id) return { type: "tool", id };
  if (kind === "workspace" && id) return { type: "workspace", id };
  return { type: "auto" };
}

export function targetKeyAt(x: number, y: number): string | null {
  const element = document.elementFromPoint(x, y);
  const target = element?.closest<HTMLElement>("[data-drop]");
  return target?.dataset.drop ?? null;
}

export interface DropSummary {
  /** false when nothing would open */
  ok: boolean;
  headline: string;
  detail: string | null;
}

function names(plan: OpenPlan): string {
  const [first, second, ...rest] = plan.opens;
  if (!first) return "";
  if (!second) return first.name;
  if (rest.length === 0) return `${first.name} and ${second.name}`;
  return `${first.name}, ${second.name} and ${rest.length} more`;
}

/** what the overlay says about a plan, in plain words */
export function describeDrop(plan: OpenPlan, target: OpenTarget): DropSummary {
  const skipped = plan.skipped[0];
  if (plan.opens.length === 0) {
    return {
      ok: false,
      headline:
        plan.skipped.length > 1 ? "These files can't be opened" : "This file can't be opened",
      detail: skipped?.reason ?? null,
    };
  }

  let headline: string;
  if (target.type === "tool") {
    headline = quickToolById(target.id)?.dropText ?? "Open in the tool";
  } else {
    const places = [...new Set(plan.opens.map((open) => open.workspace))];
    const count = plan.opens.length;
    const what = count === 1 ? "" : ` ${count} files`;
    headline =
      places.length === 1 && places[0]
        ? `Open${what} in ${workspaceById(places[0]).title}`
        : `Open${what} in ${places.length} workspaces`;
  }

  let detail = names(plan);
  if (skipped) {
    const more = plan.skipped.length > 1 ? ` and ${plan.skipped.length - 1} more` : "";
    detail += `. ${skipped.name}${more} will be skipped: ${skipped.reason}`;
  }
  return { ok: true, headline, detail };
}

let planToken = 0;

async function loadPlan(paths: string[], key: string | null): Promise<void> {
  const token = ++planToken;
  const plan = await perform((api) => api.files.plan(paths, targetFromKey(key)));
  // a newer target may have been hovered while this one was asked about
  if (token === planToken && plan) useDrop.setState({ plan });
}

/** the handler for file drags from the desktop, wired up once per window */
export function handleFileDrop(event: FileDropEvent): void {
  const state = useDrop.getState();
  switch (event.type) {
    case "enter": {
      const targetKey = targetKeyAt(event.x, event.y);
      useDrop.setState({ paths: event.paths, targetKey, plan: null });
      void loadPlan(event.paths, targetKey);
      break;
    }
    case "over": {
      if (!state.paths) return;
      const targetKey = targetKeyAt(event.x, event.y);
      if (targetKey !== state.targetKey) {
        useDrop.setState({ targetKey, plan: null });
        void loadPlan(state.paths, targetKey);
      }
      break;
    }
    case "drop": {
      const targetKey = targetKeyAt(event.x, event.y);
      planToken++;
      useDrop.setState({ paths: null, targetKey: null, plan: null });
      void openFiles(event.paths, targetFromKey(targetKey));
      break;
    }
    case "leave":
      planToken++;
      useDrop.setState({ paths: null, targetKey: null, plan: null });
      break;
  }
}

export function listenForFileDrops(backend: Backend): Promise<() => void> {
  return backend.onFileDrop(handleFileDrop);
}
