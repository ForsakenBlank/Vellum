import { describe, expect, it } from "vitest";

import fixture from "../../../e2e/fixtures/backend.json";
import type { Keymap } from "../../bindings/Keymap";
import { buildItems, searchItems, type PaletteSources } from "./items";

const sources: PaletteSources = {
  commands: fixture.commands,
  keymap: fixture.keymaps.vellum as Keymap,
  recentCommands: ["view.theme.light"],
  files: [
    {
      path: "/m/holiday.mp4",
      name: "holiday.mp4",
      kind: "video",
      openedAtMs: 1,
      sizeBytes: 10,
      pinned: false,
    },
  ],
  savedLayouts: [{ name: "Colour grading", layout: fixture.settings.layout.current }],
};

const items = buildItems(sources);
const titles = (query: string) => searchItems(items, query).map((r) => r.item.title);

describe("palette items", () => {
  it("lists every registered command with its shortcut", () => {
    const commands = items.filter((i) => i.section === "Commands");
    expect(commands).toHaveLength(fixture.commands.length);
    expect(commands.find((i) => i.title === "Undo")?.shortcut).toBe("Ctrl+Z");
  });

  it("starts with recent commands and files when nothing is typed", () => {
    const first = searchItems(items, "")
      .slice(0, 2)
      .map((r) => r.item.id);
    expect(first).toEqual(["Recent:view.theme.light", "file:/m/holiday.mp4"]);
  });

  it("finds commands, files, settings and presets", () => {
    expect(titles("undo")[0]).toBe("Undo");
    expect(titles("holiday")[0]).toBe("holiday.mp4");
    expect(titles("compact")).toContain("Compact mode");
    expect(titles("photoshop")).toContain("Use Photoshop shortcuts");
    expect(titles("colour grading")[0]).toBe("Colour grading layout");
  });

  it("finds things by what they do, not only by name", () => {
    expect(titles("discord")).toContain("Size presets");
    expect(titles("shrink")[0]).toBe("Export to 10 MB");
    expect(titles("preferences")).toContain("Theme");
  });

  it("does not list recent commands twice in search results", () => {
    expect(searchItems(items, "light").filter((r) => r.item.section === "Recent")).toEqual([]);
  });
});
