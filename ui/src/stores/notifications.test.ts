import { describe, expect, it } from "vitest";

import { job, readyBackend, connectTo } from "../test/fakes";
import { newlyFinished, useJobs } from "./jobs";
import { notificationForJob, outputPath, unreadCount, useNotifications } from "./notifications";

describe("notifications", () => {
  it("describes finished jobs and skips housekeeping", () => {
    const done = job({ state: "succeeded", output: { path: "/out/clip.gif" } });
    expect(notificationForJob(done)).toMatchObject({
      tone: "good",
      title: "Export clip.mp4 is done",
      path: "/out/clip.gif",
    });
    expect(notificationForJob(job({ state: "failed", error: "Disk full." }))).toMatchObject({
      tone: "bad",
      body: "Disk full.",
    });
    expect(notificationForJob(job({ state: "succeeded", background: true }))).toBeNull();
    expect(outputPath(job({ output: { seconds: 2 } }))).toBeNull();
  });

  it("only counts jobs it saw unfinished as newly finished", () => {
    const before = { 1: job({ id: 1, state: "running" }) };
    const after = {
      1: job({ id: 1, state: "succeeded" }),
      2: job({ id: 2, state: "succeeded" }),
    };
    expect(newlyFinished(before, after).map((entry) => entry.id)).toEqual([1]);
  });

  it("posts one when a job finishes while the app is open", async () => {
    const backend = readyBackend().reply("jobs_overview", () => ({
      jobs: [job({ id: 4, state: "running" })],
      pending: [],
    }));
    await connectTo(backend);
    backend.emit("jobs", { type: "updated", job: job({ id: 4, state: "succeeded" }) });

    const { items, toasts } = useNotifications.getState();
    expect(items.map((item) => item.title)).toEqual(["Export clip.mp4 is done"]);
    expect(toasts).toHaveLength(1);
    expect(unreadCount(items)).toBe(1);
    useNotifications.getState().markAllRead();
    expect(unreadCount(useNotifications.getState().items)).toBe(0);
  });

  it("stays quiet when notifications are turned off", async () => {
    const backend = readyBackend().reply("jobs_overview", () => ({
      jobs: [job({ id: 4, state: "running" })],
      pending: [],
    }));
    await connectTo(backend);
    backend.emit("settings.changed", {
      section: "general",
      value: { autosaveSeconds: 30, recentLimit: 30, notifyJobs: false },
    });
    backend.emit("jobs", { type: "updated", job: job({ id: 4, state: "succeeded" }) });
    expect(useNotifications.getState().items).toEqual([]);
  });

  it("opens the result of a job that was asked to open when done", async () => {
    const backend = readyBackend().reply("jobs_overview", () => ({
      jobs: [job({ id: 4, state: "running" })],
      pending: [],
    }));
    await connectTo(backend);
    useJobs.getState().toggleOpenWhenDone(4);
    backend.emit("jobs", {
      type: "updated",
      job: job({ id: 4, state: "succeeded", output: { path: "/out/a.gif" } }),
    });
    expect(backend.callsTo("path_open")).toEqual([{ path: "/out/a.gif" }]);
    expect(useJobs.getState().openWhenDone).toEqual([]);
  });
});
