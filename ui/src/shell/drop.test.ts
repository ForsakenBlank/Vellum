import { describe, expect, it } from "vitest";

import type { OpenPlan } from "../bindings/OpenPlan";
import { readyBackend, connectTo } from "../test/fakes";
import { describeDrop, targetFromKey, useDrop } from "./drop";

const open = (name: string, workspace: string, tool: string | null = null) => ({
  path: `/m/${name}`,
  name,
  kind: "video" as const,
  workspace,
  tool,
});

describe("dropping files", () => {
  it("reads targets from data-drop values", () => {
    expect(targetFromKey("tool:video-to-gif")).toEqual({ type: "tool", id: "video-to-gif" });
    expect(targetFromKey("workspace:audio")).toEqual({ type: "workspace", id: "audio" });
    expect(targetFromKey(null)).toEqual({ type: "auto" });
    expect(targetFromKey("nonsense")).toEqual({ type: "auto" });
  });

  it("says where files will open in plain words", () => {
    const one: OpenPlan = { opens: [open("clip.mp4", "video")], skipped: [] };
    expect(describeDrop(one, { type: "auto" })).toEqual({
      ok: true,
      headline: "Open in Video",
      detail: "clip.mp4",
    });

    const mixed: OpenPlan = {
      opens: [open("a.mp4", "video"), open("b.png", "image"), open("c.png", "image")],
      skipped: [
        { path: "/m/x.txt", name: "x.txt", reason: "Vellum does not know this file type yet." },
      ],
    };
    expect(describeDrop(mixed, { type: "auto" })).toEqual({
      ok: true,
      headline: "Open 3 files in 2 workspaces",
      detail:
        "a.mp4, b.png and 1 more. x.txt will be skipped: Vellum does not know this file type yet.",
    });
  });

  it("uses the tool's own words over a quick tool card", () => {
    const gif: OpenPlan = { opens: [open("clip.mp4", "quick-tools", "video-to-gif")], skipped: [] };
    expect(describeDrop(gif, { type: "tool", id: "video-to-gif" }).headline).toBe("Make a GIF");

    const refused: OpenPlan = {
      opens: [],
      skipped: [{ path: "/m/a.png", name: "a.png", reason: "This tool only takes videos." }],
    };
    expect(describeDrop(refused, { type: "tool", id: "video-to-gif" })).toEqual({
      ok: false,
      headline: "This file can't be opened",
      detail: "This tool only takes videos.",
    });
  });

  it("asks the backend for a plan while hovering and opens on drop", async () => {
    const plan: OpenPlan = { opens: [open("clip.mp4", "video")], skipped: [] };
    const backend = readyBackend().reply("files_plan", () => plan);
    await connectTo(backend);

    backend.drag({ type: "enter", paths: ["/m/clip.mp4"], x: 10, y: 10 });
    expect(useDrop.getState().paths).toEqual(["/m/clip.mp4"]);
    await expect.poll(() => useDrop.getState().plan).toEqual(plan);
    expect(backend.callsTo("files_plan")[0]).toEqual({
      paths: ["/m/clip.mp4"],
      target: { type: "auto" },
    });

    backend.drag({ type: "drop", paths: ["/m/clip.mp4"], x: 10, y: 10 });
    expect(useDrop.getState().paths).toBeNull();
    await expect
      .poll(() => backend.callsTo("shell_run"))
      .toEqual([
        {
          record: {
            id: "shell.open",
            params: { paths: ["/m/clip.mp4"], target: { type: "auto" }, window: "main" },
          },
        },
      ]);
  });

  it("forgets the drag when the files leave the window", async () => {
    const backend = readyBackend();
    await connectTo(backend);
    backend.drag({ type: "enter", paths: ["/m/a.png"], x: 0, y: 0 });
    backend.drag({ type: "leave" });
    expect(useDrop.getState()).toMatchObject({ paths: null, plan: null });
  });
});
