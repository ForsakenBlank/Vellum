import { describe, expect, it } from "vitest";

import { formatShortcut, isBareKey, isTyping, normalizeShortcut, shortcutFromEvent } from "./keys";

function press(code: string, key: string, mods: Partial<Record<string, boolean>> = {}) {
  return {
    code,
    key,
    ctrlKey: mods.ctrl ?? false,
    metaKey: mods.meta ?? false,
    altKey: mods.alt ?? false,
    shiftKey: mods.shift ?? false,
  };
}

describe("keyboard shortcuts", () => {
  it("turns key presses into the same string on every platform", () => {
    expect(shortcutFromEvent(press("KeyK", "k", { ctrl: true }), false)).toBe("Mod+K");
    expect(shortcutFromEvent(press("KeyK", "k", { meta: true }), true)).toBe("Mod+K");
    // ctrl on a mac is its own modifier
    expect(shortcutFromEvent(press("KeyK", "k", { ctrl: true }), true)).toBe("Ctrl+K");
  });

  it("reads the physical key, so shift and layouts do not change it", () => {
    expect(shortcutFromEvent(press("Digit1", "!", { shift: true, alt: true }), false)).toBe(
      "Alt+Shift+1",
    );
    expect(shortcutFromEvent(press("Comma", ",", { ctrl: true }), false)).toBe("Mod+,");
    expect(shortcutFromEvent(press("Space", " "), false)).toBe("Space");
    expect(shortcutFromEvent(press("ArrowLeft", "ArrowLeft"), false)).toBe("ArrowLeft");
  });

  it("waits while only modifiers are held", () => {
    expect(shortcutFromEvent(press("ShiftLeft", "Shift", { shift: true }), false)).toBeNull();
    expect(shortcutFromEvent(press("ControlLeft", "Control", { ctrl: true }), false)).toBeNull();
  });

  it("puts modifiers in one order so equal shortcuts compare equal", () => {
    expect(normalizeShortcut("shift+mod+k")).toBe("Mod+Shift+K");
    expect(normalizeShortcut("Alt+Mod+=")).toBe("Mod+Alt+=");
  });

  it("writes shortcuts the way each platform does", () => {
    expect(formatShortcut("Mod+Shift+K", false)).toBe("Ctrl+Shift+K");
    expect(formatShortcut("Mod+Shift+K", true)).toBe("⇧⌘K");
    expect(formatShortcut("ArrowUp", false)).toBe("↑");
  });

  it("knows which shortcuts must not fire while typing", () => {
    expect(isBareKey("J")).toBe(true);
    expect(isBareKey("Shift+1")).toBe(true);
    expect(isBareKey("Mod+J")).toBe(false);

    const input = document.createElement("input");
    expect(isTyping(input)).toBe(true);
    input.type = "checkbox";
    expect(isTyping(input)).toBe(false);
    expect(isTyping(document.createElement("button"))).toBe(false);
  });
});
