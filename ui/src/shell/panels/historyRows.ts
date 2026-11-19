import type { HistoryEntryView } from "../../bindings/HistoryEntryView";
import type { HistoryView } from "../../bindings/HistoryView";

export interface HistoryRow {
  entry: HistoryEntryView;
  /** 0 for the line leading to the current step, more for side branches */
  depth: number;
  current: boolean;
}

/**
 * flattens the history tree into rows, oldest first. the branch that leads to the current
 * step reads straight down; other branches are indented under the step they split from.
 */
export function historyRows(view: HistoryView): HistoryRow[] {
  const children = new Map<number, HistoryEntryView[]>();
  for (const entry of view.entries) {
    if (entry.parent === null) continue;
    const list = children.get(entry.parent) ?? [];
    list.push(entry);
    children.set(entry.parent, list);
  }
  const root = view.entries.find((e) => e.id === view.root);
  if (!root) return [];

  const rows: HistoryRow[] = [];
  const walk = (entry: HistoryEntryView, depth: number) => {
    rows.push({ entry, depth, current: entry.id === view.current });
    const kids = children.get(entry.id) ?? [];
    // side branches first, indented, then the main line continues at the same depth
    const main = kids.find((k) => k.onCurrentPath) ?? kids[kids.length - 1];
    for (const kid of kids) if (kid !== main) walk(kid, depth + 1);
    if (main) walk(main, depth);
  };
  walk(root, 0);
  return rows;
}
