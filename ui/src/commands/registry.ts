import { useMemo } from "react";

import type { CommandSpec } from "../bindings/CommandSpec";
import type { ShortcutSettings } from "../bindings/ShortcutSettings";
import type { Theme } from "../bindings/Theme";
import { resetLayout, toggleAllPanels, togglePanelIn } from "../dock/state";
import { useApp } from "../stores/app";
import { useBackendCommands } from "../stores/commands";
import {
  activeDocument,
  pickAndOpen,
  type Params,
  runShell,
  shell,
  useSession,
  workspaceOf,
} from "../stores/session";
import { updateSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { workspaceById, workspaces } from "../workspaces";
import { buildKeymap, type Keymap, type Scope } from "./shortcuts";

/**
 * every action the palette can find and a shortcut can run.
 *
 * actions that change shared state (opening and closing tabs, tearing them
 * off) are backend commands from the vellum-core registry: their titles and
 * default keys come from rust, and this file only knows how to fill in their
 * parameters from what is on screen. actions that only touch this window
 * (opening the palette, changing the theme, showing a panel) live here.
 */
export interface AppCommand {
  id: string;
  title: string;
  category: string;
  keywords: string[];
  /** the default key, before presets and the user's changes */
  shortcut: string | null;
  scope: Scope;
  /** false for commands that only make sense from a button with context */
  palette: boolean;
  /** false when there is nothing to act on right now */
  enabled: () => boolean;
  run: () => unknown;
}

function windowLabel(): string {
  return useApp.getState().windowLabel;
}

function currentWorkspace() {
  const { session } = useSession.getState();
  const view = session?.windows.find((window) => window.label === windowLabel())?.view;
  return workspaceById(view ? workspaceOf(view, session) : null);
}

/** fills in a backend command's parameters, or says it cannot run right now */
const binders: Record<string, () => Params | null> = {
  "shell.close": () => {
    const doc = activeDocument();
    return doc ? { id: doc.id } : null;
  },
  "shell.reopen_closed": () =>
    (useSession.getState().session?.closedCount ?? 0) > 0 ? { window: windowLabel() } : null,
  "shell.home": () => ({ window: windowLabel() }),
  "shell.next_tab": () => ({ window: windowLabel() }),
  "shell.previous_tab": () => ({ window: windowLabel() }),
  "shell.move_to_window": () => {
    const doc = activeDocument();
    return doc ? { id: doc.id, window: null, position: null } : null;
  },
};

function fromBackend(spec: CommandSpec): AppCommand | null {
  const bind = binders[spec.id];
  if (!bind) return null;
  return {
    id: spec.id,
    title: spec.title,
    category: spec.category ?? "General",
    keywords: spec.keywords,
    shortcut: spec.shortcut,
    scope: "global",
    palette: spec.palette,
    enabled: () => bind() !== null,
    run: () => {
      const params = bind();
      return params ? runShell(spec.id, params) : undefined;
    },
  };
}

function command(
  id: string,
  title: string,
  category: string,
  run: () => unknown,
  extra: Partial<Omit<AppCommand, "id" | "title" | "category" | "run">> = {},
): AppCommand {
  return {
    id,
    title,
    category,
    run,
    keywords: [],
    shortcut: null,
    scope: "global",
    palette: true,
    enabled: () => true,
    ...extra,
  };
}

const connected = () => useApp.getState().status === "ready";

function setTheme(theme: Theme) {
  return () => updateSettings("appearance", { theme });
}

function zoomBy(step: number) {
  return () => {
    const scale = useSettings.getState().sections?.appearance.uiScale ?? 1;
    const next = step === 0 ? 1 : Math.round((scale + step) * 20) / 20;
    return updateSettings("appearance", { uiScale: next });
  };
}

function appearanceToggle(key: "compact" | "reducedMotion") {
  return () => {
    const appearance = useSettings.getState().sections?.appearance;
    if (appearance) return updateSettings("appearance", { [key]: !appearance[key] });
    return undefined;
  };
}

/** commands that belong to this window rather than to the backend */
function uiCommands(): AppCommand[] {
  const ui = useUi.getState;
  const list: AppCommand[] = [
    command(
      "app.palette",
      "Command palette",
      "General",
      () => {
        ui().setPaletteOpen(true);
      },
      {
        shortcut: "Mod+K",
        keywords: ["search", "find", "actions"],
        palette: false,
      },
    ),
    command(
      "app.settings",
      "Settings",
      "General",
      () => {
        ui().openSettings();
      },
      {
        shortcut: "Mod+,",
        keywords: ["preferences", "options"],
      },
    ),
    command(
      "app.shortcuts",
      "Keyboard shortcuts",
      "General",
      () => {
        ui().openSettings("shortcuts");
      },
      { keywords: ["keys", "hotkeys", "keybindings"] },
    ),
    command(
      "app.notifications",
      "Notifications",
      "General",
      () => {
        ui().setNotificationsOpen(true);
      },
      { shortcut: "Mod+Shift+N", keywords: ["alerts", "messages"] },
    ),
    command("file.open", "Open file", "File", () => pickAndOpen(), {
      shortcut: "Mod+O",
      keywords: ["import", "load"],
      enabled: connected,
    }),
    command("view.theme.dark", "Use dark theme", "Appearance", setTheme("dark"), {
      keywords: ["night", "colour", "color"],
    }),
    command("view.theme.light", "Use light theme", "Appearance", setTheme("light"), {
      keywords: ["day", "colour", "color"],
    }),
    command(
      "view.theme.high_contrast",
      "Use high contrast theme",
      "Appearance",
      setTheme("highContrast"),
      { keywords: ["accessibility", "a11y", "colour", "color"] },
    ),
    command("view.compact", "Toggle compact mode", "Appearance", appearanceToggle("compact"), {
      keywords: ["dense", "density", "spacing"],
    }),
    command(
      "view.reduced_motion",
      "Toggle reduced motion",
      "Appearance",
      appearanceToggle("reducedMotion"),
      { keywords: ["animation", "accessibility"] },
    ),
    command("view.ui_zoom_in", "Make the interface bigger", "Appearance", zoomBy(0.05), {
      shortcut: "Mod+=",
      keywords: ["ui scale", "zoom", "larger"],
    }),
    command("view.ui_zoom_out", "Make the interface smaller", "Appearance", zoomBy(-0.05), {
      shortcut: "Mod+-",
      keywords: ["ui scale", "zoom", "smaller"],
    }),
    command("view.ui_zoom_reset", "Reset interface size", "Appearance", zoomBy(0), {
      shortcut: "Mod+0",
      keywords: ["ui scale", "zoom", "100%"],
    }),
    command(
      "view.panels",
      "Hide or show all panels",
      "Panels",
      () => toggleAllPanels(currentWorkspace()),
      { shortcut: "Mod+\\", keywords: ["focus", "distraction free", "fullscreen"] },
    ),
    command(
      "view.layout.reset",
      "Reset panel layout",
      "Panels",
      () => resetLayout(currentWorkspace()),
      { keywords: ["default layout", "restore panels"] },
    ),
  ];

  workspaces.forEach((workspace, index) => {
    list.push(
      command(
        `go.${workspace.id}`,
        `Go to ${workspace.title}`,
        "Go",
        () => shell.workspace(workspace.id),
        {
          shortcut: `Mod+${index + 1}`,
          keywords: ["workspace", workspace.summary],
          enabled: connected,
        },
      ),
    );
  });

  // a panel toggle for each panel the current workspace offers
  const here = currentWorkspace();
  for (const panel of here.panels) {
    list.push(
      command(
        `view.panel.${panel.id}`,
        `Show or hide ${panel.title}`,
        "Panels",
        () => togglePanelIn(currentWorkspace(), panel.id),
        {
          shortcut: panel.id === "jobs" ? "Mod+J" : null,
          keywords: ["panel", "window", panel.title],
        },
      ),
    );
  }
  return list;
}

/** every command, in the order the palette lists ties */
export function allCommands(
  backend: CommandSpec[] = useBackendCommands.getState().backend,
): AppCommand[] {
  const fromRust = backend.map(fromBackend).filter((entry): entry is AppCommand => entry !== null);
  return [...uiCommands(), ...fromRust];
}

/**
 * every command the shortcut editor can bind. panel toggles are listed once
 * for every panel any workspace has, since keys are set app wide
 */
export function bindableCommands(
  backend: CommandSpec[] = useBackendCommands.getState().backend,
): AppCommand[] {
  const all = allCommands(backend).filter((entry) => !entry.id.startsWith("view.panel."));
  const seen = new Set<string>();
  for (const workspace of workspaces) {
    for (const panel of workspace.panels) {
      if (seen.has(panel.id)) continue;
      seen.add(panel.id);
      all.push(
        command(
          `view.panel.${panel.id}`,
          `Show or hide ${panel.title}`,
          "Panels",
          () => togglePanelIn(currentWorkspace(), panel.id),
          { shortcut: panel.id === "jobs" ? "Mod+J" : null },
        ),
      );
    }
  }
  return all;
}

export function currentKeymap(
  shortcuts: ShortcutSettings | null = useSettings.getState().sections?.shortcuts ?? null,
  backend: CommandSpec[] = useBackendCommands.getState().backend,
): Keymap {
  return buildKeymap(bindableCommands(backend), shortcuts);
}

/** the keymap as it is set right now, redrawn when the settings or commands change */
export function useKeymap(): Keymap {
  const shortcuts = useSettings((state) => state.sections?.shortcuts ?? null);
  const backend = useBackendCommands((state) => state.backend);
  return useMemo(() => currentKeymap(shortcuts, backend), [shortcuts, backend]);
}

/** the first key of a command as it is set right now, for tooltips */
export function useShortcut(id: string): string | null {
  return useKeymap()[id]?.[0] ?? null;
}
