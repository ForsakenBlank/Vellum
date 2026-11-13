import { describe, expect, it } from "vitest";

import { fuzzyMatch, fuzzyRank } from "./fuzzy";

const labels = (query: string, items: string[]) =>
  fuzzyRank(
    query,
    items.map((label) => ({ label })),
  ).map((result) => result.item.label);

describe("fuzzy search", () => {
  it("matches letters in order, not necessarily together", () => {
    expect(fuzzyMatch("exgf", "Export to GIF")).not.toBeNull();
    expect(fuzzyMatch("gfex", "Export to GIF")).toBeNull();
    expect(fuzzyMatch("", "anything")?.score).toBe(0);
  });

  it("reports where it matched, for highlighting", () => {
    expect(fuzzyMatch("set", "Settings")?.positions).toEqual([0, 1, 2]);
    expect(fuzzyMatch("ot", "Open theme")?.positions).toEqual([0, 5]);
  });

  it("puts word starts and runs ahead of scattered letters", () => {
    expect(labels("theme", ["Go to home", "Use light theme", "Theme"])).toEqual([
      "Theme",
      "Use light theme",
    ]);
    expect(labels("gh", ["Show the graph", "Go to home"])[0]).toBe("Go to home");
    expect(labels("cp", ["Compact mode", "Command palette"])[0]).toBe("Command palette");
  });

  it("finds items by their keywords too, a little behind the label", () => {
    const ranked = fuzzyRank("dark mode", [
      { label: "Theme", keywords: ["dark mode", "light mode"] },
      { label: "Close tab" },
    ]);
    expect(ranked.map((result) => result.item.label)).toEqual(["Theme"]);
    expect(ranked[0]?.positions).toEqual([]);
  });
});
