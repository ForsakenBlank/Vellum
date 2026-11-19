import type { SettingsSection } from "../../state/ui";

/** one searchable setting. the settings window and the palette both search this list. */
export interface SettingEntry {
  id: string;
  section: SettingsSection;
  label: string;
  description: string;
  keywords?: string;
}

export const sectionTitles: Record<SettingsSection, string> = {
  appearance: "Appearance",
  shortcuts: "Keyboard shortcuts",
  autosave: "Autosave and recovery",
  jobs: "Jobs",
  history: "Undo history",
  about: "About",
};

export const settingsCatalog: SettingEntry[] = [
  {
    id: "theme",
    section: "appearance",
    label: "Theme",
    description: "Dark, light or high contrast.",
    keywords: "dark light high contrast colour mode",
  },
  {
    id: "accent",
    section: "appearance",
    label: "Accent colour",
    description: "The colour of buttons, selections and highlights.",
    keywords: "color highlight tint",
  },
  {
    id: "uiScale",
    section: "appearance",
    label: "Interface size",
    description: "Make everything bigger or smaller.",
    keywords: "ui scale zoom text size dpi",
  },
  {
    id: "compact",
    section: "appearance",
    label: "Compact mode",
    description: "Tighter rows and padding for small screens.",
    keywords: "density dense small laptop",
  },
  {
    id: "reducedMotion",
    section: "appearance",
    label: "Reduce motion",
    description: "Turn off animations.",
    keywords: "animation accessibility",
  },
  {
    id: "preset",
    section: "shortcuts",
    label: "Shortcut preset",
    description: "Use shortcuts from Photoshop, Premiere, Resolve or Audacity.",
    keywords: "keys keyboard hotkeys photoshop premiere resolve audacity",
  },
  {
    id: "autosaveEnabled",
    section: "autosave",
    label: "Autosave",
    description: "Keep a copy of open documents so nothing is lost after a crash.",
    keywords: "crash recovery backup restore",
  },
  {
    id: "autosaveInterval",
    section: "autosave",
    label: "Autosave every",
    description: "How often open documents are saved in the background.",
    keywords: "interval seconds minutes",
  },
  {
    id: "maxCpuJobs",
    section: "jobs",
    label: "CPU jobs at once",
    description: "Conversions and encodes that can run side by side.",
    keywords: "parallel queue processor threads",
  },
  {
    id: "maxGpuJobs",
    section: "jobs",
    label: "GPU jobs at once",
    description: "AI tools and renders that can run side by side.",
    keywords: "parallel queue graphics vram",
  },
  {
    id: "maxIoJobs",
    section: "jobs",
    label: "Disk and network jobs at once",
    description: "Downloads and big copies that can run side by side.",
    keywords: "parallel queue io download",
  },
  {
    id: "maxSteps",
    section: "history",
    label: "Undo steps to keep",
    description: "How far back undo can go in each document.",
    keywords: "history levels",
  },
  {
    id: "maxMemory",
    section: "history",
    label: "Undo memory",
    description: "How much memory undo may use per document.",
    keywords: "history ram limit",
  },
  {
    id: "version",
    section: "about",
    label: "Version",
    description: "Which Vellum this is, and where its settings live.",
    keywords: "about build settings file path",
  },
];
