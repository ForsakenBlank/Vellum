import { create } from "zustand";

import type { AutosaveStatus } from "../bindings/AutosaveStatus";
import type { RecoveryOffer } from "../bindings/RecoveryOffer";
import type { DropPlan } from "../lib/routing";

export type ToastLevel = "info" | "success" | "warning" | "error";

export interface Toast {
  id: number;
  level: ToastLevel;
  title: string;
  body?: string | undefined;
  action?: { label: string; run: () => void } | undefined;
}

export type SettingsSection =
  "appearance" | "shortcuts" | "autosave" | "jobs" | "history" | "media" | "export" | "about";

export interface ConfirmRequest {
  title: string;
  body: string;
  confirmLabel: string;
  /** a third button, eg "Save and close" */
  altLabel?: string | undefined;
  danger?: boolean | undefined;
  resolve: (answer: "confirm" | "alt" | "cancel") => void;
}

interface UiStore {
  paletteOpen: boolean;
  settingsSection: SettingsSection | null;
  /** search text to start the settings window with */
  settingsQuery: string;
  notificationsOpen: boolean;
  toasts: Toast[];
  confirm: ConfirmRequest | null;
  recovery: RecoveryOffer | null;
  autosave: AutosaveStatus;
  /** what the files being dragged over the window will do, null when nothing is dragged */
  drop: DropPlan | null;
  /** the quick tool card under the pointer during a drag */
  dropTool: string | null;

  setPalette: (open: boolean) => void;
  openSettings: (section?: SettingsSection, query?: string) => void;
  closeSettings: () => void;
  setNotificationsOpen: (open: boolean) => void;
  toast: (toast: Omit<Toast, "id">) => number;
  dismissToast: (id: number) => void;
  setRecovery: (offer: RecoveryOffer | null) => void;
  setAutosave: (status: AutosaveStatus) => void;
  setDrop: (plan: DropPlan | null, tool: string | null) => void;
}

let nextToast = 1;

export const useUi = create<UiStore>()((set) => ({
  paletteOpen: false,
  settingsSection: null,
  settingsQuery: "",
  notificationsOpen: false,
  toasts: [],
  confirm: null,
  recovery: null,
  autosave: { savedAtMs: null, error: null },
  drop: null,
  dropTool: null,

  setPalette: (open) => {
    set({ paletteOpen: open });
  },
  openSettings: (section = "appearance", query = "") => {
    set({ settingsSection: section, settingsQuery: query, paletteOpen: false });
  },
  closeSettings: () => {
    set({ settingsSection: null, settingsQuery: "" });
  },
  setNotificationsOpen: (open) => {
    set({ notificationsOpen: open });
  },
  toast: (toast) => {
    const id = nextToast++;
    // at most four on screen, the oldest goes first
    set((state) => ({ toasts: [...state.toasts.slice(-3), { ...toast, id }] }));
    return id;
  },
  dismissToast: (id) => {
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
  },
  setRecovery: (offer) => {
    set({ recovery: offer });
  },
  setAutosave: (status) => {
    set({ autosave: status });
  },
  setDrop: (plan, tool) => {
    set({ drop: plan, dropTool: tool });
  },
}));

/** asks a yes or no question in a dialog and waits for the answer */
export function askConfirm(request: Omit<ConfirmRequest, "resolve">) {
  return new Promise<"confirm" | "alt" | "cancel">((resolve) => {
    useUi.setState({
      confirm: {
        ...request,
        resolve: (answer) => {
          useUi.setState({ confirm: null });
          resolve(answer);
        },
      },
    });
  });
}
