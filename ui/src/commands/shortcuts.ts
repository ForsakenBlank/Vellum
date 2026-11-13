import type { ShortcutPreset } from "../bindings/ShortcutPreset";
import type { ShortcutSettings } from "../bindings/ShortcutSettings";
import { normalizeShortcut } from "../lib/keys";

/**
 * which keys run which command.
 *
 * three layers, each on top of the one before: the default each command
 * declares, the preset the user picked, and the user's own changes. a layer
 * that names a command replaces that command's keys entirely.
 */

/** where a command works. global commands work everywhere */
export type Scope = "global" | (string & {});

export interface Bindable {
  id: string;
  title: string;
  shortcut: string | null;
  scope: Scope;
}

export type Keymap = Record<string, string[]>;

export interface PresetInfo {
  label: string;
  description: string;
  bindings: Keymap;
}

/**
 * presets only list what differs from vellum's own keys. they will grow as
 * the editors arrive and there are more commands to map
 */
export const PRESETS: Record<ShortcutPreset, PresetInfo> = {
  vellum: {
    label: "Vellum",
    description: "Vellum's own shortcuts.",
    bindings: {},
  },
  photoshop: {
    label: "Photoshop",
    description: "Preferences on Ctrl+K and Tab to hide panels, as in Photoshop.",
    bindings: {
      "app.settings": ["Mod+K"],
      "app.palette": ["Mod+Shift+P"],
      "view.panels": ["Tab"],
      "view.ui_zoom_in": ["Mod+Alt+="],
      "view.ui_zoom_out": ["Mod+Alt+-"],
    },
  },
  premiere: {
    label: "Premiere Pro",
    description: "Workspaces on Alt+Shift and numbers, as in Premiere Pro.",
    bindings: {
      "go.quick-tools": ["Alt+Shift+1"],
      "go.image": ["Alt+Shift+2"],
      "go.video": ["Alt+Shift+3"],
      "go.audio": ["Alt+Shift+4"],
      "go.color": ["Alt+Shift+5"],
      "go.motion": ["Alt+Shift+6"],
      "go.vector": ["Alt+Shift+7"],
      "view.panels": ["`"],
      "file.open": ["Mod+O", "Mod+I"],
    },
  },
  resolve: {
    label: "DaVinci Resolve",
    description: "Pages on Shift and numbers, as in DaVinci Resolve.",
    bindings: {
      "go.quick-tools": ["Shift+1"],
      "go.image": ["Shift+2"],
      "go.video": ["Shift+4"],
      "go.motion": ["Shift+5"],
      "go.color": ["Shift+6"],
      "go.audio": ["Shift+7"],
      "go.vector": ["Shift+8"],
      "view.panels": ["Mod+F"],
    },
  },
  audacity: {
    label: "Audacity",
    description: "Preferences on Ctrl+P, as in Audacity.",
    bindings: {
      "app.settings": ["Mod+P"],
      "file.open": ["Mod+O"],
    },
  },
};

export const PRESET_ORDER: ShortcutPreset[] = [
  "vellum",
  "photoshop",
  "premiere",
  "resolve",
  "audacity",
];

function normalized(keys: string[]): string[] {
  return [...new Set(keys.map(normalizeShortcut))];
}

/** the keys of every command after defaults, preset and the user's changes */
export function buildKeymap(commands: Bindable[], settings: ShortcutSettings | null): Keymap {
  const preset = PRESETS[settings?.preset ?? "vellum"].bindings;
  const own = settings?.bindings ?? {};
  const keymap: Keymap = {};
  for (const command of commands) {
    const keys =
      own[command.id] ?? preset[command.id] ?? (command.shortcut ? [command.shortcut] : []);
    keymap[command.id] = normalized(keys);
  }
  return keymap;
}

/** the keys a command would have without the user's own changes */
export function presetKeys(command: Bindable, preset: ShortcutPreset): string[] {
  const keys = PRESETS[preset].bindings[command.id] ?? (command.shortcut ? [command.shortcut] : []);
  return normalized(keys);
}

/** two commands clash if they share a key and can both be reached at the same time */
export function scopesOverlap(a: Scope, b: Scope): boolean {
  return a === "global" || b === "global" || a === b;
}

/** the other commands that already use `shortcut` where `command` works */
export function clashesWith<T extends Bindable>(
  shortcut: string,
  command: Bindable,
  commands: T[],
  keymap: Keymap,
): T[] {
  const key = normalizeShortcut(shortcut);
  return commands.filter(
    (other) =>
      other.id !== command.id &&
      scopesOverlap(other.scope, command.scope) &&
      (keymap[other.id] ?? []).includes(key),
  );
}

export interface Conflict {
  shortcut: string;
  commands: Bindable[];
}

/** every key that runs more than one command in the same place */
export function findConflicts(commands: Bindable[], keymap: Keymap): Conflict[] {
  const byKey = new Map<string, Bindable[]>();
  for (const command of commands) {
    for (const key of keymap[command.id] ?? []) {
      byKey.set(key, [...(byKey.get(key) ?? []), command]);
    }
  }
  const conflicts: Conflict[] = [];
  for (const [shortcut, users] of byKey) {
    const clashing = users.filter((a) =>
      users.some((b) => a !== b && scopesOverlap(a.scope, b.scope)),
    );
    if (clashing.length > 1) conflicts.push({ shortcut, commands: clashing });
  }
  return conflicts;
}

/**
 * the user's bindings after giving `shortcut` to `command`. with `steal` the
 * commands that clash lose it, otherwise both keep it and the conflict shows
 */
export function assignShortcut(
  settings: ShortcutSettings,
  commands: Bindable[],
  command: Bindable,
  shortcut: string,
  steal: boolean,
): ShortcutSettings["bindings"] {
  const keymap = buildKeymap(commands, settings);
  const key = normalizeShortcut(shortcut);
  const bindings = { ...settings.bindings };
  bindings[command.id] = normalized([...(keymap[command.id] ?? []), key]);
  if (steal) {
    for (const other of clashesWith(key, command, commands, keymap)) {
      bindings[other.id] = (keymap[other.id] ?? []).filter((existing) => existing !== key);
    }
  }
  return bindings;
}

export function removeShortcut(
  settings: ShortcutSettings,
  commands: Bindable[],
  command: Bindable,
  shortcut: string,
): ShortcutSettings["bindings"] {
  const keymap = buildKeymap(commands, settings);
  return {
    ...settings.bindings,
    [command.id]: (keymap[command.id] ?? []).filter((key) => key !== normalizeShortcut(shortcut)),
  };
}

/** forgets the user's change to one command, back to what the preset says */
export function resetShortcut(
  settings: ShortcutSettings,
  command: Bindable,
): ShortcutSettings["bindings"] {
  return Object.fromEntries(Object.entries(settings.bindings).filter(([id]) => id !== command.id));
}
