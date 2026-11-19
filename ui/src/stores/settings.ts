import { create } from "zustand";
import type { Settings } from "../bindings/Settings";

interface SettingsStore {
  settings: Settings | null;
  set: (settings: Settings) => void;
}

export const useSettings = create<SettingsStore>()((set) => ({
  settings: null,
  set: (settings) => set({ settings }),
}));

// pushes appearance settings onto the document so css tokens pick them up
export function applyAppearance(settings: Settings, root: HTMLElement = document.documentElement) {
  const { appearance } = settings;
  const themes = { dark: "dark", light: "light", highContrast: "high-contrast" } as const;
  root.dataset.theme = themes[appearance.theme];
  root.dataset.compact = String(appearance.compact);
  root.dataset.reducedMotion = String(appearance.reducedMotion);
  root.style.setProperty("--accent", appearance.accent);
  root.style.fontSize = `${appearance.uiScale * 100}%`;
}
