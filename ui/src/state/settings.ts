import { create } from "zustand";

import type { AppSettings } from "../bindings/AppSettings";
import { ipc, type DeepPartial } from "../lib/ipc";

interface SettingsStore {
  /** null until loaded from rust */
  settings: AppSettings | null;
  error: string | null;
  replace: (settings: AppSettings) => void;
  update: (patch: DeepPartial<AppSettings>) => Promise<void>;
}

export const useSettings = create<SettingsStore>()((set) => ({
  settings: null,
  error: null,
  replace: (settings) => {
    set({ settings, error: null });
  },
  update: async (patch) => {
    try {
      // rust saves, sanitises and echoes the result back, so the store follows rust
      const settings = await ipc.updateSettings(patch);
      set({ settings, error: null });
    } catch (err) {
      set({ error: String(err) });
    }
  },
}));

/** pushes appearance settings onto the document so css can follow them */
export function applyAppearance(
  settings: AppSettings,
  root: HTMLElement = document.documentElement,
) {
  const look = settings.appearance;
  root.dataset.theme = look.theme;
  root.style.setProperty("--accent", look.accent);
  root.style.fontSize = `${16 * look.uiScale}px`;
  if (look.reducedMotion) {
    root.dataset.reducedMotion = "";
  } else {
    delete root.dataset.reducedMotion;
  }
}
