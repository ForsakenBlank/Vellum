import { describe, expect, it } from "vitest";

import { createApi } from "../lib/api";
import { FakeBackend, doc, sessionWith } from "../test/fakes";
import { useApp } from "./app";
import { activeDocument, openFiles, skippedNotice, useSession, workspaceOf } from "./session";

function connected(backend: FakeBackend) {
  useApp.setState({ api: createApi(backend), status: "ready" });
}

describe("session store", () => {
  it("ignores a view older than the one it has", () => {
    const { applyView } = useSession.getState();
    applyView(sessionWith([doc()], { type: "home" }, 5));
    applyView(sessionWith([], { type: "home" }, 4));
    expect(useSession.getState().session?.documents).toHaveLength(1);
  });

  it("knows what this window shows", () => {
    const video = doc({ id: 3 });
    const session = sessionWith([video], { type: "document", id: 3 });
    useSession.getState().applyView(session);
    expect(activeDocument()).toEqual(video);
    expect(workspaceOf({ type: "document", id: 3 }, session)).toBe("video");
    expect(workspaceOf({ type: "workspace", id: "audio" }, session)).toBe("audio");
    expect(workspaceOf({ type: "home" }, session)).toBeNull();
  });

  it("opens what it can and says what it skipped", async () => {
    const backend = new FakeBackend()
      .reply("files_plan", () => ({
        opens: [{ path: "/a.png", name: "a.png", kind: "image", workspace: "image", tool: null }],
        skipped: [
          { path: "/b.txt", name: "b.txt", reason: "Vellum does not know this file type yet." },
        ],
      }))
      .reply("shell_run", () => sessionWith([doc()], { type: "home" }, 2));
    connected(backend);

    await openFiles(["/a.png", "/b.txt"]);
    expect(useApp.getState().notice).toBe(
      "b.txt was not opened. Vellum does not know this file type yet.",
    );
    expect(backend.callsTo("shell_run")).toHaveLength(1);
    expect(useSession.getState().session?.revision).toBe(2);
  });

  it("does not ask to open anything when nothing can be opened", async () => {
    const backend = new FakeBackend().reply("files_plan", () => ({
      opens: [],
      skipped: [
        { path: "/a.txt", name: "a.txt", reason: "x" },
        { path: "/b.txt", name: "b.txt", reason: "x" },
      ],
    }));
    connected(backend);
    await openFiles(["/a.txt", "/b.txt"]);
    expect(backend.callsTo("shell_run")).toHaveLength(0);
    expect(useApp.getState().notice).toBe("2 files were not opened. a.txt: x");
  });

  it("has nothing to say when nothing was skipped", () => {
    expect(skippedNotice({ opens: [], skipped: [] })).toBeNull();
  });
});
