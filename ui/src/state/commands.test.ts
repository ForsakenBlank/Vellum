import { beforeEach, describe, expect, it, vi } from "vitest";

import { ipc } from "../lib/ipc";
import { registerUiHandlers, runCommand, shortcutFor, useCommands } from "./commands";
import { useDocuments } from "./documents";
import { useUi } from "./ui";

vi.mock("../lib/ipc", () => ({
  inDesktopApp: true,
  ipc: { runCommand: vi.fn() },
}));

describe("runCommand", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUi.setState({ toasts: [] });
    useCommands.setState({ recent: [] });
    useDocuments.setState({ activeId: "doc-3" });
  });

  it("runs through rust with the active document and performs the ui actions it returns", async () => {
    vi.mocked(ipc.runCommand).mockResolvedValueOnce({
      uiActions: [{ kind: "app.command_palette", params: {} }],
      message: null,
    });
    const open = vi.fn();
    const cleanup = registerUiHandlers({ "app.command_palette": open });
    await runCommand("app.command_palette");
    cleanup();
    expect(ipc.runCommand).toHaveBeenCalledWith("app.command_palette", "doc-3");
    expect(open).toHaveBeenCalledOnce();
    expect(useCommands.getState().recent).toEqual(["app.command_palette"]);
  });

  it("shows rust's message as a toast", async () => {
    vi.mocked(ipc.runCommand).mockResolvedValueOnce({ uiActions: [], message: "Nothing to undo" });
    await runCommand("edit.undo");
    expect(useUi.getState().toasts[0]).toMatchObject({ level: "info", title: "Nothing to undo" });
  });

  it("explains failures", async () => {
    useCommands.setState({
      infos: [{ kind: "file.save", title: "Save", category: "File", description: "" }],
    });
    vi.mocked(ipc.runCommand).mockRejectedValueOnce("disk is full");
    await runCommand("file.save");
    expect(useUi.getState().toasts[0]).toMatchObject({
      level: "error",
      title: "Save did not work",
      body: "disk is full. Try again, or restart Vellum if it keeps happening.",
    });
  });

  it("keeps recent commands short and without repeats", () => {
    const { noteUsed } = useCommands.getState();
    for (const kind of ["a", "b", "a", "c", "d", "e", "f", "g", "h", "i"]) noteUsed(kind);
    expect(useCommands.getState().recent).toEqual(["i", "h", "g", "f", "e", "d", "c", "a"]);
  });
});

describe("shortcutFor", () => {
  it("returns the first chord", () => {
    const keymap = {
      preset: "vellum" as const,
      bindings: { "edit.redo": ["Ctrl+Shift+Z", "Ctrl+Y"] },
      conflicts: [],
    };
    expect(shortcutFor("edit.redo", keymap)).toBe("Ctrl+Shift+Z");
    expect(shortcutFor("file.save", keymap)).toBeUndefined();
  });
});
