import type { SettingsPane } from "../../stores/ui";

/** every setting, so the settings search and the command palette can find them */
export interface SettingEntry {
  id: string;
  title: string;
  description: string;
  pane: SettingsPane;
  keywords: string[];
}

export const PANES: { id: SettingsPane; title: string }[] = [
  { id: "appearance", title: "Appearance" },
  { id: "general", title: "General" },
  { id: "performance", title: "Performance" },
  { id: "shortcuts", title: "Shortcuts" },
];

/** shown at the top of the shortcut editor as well as in search */
export const PRESET_SETTING: SettingEntry = {
  id: "shortcut-preset",
  title: "Shortcut preset",
  description: "Start from the keys of an app you already know.",
  pane: "shortcuts",
  keywords: ["photoshop", "premiere", "resolve", "audacity", "keymap", "keys"],
};

export const SETTINGS: SettingEntry[] = [
  {
    id: "theme",
    title: "Theme",
    description: "Dark, light, or high contrast for the clearest edges.",
    pane: "appearance",
    keywords: ["dark mode", "light mode", "colours", "colors"],
  },
  {
    id: "accent",
    title: "Accent colour",
    description: "The colour of selections, buttons and progress.",
    pane: "appearance",
    keywords: ["color", "highlight", "tint"],
  },
  {
    id: "ui-scale",
    title: "Interface size",
    description: "Makes all text and controls bigger or smaller.",
    pane: "appearance",
    keywords: ["ui scale", "zoom", "font size", "text size"],
  },
  {
    id: "compact",
    title: "Compact mode",
    description: "Tighter spacing, so more fits on a small screen.",
    pane: "appearance",
    keywords: ["density", "dense", "spacing"],
  },
  {
    id: "reduced-motion",
    title: "Reduce motion",
    description: "Turns off animations and transitions.",
    pane: "appearance",
    keywords: ["animation", "accessibility", "a11y"],
  },
  {
    id: "autosave",
    title: "Autosave",
    description:
      "How often open files are saved in the background, so nothing is lost if Vellum closes unexpectedly.",
    pane: "general",
    keywords: ["crash recovery", "backup", "save"],
  },
  {
    id: "recent-limit",
    title: "Recent files to remember",
    description: "Pinned files are always kept and do not count.",
    pane: "general",
    keywords: ["history", "home screen"],
  },
  {
    id: "notify-jobs",
    title: "Notify when jobs finish",
    description: "Shows a notification when an export or other job is done or fails.",
    pane: "general",
    keywords: ["notifications", "alerts", "toast"],
  },
  {
    id: "gpu",
    title: "GPU acceleration",
    description:
      "Uses the graphics card for previews and effects. Turn it off if things look wrong.",
    pane: "performance",
    keywords: ["graphics", "hardware", "acceleration"],
  },
  {
    id: "cpu-jobs",
    title: "Jobs at the same time",
    description:
      "How many exports and conversions run at once. Automatic picks from your processor.",
    pane: "performance",
    keywords: ["parallel", "cpu", "queue", "threads"],
  },
  {
    id: "gpu-jobs",
    title: "GPU jobs at the same time",
    description: "AI tools and GPU effects that can run at once. More needs more video memory.",
    pane: "performance",
    keywords: ["parallel", "vram", "ai"],
  },
  {
    id: "low-memory",
    title: "Low memory mode",
    description: "Uses less memory for big files, at some cost to speed.",
    pane: "performance",
    keywords: ["ram", "old computer", "slow"],
  },
  PRESET_SETTING,
];
