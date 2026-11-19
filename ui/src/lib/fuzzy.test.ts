import { describe, expect, it } from "vitest";

import { fuzzyMatch, rank } from "./fuzzy";

describe("fuzzyMatch", () => {
  it("needs every character in order", () => {
    expect(fuzzyMatch("exp", "Export to 10 MB")).not.toBeNull();
    expect(fuzzyMatch("pxe", "Export to 10 MB")).toBeNull();
    expect(fuzzyMatch("toolong", "tool")).toBeNull();
  });

  it("returns the positions to highlight", () => {
    expect(fuzzyMatch("ss", "Save settings")?.positions).toEqual([0, 5]);
    expect(fuzzyMatch("gif", "Video to GIF")?.positions).toEqual([9, 10, 11]);
  });

  it("prefers word starts and runs of characters", () => {
    const starts = fuzzyMatch("dt", "Dark theme");
    const middle = fuzzyMatch("dt", "Undated");
    expect(starts && middle && starts.score > middle.score).toBe(true);

    const run = fuzzyMatch("zoo", "Zoom in");
    const scattered = fuzzyMatch("zoo", "Zip of old");
    expect(run && scattered && run.score > scattered.score).toBe(true);
  });

  it("treats an empty query as a match", () => {
    expect(fuzzyMatch("", "anything")).toEqual({ score: 0, positions: [] });
  });
});

describe("rank", () => {
  const items = [
    { title: "Undo", keywords: "edit step back" },
    { title: "Dark theme", keywords: "view appearance colour" },
    { title: "Light theme", keywords: "view appearance colour" },
    { title: "Export to 10 MB", keywords: "quick tools discord shrink compress" },
  ];

  it("ranks the best title match first", () => {
    const result = rank("theme", items, (i) => i);
    expect(result.map((r) => r.item.title)).toEqual(["Dark theme", "Light theme"]);
  });

  it("finds items through their keywords", () => {
    const result = rank("discord", items, (i) => i);
    expect(result.map((r) => r.item.title)).toEqual(["Export to 10 MB"]);
    expect(result[0]?.positions).toEqual([]);
  });

  it("needs every word to match somewhere", () => {
    expect(rank("dark colour", items, (i) => i).map((r) => r.item.title)).toEqual(["Dark theme"]);
    expect(rank("dark discord", items, (i) => i)).toEqual([]);
  });

  it("keeps the original order for an empty query", () => {
    expect(rank("  ", items, (i) => i).map((r) => r.item.title)).toEqual(items.map((i) => i.title));
  });
});
