import { describe, expect, it } from "vitest";

import type { HistoryView } from "../../bindings/HistoryView";
import { historyRows } from "./historyRows";

function entry(id: number, parent: number | null, onCurrentPath: boolean) {
  return { id, parent, name: id === 0 ? "Start" : `Step ${id}`, kind: "x", onCurrentPath };
}

describe("historyRows", () => {
  it("reads straight down a history with no branches", () => {
    const view: HistoryView = {
      root: 0,
      current: 2,
      entries: [entry(0, null, true), entry(1, 0, true), entry(2, 1, true)],
      snapshots: [],
    };
    expect(historyRows(view).map((r) => [r.entry.id, r.depth, r.current])).toEqual([
      [0, 0, false],
      [1, 0, false],
      [2, 0, true],
    ]);
  });

  it("indents side branches under the step they split from", () => {
    // 0 -> 1 -> 2 (abandoned), and 0 -> 1 -> 3 -> 4 (current)
    const view: HistoryView = {
      root: 0,
      current: 4,
      entries: [
        entry(0, null, true),
        entry(1, 0, true),
        entry(2, 1, false),
        entry(3, 1, true),
        entry(4, 3, true),
      ],
      snapshots: [],
    };
    expect(historyRows(view).map((r) => [r.entry.id, r.depth])).toEqual([
      [0, 0],
      [1, 0],
      [2, 1],
      [3, 0],
      [4, 0],
    ]);
  });

  it("copes with a missing root", () => {
    expect(historyRows({ root: 9, current: 9, entries: [], snapshots: [] })).toEqual([]);
  });
});
