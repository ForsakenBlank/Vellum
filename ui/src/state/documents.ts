import { create } from "zustand";

import type { DocumentInfo } from "../bindings/DocumentInfo";
import type { HistorySummary } from "../bindings/HistorySummary";
import type { Workspace } from "../bindings/Workspace";
import { windowLabel } from "../lib/window";

interface DocumentsData {
  /** every open document in every window, in tab order */
  documents: DocumentInfo[];
  summaries: Partial<Record<string, HistorySummary>>;
  /** the tab shown in this window, null for home or an empty workspace */
  activeId: string | null;
  /** a workspace shown without a document, after picking it in the rail */
  emptyWorkspace: Workspace | null;
  loaded: boolean;
}

interface DocumentsStore extends DocumentsData {
  setDocuments: (documents: DocumentInfo[]) => void;
  setSummary: (id: string, summary: HistorySummary) => void;
  activate: (id: string) => void;
  showHome: () => void;
  showWorkspace: (workspace: Workspace) => void;
}

/** the documents shown as tabs in a window */
export function tabsIn(documents: DocumentInfo[], window = windowLabel): DocumentInfo[] {
  return documents.filter((d) => d.window === window);
}

/**
 * picks the tab to show after the document list changed. the active tab stays if it is
 * still here; if it closed or moved away, its neighbour takes over, like closing a browser
 * tab. returns null for home.
 */
export function nextActive(
  before: DocumentInfo[],
  after: DocumentInfo[],
  activeId: string | null,
  window = windowLabel,
): string | null {
  const now = tabsIn(after, window);
  if (activeId === null) return null;
  if (now.some((d) => d.id === activeId)) return activeId;
  const was = tabsIn(before, window).findIndex((d) => d.id === activeId);
  if (was === -1 || now.length === 0) return null;
  return now[Math.min(was, now.length - 1)]?.id ?? null;
}

export const useDocuments = create<DocumentsStore>()((set) => ({
  documents: [],
  summaries: {},
  activeId: null,
  emptyWorkspace: null,
  loaded: false,
  setDocuments: (documents) => {
    set((state) => {
      const ids = new Set(documents.map((d) => d.id));
      const summaries = Object.fromEntries(
        Object.entries(state.summaries).filter(([id]) => ids.has(id)),
      );
      return {
        documents,
        summaries,
        loaded: true,
        activeId: nextActive(state.documents, documents, state.activeId),
      };
    });
  },
  setSummary: (id, summary) => {
    set((state) => ({ summaries: { ...state.summaries, [id]: summary } }));
  },
  activate: (id) => {
    set({ activeId: id, emptyWorkspace: null });
  },
  showHome: () => {
    set({ activeId: null, emptyWorkspace: null });
  },
  showWorkspace: (workspace) => {
    set((state) => {
      // the most recent tab of that workspace, or the workspace on its own
      const tabs = tabsIn(state.documents).filter((d) => d.workspace === workspace);
      const current = tabs.find((d) => d.id === state.activeId);
      const pick = current ?? tabs[tabs.length - 1];
      return pick
        ? { activeId: pick.id, emptyWorkspace: null }
        : { activeId: null, emptyWorkspace: workspace };
    });
  },
}));

export function activeDocument(state: DocumentsData): DocumentInfo | undefined {
  return state.documents.find((d) => d.id === state.activeId);
}
