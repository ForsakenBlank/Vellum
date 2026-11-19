import { create } from "zustand";

import type { RecentFile } from "../bindings/RecentFile";

interface RecentStore {
  /** null while loading */
  files: RecentFile[] | null;
  replace: (files: RecentFile[]) => void;
}

export const useRecent = create<RecentStore>()((set) => ({
  files: null,
  replace: (files) => {
    set({ files });
  },
}));
