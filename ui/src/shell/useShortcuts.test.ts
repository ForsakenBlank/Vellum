import { describe, expect, it } from "vitest";

import { commandForKey } from "./useShortcuts";

function key(code: string, mods: Partial<KeyboardEventInit> = {}, target?: HTMLElement) {
  const event = new KeyboardEvent("keydown", { code, ...mods });
  if (target) Object.defineProperty(event, "target", { value: target });
  return event;
}

const bindings = {
  "app.command_palette": ["Ctrl+K"],
  "edit.undo": ["Ctrl+Z"],
  "image.brush": ["B"],
  "video.blade": ["B"],
};

describe("commandForKey", () => {
  it("finds the bound command", () => {
    expect(commandForKey(key("KeyK", { ctrlKey: true }), bindings, null)).toBe(
      "app.command_palette",
    );
    expect(commandForKey(key("KeyQ", { ctrlKey: true }), bindings, null)).toBeNull();
  });

  it("only runs workspace commands in their workspace", () => {
    expect(commandForKey(key("KeyB"), bindings, "image")).toBe("image.brush");
    expect(commandForKey(key("KeyB"), bindings, "video")).toBe("video.blade");
    expect(commandForKey(key("KeyB"), bindings, null)).toBeNull();
  });

  it("leaves typing alone but lets app shortcuts through", () => {
    const input = document.createElement("textarea");
    expect(commandForKey(key("KeyZ", { ctrlKey: true }, input), bindings, null)).toBeNull();
    expect(commandForKey(key("KeyB", {}, input), bindings, "image")).toBeNull();
    expect(commandForKey(key("KeyK", { ctrlKey: true }, input), bindings, null)).toBe(
      "app.command_palette",
    );
  });
});
