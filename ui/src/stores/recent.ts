import { create } from "zustand";

import type { RecentEntry } from "../bindings/RecentEntry";

interface RecentState {
  /** null until loaded, newest first */
  entries: RecentEntry[] | null;
  apply: (entries: RecentEntry[]) => void;
}

export const useRecent = create<RecentState>()((set) => ({
  entries: null,
  apply: (entries) => {
    set({ entries });
  },
}));
