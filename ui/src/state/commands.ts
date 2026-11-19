import { create } from "zustand";

import type { CommandInfo } from "../bindings/CommandInfo";
import type { Keymap } from "../bindings/Keymap";
import { inDesktopApp, ipc } from "../lib/ipc";
import { useDocuments } from "./documents";
import { useUi } from "./ui";

const RECENT_KEY = "vellum.recentCommands";

function loadRecent(): string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

interface CommandsStore {
  /** every app command rust registered, for the palette and menus */
  infos: CommandInfo[];
  keymap: Keymap | null;
  /** commands run lately, newest first. a palette convenience, kept in this window only. */
  recent: string[];
  setInfos: (infos: CommandInfo[]) => void;
  setKeymap: (keymap: Keymap) => void;
  noteUsed: (kind: string) => void;
}

export const useCommands = create<CommandsStore>()((set) => ({
  infos: [],
  keymap: null,
  recent: loadRecent(),
  setInfos: (infos) => {
    set({ infos });
  },
  setKeymap: (keymap) => {
    set({ keymap });
  },
  noteUsed: (kind) => {
    set((state) => {
      const recent = [kind, ...state.recent.filter((k) => k !== kind)].slice(0, 8);
      try {
        localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
      } catch {
        // storage can be full or blocked, the list is only a convenience
      }
      return { recent };
    });
  },
}));

export function shortcutFor(
  kind: string,
  keymap = useCommands.getState().keymap,
): string | undefined {
  return keymap?.bindings[kind]?.[0];
}

export function commandInfo(kind: string): CommandInfo | undefined {
  return useCommands.getState().infos.find((info) => info.kind === kind);
}

type UiHandler = () => void | Promise<void>;
const uiHandlers = new Map<string, UiHandler>();

/** lets the shell perform ui actions that commands hand back. returns a cleanup. */
export function registerUiHandlers(handlers: Record<string, UiHandler>): () => void {
  for (const [kind, handler] of Object.entries(handlers)) uiHandlers.set(kind, handler);
  return () => {
    for (const [kind, handler] of Object.entries(handlers)) {
      if (uiHandlers.get(kind) === handler) uiHandlers.delete(kind);
    }
  };
}

async function performUiAction(kind: string) {
  const handler = uiHandlers.get(kind);
  if (handler) await handler();
}

/**
 * runs an app command through rust, then performs the ui actions it hands back. every
 * menu item, shortcut, palette entry and button uses this, so they all behave the same and
 * a macro recorder sees them all.
 */
export async function runCommand(kind: string): Promise<void> {
  useCommands.getState().noteUsed(kind);
  if (!inDesktopApp) {
    // no rust in a plain browser, so only ui actions can work
    await performUiAction(kind);
    return;
  }
  try {
    const outcome = await ipc.runCommand(kind, useDocuments.getState().activeId);
    for (const action of outcome.uiActions) await performUiAction(action.kind);
    if (outcome.message) useUi.getState().toast({ level: "info", title: outcome.message });
  } catch (err) {
    useUi.getState().toast({
      level: "error",
      title: `${commandInfo(kind)?.title ?? kind} did not work`,
      body: `${String(err)}. Try again, or restart Vellum if it keeps happening.`,
    });
  }
}
