import { create } from "zustand";

export type SettingsPane = "appearance" | "general" | "performance" | "shortcuts";

/** which overlays are open. these are per window and never saved */
interface UiState {
  paletteOpen: boolean;
  /** the settings pane on show, or null when settings are closed */
  settingsPane: SettingsPane | null;
  /** a setting to scroll to and flash when settings open, from the palette */
  settingsFocus: string | null;
  notificationsOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  openSettings: (pane?: SettingsPane, focus?: string | null) => void;
  closeSettings: () => void;
  setNotificationsOpen: (open: boolean) => void;
}

export const useUi = create<UiState>()((set) => ({
  paletteOpen: false,
  settingsPane: null,
  settingsFocus: null,
  notificationsOpen: false,
  setPaletteOpen: (paletteOpen) => {
    set({ paletteOpen });
  },
  openSettings: (pane = "appearance", focus = null) => {
    set({ settingsPane: pane, settingsFocus: focus, paletteOpen: false });
  },
  closeSettings: () => {
    set({ settingsPane: null, settingsFocus: null });
  },
  setNotificationsOpen: (notificationsOpen) => {
    set({ notificationsOpen });
  },
}));
