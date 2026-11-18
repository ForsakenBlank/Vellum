import { describe, expect, it } from "vitest";

import type { JobSnapshot } from "../bindings/JobSnapshot";
import { applyJobEvent, emptyJobs, fromSnapshots, mergeOverview, orderedJobs } from "./jobs";

function job(id: number, revision: number, overrides: Partial<JobSnapshot> = {}): JobSnapshot {
  return {
    id,
    title: `Job ${id}`,
    resource: "cpu",
    priority: "normal",
    state: "queued",
    progress: null,
    message: null,
    etaSecs: null,
    elapsedSecs: 0,
    attempt: 0,
    maxRetries: 0,
    error: null,
    result: null,
    createdAtMs: 1000 + id,
    revision,
    ...overrides,
  };
}

describe("applyJobEvent", () => {
  it("adds and updates jobs", () => {
    let data = applyJobEvent(emptyJobs, { type: "updated", job: job(1, 1) });
    data = applyJobEvent(data, { type: "updated", job: job(1, 2, { state: "running" }) });
    expect(data.jobs[1]?.state).toBe("running");
  });

  it("ignores updates older than what it has", () => {
    let data = applyJobEvent(emptyJobs, {
      type: "updated",
      job: job(1, 5, { state: "completed" }),
    });
    const before = data;
    data = applyJobEvent(data, { type: "updated", job: job(1, 3, { state: "running" }) });
    expect(data).toBe(before);
    expect(data.jobs[1]?.state).toBe("completed");
  });

  it("does not bring back a removed job from a late update", () => {
    let data = applyJobEvent(emptyJobs, { type: "updated", job: job(1, 1) });
    data = applyJobEvent(data, { type: "removed", id: 1, revision: 9 });
    expect(data.jobs[1]).toBeUndefined();
    data = applyJobEvent(data, { type: "updated", job: job(1, 8, { state: "completed" }) });
    expect(data.jobs[1]).toBeUndefined();
  });

  it("keeps only the newest waiting order", () => {
    let data = applyJobEvent(emptyJobs, { type: "order", waiting: [2, 1], revision: 4 });
    data = applyJobEvent(data, { type: "order", waiting: [1, 2], revision: 3 });
    expect(data.waiting).toEqual([2, 1]);
  });
});

describe("mergeOverview", () => {
  it("keeps events that are newer than the overview", () => {
    // job 1 changed and job 2 was created after rust built the overview at revision 10
    let data = applyJobEvent(emptyJobs, { type: "updated", job: job(1, 11, { state: "running" }) });
    data = applyJobEvent(data, { type: "updated", job: job(2, 12) });
    // job 3 is stale, rust no longer has it
    data = applyJobEvent(data, { type: "updated", job: job(3, 4) });

    data = mergeOverview(data, {
      jobs: [job(1, 10, { state: "queued" }), job(4, 10, { state: "paused" })],
      waiting: [1],
      revision: 10,
    });

    expect(data.jobs[1]?.state).toBe("running");
    expect(data.jobs[2]).toBeDefined();
    expect(data.jobs[3]).toBeUndefined();
    expect(data.jobs[4]?.state).toBe("paused");
    expect(data.waiting).toEqual([1]);
  });
});

describe("orderedJobs", () => {
  it("shows running, then waiting in order, then paused, then finished newest first", () => {
    const data = {
      ...fromSnapshots([
        job(1, 1, { state: "completed" }),
        job(2, 2, { state: "running" }),
        job(3, 3),
        job(4, 4),
        job(5, 5, { state: "paused" }),
        job(6, 6, { state: "failed" }),
      ]),
      waiting: [4, 3],
    };
    expect(orderedJobs(data).map((j) => j.id)).toEqual([2, 4, 3, 5, 6, 1]);
  });
});
