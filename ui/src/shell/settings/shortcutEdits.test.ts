import { describe, expect, it } from "vitest";

import { addChord, clashesFor, contextOf, removeChord, resetCommand } from "./shortcutEdits";

const bindings = {
  "app.command_palette": ["Ctrl+K"],
  "file.save": ["Ctrl+S"],
  "image.brush": ["B"],
  "video.blade": ["B"],
};

describe("shortcut edits", () => {
  it("knows the context of a command like rust does", () => {
    expect(contextOf("image.layer.new")).toBe("image");
    expect(contextOf("edit.undo")).toBe("global");
  });

  it("finds clashes, but not between different workspaces", () => {
    expect(clashesFor(bindings, "file.save", "Ctrl+K")).toEqual(["app.command_palette"]);
    expect(clashesFor(bindings, "image.blur", "B")).toEqual(["image.brush"]);
    expect(clashesFor(bindings, "app.home", "B")).toEqual(["image.brush", "video.blade"]);
    expect(clashesFor(bindings, "file.save", "Ctrl+S")).toEqual([]);
  });

  it("adds a chord and can take it from the command that had it", () => {
    expect(addChord(bindings, {}, "file.save", "Ctrl+K", false)).toEqual({
      "file.save": ["Ctrl+S", "Ctrl+K"],
    });
    expect(addChord(bindings, {}, "file.save", "Ctrl+K", true)).toEqual({
      "file.save": ["Ctrl+S", "Ctrl+K"],
      "app.command_palette": [],
    });
  });

  it("removes a chord and resets a command", () => {
    const removed = removeChord(bindings, {}, "file.save", "Ctrl+S");
    expect(removed).toEqual({ "file.save": [] });
    expect(resetCommand(removed, "file.save")).toEqual({});
  });
});
