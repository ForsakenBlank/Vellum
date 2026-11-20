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
  media: "Media and cache",
  export: "Export and quick tools",
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
    id: "hardwareAcceleration",
    section: "media",
    label: "Hardware encoding",
    description: "Use the graphics card to encode video when it can. Much faster, a little larger.",
    keywords: "gpu nvenc quick sync amf videotoolbox vaapi acceleration encoder",
  },
  {
    id: "hardwareEncoders",
    section: "media",
    label: "Encoders found",
    description: "Hardware encoders that passed a test encode on this computer.",
    keywords: "gpu nvenc quick sync amf videotoolbox vaapi",
  },
  {
    id: "ffmpeg",
    section: "media",
    label: "FFmpeg",
    description: "Reads and writes video and audio. Without it only images open.",
    keywords: "codec video audio decoder",
  },
  {
    id: "cacheLimit",
    section: "media",
    label: "Preview cache size",
    description: "Thumbnails, filmstrips and waveforms are kept up to this size, oldest first out.",
    keywords: "disk space thumbnails storage limit",
  },
  {
    id: "cacheUsage",
    section: "media",
    label: "Preview cache",
    description: "Cleaning it is safe, previews are made again when needed.",
    keywords: "clear clean delete disk space thumbnails storage",
  },
  {
    id: "exportTarget",
    section: "export",
    label: "Export to size starts at",
    description: "The size Export to size aims for until you pick another.",
    keywords: "target megabytes mb mib discord limit default",
  },
  {
    id: "exportMargin",
    section: "export",
    label: "Safety margin",
    description: "How far under the limit to aim, so files never land just over it.",
    keywords: "percent headroom under limit",
  },
  {
    id: "exportFolder",
    section: "export",
    label: "Save results in",
    description: "Where quick tools put what they make. Nothing is ever overwritten.",
    keywords: "output folder directory location save",
  },
  {
    id: "exportPresets",
    section: "export",
    label: "Size presets",
    description:
      "The chips in Export to size. Platforms change their limits, so change these when they do.",
    keywords: "discord nitro whatsapp telegram email twitter x upload limit platforms",
  },
  {
    id: "version",
    section: "about",
    label: "Version",
    description: "Which Vellum this is, and where its settings live.",
    keywords: "about build settings file path",
  },
];
