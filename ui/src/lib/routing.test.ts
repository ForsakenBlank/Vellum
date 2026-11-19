import { describe, expect, it } from "vitest";

import type { Workspace } from "../bindings/Workspace";
import { quickTools } from "../workspaces/quickTools";
import { workspaceInfo, workspaces } from "../workspaces/workspaces";
import { extensionOf, fileKinds, fileName, kindOf, planDrop } from "./routing";

describe("file kinds", () => {
  it("reads the table rust generated", () => {
    expect(kindOf("/a/clip.MOV")?.workspace).toBe("video");
    expect(kindOf("C:\\Users\\me\\photo.jpeg")?.kind).toBe("image");
    expect(kindOf("notes.txt")).toBeUndefined();
    expect(kindOf(".bashrc")).toBeUndefined();
  });

  it("splits paths on both slash styles", () => {
    expect(fileName("C:\\a\\b.png")).toBe("b.png");
    expect(fileName("/a/b/")).toBe("b");
    expect(extensionOf("/a/archive.tar.GZ")).toBe("gz");
  });

  it("has a workspace for every kind rust knows", () => {
    const ids = workspaces.map((w) => w.id);
    for (const row of fileKinds) {
      expect(ids).toContain(row.workspace);
      expect(workspaceInfo(row.workspace).id).toBe(row.workspace);
    }
  });

  it("only lets tools accept kinds that exist", () => {
    const kinds = fileKinds.map((row) => row.kind);
    for (const tool of quickTools) {
      for (const kind of tool.accepts) expect(kinds).toContain(kind);
    }
  });
});

describe("planDrop on the window", () => {
  it("opens one file in its workspace", () => {
    const plan = planDrop(["/m/holiday.mp4"], { type: "window" });
    expect(plan).toMatchObject({
      action: "open",
      files: ["/m/holiday.mp4"],
      workspace: "video" satisfies Workspace,
      title: "Open holiday.mp4 in Video",
      detail: "A video file.",
    });
  });

  it("opens several files and says where each goes", () => {
    const plan = planDrop(["/m/a.png", "/m/b.mp3", "/m/c.wav"], { type: "window" });
    expect(plan.title).toBe("Open 3 files in Image, Audio");
    expect(plan.detail).toBe("Each file opens in its own tab.");
  });

  it("skips files it cannot open but keeps the rest", () => {
    const plan = planDrop(["/m/a.png", "/m/readme.txt"], { type: "window" });
    expect(plan).toMatchObject({
      action: "open",
      files: ["/m/a.png"],
      detail: "readme.txt will be skipped.",
    });
  });

  it("rejects drops with nothing it can open", () => {
    expect(planDrop(["/m/a.txt", "/m/b.txt"], { type: "window" })).toMatchObject({
      action: "reject",
      title: "Vellum cannot open .txt files yet",
    });
    expect(planDrop([], { type: "window" }).action).toBe("reject");
  });
});

describe("planDrop on a quick tool", () => {
  it("opens the tool with the files it takes", () => {
    const plan = planDrop(["/m/clip.mp4"], { type: "tool", tool: "video_to_gif" });
    expect(plan).toMatchObject({
      action: "tool",
      tool: "video_to_gif",
      title: "Make a GIF from clip.mp4",
      detail: "Opens in Video to GIF.",
    });
  });

  it("leaves out files the tool cannot take", () => {
    const plan = planDrop(["/m/a.mp4", "/m/b.png", "/m/c.mov"], {
      type: "tool",
      tool: "video_to_gif",
    });
    expect(plan).toMatchObject({
      action: "tool",
      files: ["/m/a.mp4", "/m/c.mov"],
      title: "Make a GIF from 2 files",
      detail: "b.png will be skipped.",
    });
  });

  it("explains when nothing fits", () => {
    expect(planDrop(["/m/a.png"], { type: "tool", tool: "video_to_gif" })).toMatchObject({
      action: "reject",
      title: "Video to GIF needs a video",
    });
    expect(planDrop(["/m/a.mp4"], { type: "tool", tool: "screen_record" })).toMatchObject({
      action: "reject",
      title: "Screen record makes new videos, it does not take files",
    });
  });

  it("shrinks anything with a size", () => {
    expect(planDrop(["/m/song.flac"], { type: "tool", tool: "export_to_size" }).title).toBe(
      "Shrink song.flac to 10 MB",
    );
  });
});
