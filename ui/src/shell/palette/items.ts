// everything the command palette can find, built from the stores. kept apart from the
// component so the list and its order can be tested without rendering.

import type { CommandInfo } from "../../bindings/CommandInfo";
import type { Keymap } from "../../bindings/Keymap";
import type { RecentFile } from "../../bindings/RecentFile";
import type { SavedLayout } from "../../bindings/SavedLayout";
import type { ShortcutPreset } from "../../bindings/ShortcutPreset";
import { rank, type Ranked } from "../../lib/fuzzy";
import { layoutPresets } from "../../lib/dock";
import { kindOf } from "../../lib/routing";
import { sectionTitles, settingsCatalog, type SettingEntry } from "../settings/catalog";

export type PaletteSection = "Recent" | "Commands" | "Files" | "Settings" | "Presets";

export type PaletteAction =
  | { type: "command"; kind: string }
  | { type: "file"; path: string }
  | { type: "setting"; entry: SettingEntry }
  | { type: "shortcutPreset"; preset: ShortcutPreset }
  | { type: "layout"; name: string };

export interface PaletteItem {
  id: string;
  section: PaletteSection;
  title: string;
  subtitle: string;
  keywords: string;
  shortcut?: string | undefined;
  action: PaletteAction;
}

export const shortcutPresetNames: Record<ShortcutPreset, string> = {
  vellum: "Vellum",
  photoshop: "Photoshop",
  premiere: "Premiere Pro",
  resolve: "DaVinci Resolve",
  audacity: "Audacity",
};

export interface PaletteSources {
  commands: CommandInfo[];
  keymap: Keymap | null;
  recentCommands: string[];
  files: RecentFile[];
  savedLayouts: SavedLayout[];
}

export function buildItems(sources: PaletteSources): PaletteItem[] {
  const commandItem = (info: CommandInfo, section: PaletteSection): PaletteItem => ({
    id: `${section}:${info.kind}`,
    section,
    title: info.title,
    subtitle: info.category,
    keywords: `${info.category} ${info.description} ${info.kind.replace(/[._]/g, " ")}`,
    shortcut: sources.keymap?.bindings[info.kind]?.[0],
    action: { type: "command", kind: info.kind },
  });

  const recent = sources.recentCommands
    .map((kind) => sources.commands.find((c) => c.kind === kind))
    .filter((c): c is CommandInfo => c !== undefined)
    .map((info) => commandItem(info, "Recent"));

  const commands = sources.commands.map((info) => commandItem(info, "Commands"));

  const files: PaletteItem[] = sources.files.map((file) => ({
    id: `file:${file.path}`,
    section: "Files",
    title: file.name,
    subtitle: file.path,
    keywords: `${kindOf(file.path)?.label ?? ""} ${file.pinned ? "pinned" : "recent"} open file`,
    action: { type: "file", path: file.path },
  }));

  const settings: PaletteItem[] = settingsCatalog.map((entry) => ({
    id: `setting:${entry.id}`,
    section: "Settings",
    title: entry.label,
    subtitle: sectionTitles[entry.section],
    keywords: `settings preferences ${entry.description} ${entry.keywords ?? ""}`,
    action: { type: "setting", entry },
  }));

  const presets: PaletteItem[] = [
    ...(Object.keys(shortcutPresetNames) as ShortcutPreset[]).map((preset): PaletteItem => ({
      id: `preset:${preset}`,
      section: "Presets",
      title: `Use ${shortcutPresetNames[preset]} shortcuts`,
      subtitle: "Shortcut preset",
      keywords: "keyboard keys hotkeys preset",
      action: { type: "shortcutPreset", preset },
    })),
    ...layoutPresets.map((preset): PaletteItem => ({
      id: `layout:${preset.id}`,
      section: "Presets",
      title: `${preset.name} layout`,
      subtitle: preset.description,
      keywords: "layout panels workspace arrangement",
      action: { type: "layout", name: preset.id },
    })),
    ...sources.savedLayouts.map((saved): PaletteItem => ({
      id: `layout:${saved.name}`,
      section: "Presets",
      title: `${saved.name} layout`,
      subtitle: "Saved layout",
      keywords: "layout panels workspace arrangement saved",
      action: { type: "layout", name: saved.name },
    })),
  ];

  return [...recent, ...commands, ...files, ...settings, ...presets];
}

const sectionOrder: PaletteSection[] = ["Recent", "Commands", "Files", "Settings", "Presets"];

/**
 * with no query: recent commands, then recent files, then everything else in order.
 * with a query: the best matches first, with recent duplicates dropped.
 */
export function searchItems(
  items: PaletteItem[],
  query: string,
  limit = 60,
): Ranked<PaletteItem>[] {
  if (query.trim() === "") {
    const shown = items.filter((i) => i.section !== "Settings" && i.section !== "Presets");
    const order = (i: PaletteItem) =>
      i.section === "Files" ? 1 : sectionOrder.indexOf(i.section) * 2;
    return [...shown]
      .sort((a, b) => order(a) - order(b))
      .slice(0, limit)
      .map((item) => ({ item, score: 0, positions: [] }));
  }
  const pool = items.filter((i) => i.section !== "Recent");
  return rank(query, pool, (i) => ({ title: i.title, keywords: i.keywords })).slice(0, limit);
}
