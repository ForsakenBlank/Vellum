import { describe, expect, it } from "vitest";

import type { ShortcutSettings } from "../bindings/ShortcutSettings";
import {
  assignShortcut,
  buildKeymap,
  clashesWith,
  findConflicts,
  removeShortcut,
  resetShortcut,
  type Bindable,
} from "./shortcuts";

const commands: Bindable[] = [
  { id: "app.palette", title: "Command palette", shortcut: "Mod+K", scope: "global" },
  { id: "app.settings", title: "Settings", shortcut: "Mod+,", scope: "global" },
  { id: "go.video", title: "Go to Video", shortcut: "Mod+3", scope: "global" },
  { id: "image.brush", title: "Brush", shortcut: "B", scope: "image" },
  { id: "video.blade", title: "Blade", shortcut: "B", scope: "video" },
];

const settings = (over: Partial<ShortcutSettings> = {}): ShortcutSettings => ({
  preset: "vellum",
  bindings: {},
  ...over,
});

const byId = (id: string) => {
  const command = commands.find((entry) => entry.id === id);
  if (!command) throw new Error(id);
  return command;
};

describe("shortcuts", () => {
  it("layers defaults, the preset and the user's own keys", () => {
    expect(buildKeymap(commands, settings())["app.settings"]).toEqual(["Mod+,"]);

    const photoshop = buildKeymap(commands, settings({ preset: "photoshop" }));
    expect(photoshop["app.settings"]).toEqual(["Mod+K"]);
    expect(photoshop["app.palette"]).toEqual(["Mod+Shift+P"]);

    const own = buildKeymap(
      commands,
      settings({ preset: "photoshop", bindings: { "app.settings": ["Mod+Shift+S"] } }),
    );
    expect(own["app.settings"]).toEqual(["Mod+Shift+S"]);
  });

  it("only counts a shared key as a conflict where both commands work", () => {
    const keymap = buildKeymap(commands, settings());
    // the brush and the blade share B but live in different workspaces
    expect(findConflicts(commands, keymap)).toEqual([]);

    const clashing = buildKeymap(commands, settings({ bindings: { "go.video": ["B"] } }));
    const conflicts = findConflicts(commands, clashing);
    expect(conflicts).toHaveLength(1);
    expect(conflicts[0]?.commands.map((c) => c.id).sort()).toEqual([
      "go.video",
      "image.brush",
      "video.blade",
    ]);
  });

  it("says who already uses a key before it is given to another command", () => {
    const keymap = buildKeymap(commands, settings());
    expect(clashesWith("mod+k", byId("app.settings"), commands, keymap).map((c) => c.id)).toEqual([
      "app.palette",
    ]);
    expect(clashesWith("B", byId("image.brush"), commands, keymap)).toEqual([]);
  });

  it("can take a key away from the command that had it", () => {
    const stolen = assignShortcut(settings(), commands, byId("app.settings"), "Mod+K", true);
    expect(stolen).toEqual({ "app.settings": ["Mod+,", "Mod+K"], "app.palette": [] });

    const shared = assignShortcut(settings(), commands, byId("app.settings"), "Mod+K", false);
    expect(shared).toEqual({ "app.settings": ["Mod+,", "Mod+K"] });
  });

  it("removes and resets single commands", () => {
    const removed = removeShortcut(settings(), commands, byId("app.palette"), "Mod+K");
    expect(removed).toEqual({ "app.palette": [] });
    expect(buildKeymap(commands, settings({ bindings: removed }))["app.palette"]).toEqual([]);

    expect(resetShortcut(settings({ bindings: removed }), byId("app.palette"))).toEqual({});
  });
});
