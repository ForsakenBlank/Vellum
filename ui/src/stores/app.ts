import { create } from "zustand";

import type { AppInfo } from "../bindings/AppInfo";
import type { Api } from "../lib/api";
import { errorMessage } from "../lib/api";

/** `browser` means the ui was opened without the desktop app around it */
export type ConnectionStatus = "connecting" | "ready" | "browser" | "failed";

interface AppState {
  status: ConnectionStatus;
  info: AppInfo | null;
  api: Api | null;
  /** label of the window this ui runs in */
  windowLabel: string;
  /** turns a local path into an image url, null without the desktop app */
  previewUrl: ((path: string) => string) | null;
  /** the last thing that went wrong, shown until dismissed */
  notice: string | null;
  setNotice: (notice: string | null) => void;
}

export const useApp = create<AppState>()((set) => ({
  status: "connecting",
  info: null,
  api: null,
  windowLabel: "main",
  previewUrl: null,
  notice: null,
  setNotice: (notice) => {
    set({ notice });
  },
}));

/**
 * runs a backend call started by the user. failures end up in the notice bar
 * instead of being lost in the console
 */
export async function perform<T>(action: (api: Api) => Promise<T>): Promise<T | undefined> {
  const { api, setNotice } = useApp.getState();
  if (!api) {
    setNotice("Vellum is not connected to its backend yet. Wait a moment and try again.");
    return undefined;
  }
  try {
    return await action(api);
  } catch (error) {
    setNotice(errorMessage(error));
    return undefined;
  }
}
