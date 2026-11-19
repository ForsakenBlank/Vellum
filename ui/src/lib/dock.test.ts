import { describe, expect, it } from "vitest";

import type { DockLayout } from "../bindings/DockLayout";
import defaults from "../../e2e/fixtures/backend.json";
import {
  anyDockOpen,
  closePanel,
  findPanel,
  isPanelVisible,
  layoutPresets,
  movePanel,
  normalizeLayout,
  resizeArea,
  resizeGroups,
  showPanel,
  toggleDocks,
  togglePanel,
} from "./dock";

const known = ["info", "history", "jobs"];

function base(): DockLayout {
  const preset = layoutPresets.find((p) => p.id === "default");
  if (!preset) throw new Error("no default preset");
  return structuredClone(preset.layout);
}

function panelsOf(layout: DockLayout) {
  return {
    left: layout.left.groups.map((g) => g.panels),
    right: layout.right.groups.map((g) => g.panels),
    bottom: layout.bottom.groups.map((g) => g.panels),
  };
}

describe("layout presets", () => {
  it("the default preset matches the default rust saves", () => {
    expect(base()).toEqual(defaults.settings.layout.current);
  });

  it("every preset only uses known panels", () => {
    for (const preset of layoutPresets) {
      expect(normalizeLayout(preset.layout, known)).toEqual(preset.layout);
    }
  });
});

describe("normalizeLayout", () => {
  it("drops unknown and repeated panels, empty groups and bad active tabs", () => {
    const layout = base();
    layout.right.groups[0] = {
      id: "right-top",
      panels: ["ghost", "info"],
      active: "ghost",
      weight: 1,
    };
    layout.bottom.groups.push({ id: "bottom", panels: ["info"], active: "info", weight: 1 });
    layout.bottom.groups.push({ id: "", panels: ["ghost"], active: "", weight: 1 });
    const clean = normalizeLayout(layout, known);
    expect(panelsOf(clean)).toEqual({ left: [], right: [["info"], ["jobs"]], bottom: [] });
    expect(clean.right.groups[0]?.active).toBe("info");
  });
});

describe("movePanel", () => {
  it("moves a panel into another group as a tab", () => {
    const next = movePanel(base(), "jobs", {
      type: "tab",
      area: "right",
      group: "right-top",
      index: 1,
    });
    expect(panelsOf(next).right).toEqual([["info", "jobs", "history"]]);
    expect(next.right.groups[0]?.active).toBe("jobs");
  });

  it("reorders tabs inside a group either way", () => {
    const forward = movePanel(base(), "info", {
      type: "tab",
      area: "right",
      group: "right-top",
      index: 2,
    });
    expect(panelsOf(forward).right[0]).toEqual(["history", "info"]);
    const back = movePanel(forward, "info", {
      type: "tab",
      area: "right",
      group: "right-top",
      index: 0,
    });
    expect(panelsOf(back).right[0]).toEqual(["info", "history"]);
  });

  it("makes a new group in another dock and opens it", () => {
    let layout = base();
    layout.bottom.collapsed = true;
    layout = movePanel(layout, "history", { type: "group", area: "bottom", index: 0 });
    expect(panelsOf(layout)).toEqual({
      left: [],
      right: [["info"], ["jobs"]],
      bottom: [["history"]],
    });
    expect(layout.bottom.collapsed).toBe(false);
    // the group it left shows its next panel
    expect(layout.right.groups[0]?.active).toBe("info");
  });

  it("moves a group of one to another place in the same dock", () => {
    const next = movePanel(base(), "jobs", { type: "group", area: "right", index: 0 });
    expect(panelsOf(next).right).toEqual([["jobs"], ["info", "history"]]);
  });

  it("ignores panels that are not placed", () => {
    const layout = base();
    expect(movePanel(layout, "nope", { type: "group", area: "left", index: 0 })).toBe(layout);
  });
});

describe("showing and hiding", () => {
  it("brings a panel to the front and opens its dock", () => {
    let layout = base();
    layout.right.collapsed = true;
    layout = showPanel(layout, "history", "right");
    expect(layout.right.collapsed).toBe(false);
    expect(isPanelVisible(layout, "history")).toBe(true);
  });

  it("puts a closed panel back in its home dock", () => {
    const closed = closePanel(base(), "jobs");
    expect(findPanel(closed, "jobs")).toBeNull();
    const shown = showPanel(closed, "jobs", "bottom");
    expect(panelsOf(shown).bottom).toEqual([["jobs"]]);
  });

  it("toggling a visible panel hides its dock", () => {
    const hidden = togglePanel(base(), "jobs", "right");
    expect(hidden.right.collapsed).toBe(true);
    expect(isPanelVisible(togglePanel(hidden, "jobs", "right"), "jobs")).toBe(true);
  });

  it("hides every dock and brings back the ones with panels", () => {
    const hidden = toggleDocks(base());
    expect(anyDockOpen(hidden)).toBe(false);
    const shown = toggleDocks(hidden);
    expect(shown.right.collapsed).toBe(false);
    expect(shown.left.collapsed).toBe(true);
  });
});

describe("resizing", () => {
  it("keeps docks and groups within limits", () => {
    expect(resizeArea(base(), "right", 9000).right.size).toBe(640);
    expect(resizeArea(base(), "bottom", 10).bottom.size).toBe(100);
    const groups = resizeGroups(base(), "right", 0, [0, 30]).right.groups;
    expect(groups.map((g) => g.weight)).toEqual([0.1, 10]);
  });
});
