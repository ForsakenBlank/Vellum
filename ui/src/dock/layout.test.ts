import { describe, expect, it } from "vitest";

import type { DockLayout } from "../bindings/DockLayout";
import { dockLayout } from "./panels";
import {
  anyZoneOpen,
  findPanel,
  keepKnown,
  movePanel,
  openPanel,
  removePanel,
  resizeZone,
  setAllZones,
  shareStacks,
  togglePanel,
} from "./layout";

const stacks = (layout: DockLayout, zone: "left" | "right" | "bottom") =>
  layout[zone].stacks.map((stack) => stack.panels);

const start = () =>
  dockLayout({
    left: [["media", "effects"]],
    right: [["inspector"], ["history"]],
    bottom: [["timeline"]],
  });

describe("dock layouts", () => {
  it("adds a panel to a stack as a tab and brings it to the front", () => {
    const next = movePanel(start(), "history", { zone: "left", stack: 0, place: "into" });
    expect(stacks(next, "left")).toEqual([["media", "effects", "history"]]);
    expect(next.left.stacks[0]?.active).toBe("history");
    // its old stack was left empty, so it is gone
    expect(stacks(next, "right")).toEqual([["inspector"]]);
  });

  it("splits a panel into its own stack before or after another", () => {
    const before = movePanel(start(), "effects", { zone: "right", stack: 1, place: "before" });
    expect(stacks(before, "right")).toEqual([["inspector"], ["effects"], ["history"]]);
    const after = movePanel(start(), "media", { zone: "bottom", stack: 0, place: "after" });
    expect(stacks(after, "bottom")).toEqual([["timeline"], ["media"]]);
  });

  it("counts the stack a panel leaves when moving it further down the same zone", () => {
    const next = movePanel(start(), "inspector", { zone: "right", stack: 1, place: "after" });
    expect(stacks(next, "right")).toEqual([["history"], ["inspector"]]);
  });

  it("does nothing when a lone panel is dropped on its own stack", () => {
    const layout = start();
    expect(movePanel(layout, "history", { zone: "right", stack: 1, place: "before" })).toEqual(
      layout,
    );
  });

  it("closes a panel and opens it again where it belongs", () => {
    const defaults = start();
    const closed = removePanel(defaults, "effects");
    expect(findPanel(closed, "effects")).toBeNull();
    expect(closed.left.stacks[0]?.active).toBe("media");

    const reopened = openPanel(closed, "effects", defaults);
    expect(stacks(reopened, "left")).toEqual([["media", "effects"]]);

    // a panel with no home in the defaults goes to the right
    const stranger = openPanel(defaults, "jobs", defaults);
    expect(findPanel(stranger, "jobs")?.zone).toBe("right");
  });

  it("toggles a panel: hidden behind a tab comes forward, in front goes away", () => {
    const defaults = start();
    const forward = togglePanel(defaults, "effects", defaults);
    expect(forward.left.stacks[0]?.active).toBe("effects");
    const gone = togglePanel(forward, "effects", defaults);
    expect(findPanel(gone, "effects")).toBeNull();
  });

  it("keeps sizes and shares within limits", () => {
    expect(resizeZone(start(), "left", 999).left.size).toBe(40);
    expect(resizeZone(start(), "bottom", 1).bottom.size).toBe(6);
    const shared = shareStacks(start(), "right", 0, 0.99);
    expect(shared.right.stacks[0]?.weight).toBeCloseTo(1.8);
    expect(shared.right.stacks[1]?.weight).toBeCloseTo(0.2);
  });

  it("hides every zone and brings them back", () => {
    const hidden = setAllZones(start(), false);
    expect(anyZoneOpen(hidden)).toBe(false);
    expect(anyZoneOpen(setAllZones(hidden, true))).toBe(true);
  });

  it("drops panels this build does not know", () => {
    const known = new Set(["media", "inspector", "history", "timeline"]);
    const kept = keepKnown(start(), known);
    expect(stacks(kept, "left")).toEqual([["media"]]);
  });

  it("never changes the layout it was given", () => {
    const layout = start();
    const copy = structuredClone(layout);
    movePanel(layout, "media", { zone: "right", stack: 0, place: "into" });
    removePanel(layout, "inspector");
    resizeZone(layout, "left", 30);
    expect(layout).toEqual(copy);
  });
});
