import { describe, expect, it } from "vitest";

import {
  chordFromEvent,
  commandsForChord,
  firesWhileTyping,
  formatChord,
  isEditable,
  keyFromCode,
  type KeyLike,
} from "./keys";

function press(code: string, mods: Partial<KeyLike> = {}): KeyLike {
  return {
    code,
    key: "",
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...mods,
  };
}

describe("chordFromEvent", () => {
  it("builds canonical chords from physical keys", () => {
    expect(chordFromEvent(press("KeyK", { ctrlKey: true }), false)).toBe("Ctrl+K");
    expect(chordFromEvent(press("KeyZ", { ctrlKey: true, shiftKey: true }), false)).toBe(
      "Ctrl+Shift+Z",
    );
    expect(chordFromEvent(press("Comma", { ctrlKey: true }), false)).toBe("Ctrl+,");
    expect(chordFromEvent(press("Digit4", { shiftKey: true }), false)).toBe("Shift+4");
    expect(chordFromEvent(press("F12"), false)).toBe("F12");
    expect(chordFromEvent(press("ArrowUp", { altKey: true }), false)).toBe("Alt+Up");
  });

  it("treats command as ctrl on macos and keeps the windows key apart elsewhere", () => {
    expect(chordFromEvent(press("KeyK", { metaKey: true }), true)).toBe("Ctrl+K");
    expect(chordFromEvent(press("KeyK", { metaKey: true }), false)).toBe("Meta+K");
  });

  it("ignores lone modifiers", () => {
    expect(chordFromEvent(press("ShiftLeft", { shiftKey: true }), false)).toBeNull();
    expect(chordFromEvent(press("ControlRight", { ctrlKey: true }), false)).toBeNull();
  });

  it("knows the keys rust knows", () => {
    expect(keyFromCode("Backslash")).toBe("\\");
    expect(keyFromCode("Numpad7")).toBe("7");
    expect(keyFromCode("F25")).toBeNull();
    expect(keyFromCode("IntlRo")).toBeNull();
  });
});

describe("formatChord", () => {
  it("uses symbols on macos and words elsewhere", () => {
    expect(formatChord("Ctrl+Shift+Z", true)).toBe("⌘⇧Z");
    expect(formatChord("Ctrl+Shift+Z", false)).toBe("Ctrl+Shift+Z");
    expect(formatChord("Meta+Up", false)).toBe("Win+↑");
  });
});

describe("firesWhileTyping", () => {
  it("leaves text editing to the field", () => {
    expect(firesWhileTyping("Ctrl+Z")).toBe(false);
    expect(firesWhileTyping("Ctrl+A")).toBe(false);
    expect(firesWhileTyping("K")).toBe(false);
    expect(firesWhileTyping("Shift+4")).toBe(false);
  });

  it("lets app shortcuts through", () => {
    expect(firesWhileTyping("Ctrl+K")).toBe(true);
    expect(firesWhileTyping("Ctrl+S")).toBe(true);
    expect(firesWhileTyping("F1")).toBe(true);
    expect(firesWhileTyping("Alt+Shift+1")).toBe(true);
  });
});

describe("isEditable", () => {
  it("spots text fields but not buttons or checkboxes", () => {
    const input = document.createElement("input");
    const box = document.createElement("input");
    box.type = "checkbox";
    expect(isEditable(input)).toBe(true);
    expect(isEditable(document.createElement("textarea"))).toBe(true);
    expect(isEditable(box)).toBe(false);
    expect(isEditable(document.createElement("button"))).toBe(false);
    expect(isEditable(null)).toBe(false);
  });
});

describe("commandsForChord", () => {
  it("finds every command bound to a chord", () => {
    const bindings = { "edit.redo": ["Ctrl+Shift+Z", "Ctrl+Y"], "file.save": ["Ctrl+S"] };
    expect(commandsForChord(bindings, "Ctrl+Y")).toEqual(["edit.redo"]);
    expect(commandsForChord(bindings, "Ctrl+Q")).toEqual([]);
  });
});
