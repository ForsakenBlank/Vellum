import { beforeEach, describe, expect, it } from "vitest";

import type { DocumentInfo } from "../bindings/DocumentInfo";
import { nextActive, tabsIn, useDocuments } from "./documents";

function doc(
  id: string,
  window = "main",
  workspace: DocumentInfo["workspace"] = "image",
): DocumentInfo {
  return {
    id,
    title: id,
    path: `/m/${id}.png`,
    kind: "image",
    workspace,
    tool: null,
    inputs: [],
    window,
    dirty: false,
    docType: "placeholder",
  };
}

describe("nextActive", () => {
  const before = [doc("a"), doc("b"), doc("c")];

  it("keeps the active tab when it is still open", () => {
    expect(nextActive(before, before, "b", "main")).toBe("b");
  });

  it("moves to the neighbour when the active tab closes", () => {
    expect(nextActive(before, [doc("a"), doc("c")], "b", "main")).toBe("c");
    expect(nextActive(before, [doc("a"), doc("b")], "c", "main")).toBe("b");
  });

  it("goes home when the last tab closes or the tab moves to another window", () => {
    expect(nextActive([doc("a")], [], "a", "main")).toBeNull();
    expect(nextActive([doc("a")], [doc("a", "doc-1")], "a", "main")).toBeNull();
  });

  it("stays home when nothing was active", () => {
    expect(nextActive(before, before, null, "main")).toBeNull();
  });
});

describe("useDocuments", () => {
  beforeEach(() => {
    useDocuments.setState({ documents: [], summaries: {}, activeId: null, emptyWorkspace: null });
  });

  it("lists only this window's tabs", () => {
    expect(tabsIn([doc("a"), doc("b", "doc-2")], "main").map((d) => d.id)).toEqual(["a"]);
  });

  it("drops summaries of closed documents", () => {
    const store = useDocuments.getState();
    store.setDocuments([doc("a")]);
    store.setSummary("a", {
      canUndo: true,
      canRedo: false,
      undoName: "x",
      redoName: null,
      dirty: true,
    });
    useDocuments.getState().setDocuments([]);
    expect(useDocuments.getState().summaries).toEqual({});
  });

  it("shows a workspace's latest tab, or the empty workspace", () => {
    const store = useDocuments.getState();
    store.setDocuments([doc("a", "main", "video"), doc("b", "main", "video"), doc("c")]);
    store.showWorkspace("video");
    expect(useDocuments.getState().activeId).toBe("b");
    store.showWorkspace("audio");
    expect(useDocuments.getState()).toMatchObject({ activeId: null, emptyWorkspace: "audio" });
  });
});
