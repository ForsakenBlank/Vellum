import { create } from "zustand";

import type { SettingsChanged } from "../bindings/SettingsChanged";
import type { SettingsMap } from "../bindings/SettingsMap";
import { perform } from "./app";

interface SettingsState {
  /** null until the backend has sent the settings */
  sections: SettingsMap | null;
  applyAll: (sections: SettingsMap) => void;
  applyChange: (change: SettingsChanged) => void;
}

export const useSettings = create<SettingsState>()((set) => ({
  sections: null,
  applyAll: (sections) => {
    set({ sections });
  },
  applyChange: (change) => {
    set((state) =>
      state.sections ? { sections: { ...state.sections, [change.section]: change.value } } : state,
    );
  },
}));

/**
 * changes some fields of one settings section. the backend checks and saves
 * the value, and what it sends back is what the store ends up with
 */
export async function updateSettings<K extends keyof SettingsMap>(
  section: K,
  patch: Partial<SettingsMap[K]>,
): Promise<void> {
  const current = useSettings.getState().sections?.[section];
  if (!current) return;
  const next = { ...current, ...patch };
  const saved = await perform((api) => api.settings.set(section, next));
  if (saved !== undefined) {
    useSettings.getState().applyChange({ section, value: saved });
  }
}
