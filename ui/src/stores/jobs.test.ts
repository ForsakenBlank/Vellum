import { describe, expect, it } from "vitest";
import type { JobInfo } from "../bindings/JobInfo";
import { activeJobCount, emptyJobs, jobsFromList, reduceJobs } from "./jobs";

function job(id: number, patch: Partial<JobInfo> = {}): JobInfo {
  return {
    id,
    label: `job ${id}`,
    kind: "cpu",
    priority: "normal",
    status: "queued",
    progress: 0,
    message: null,
    etaSecs: null,
    error: null,
    attempts: 0,
    result: null,
    ...patch,
  };
}

describe("reduceJobs", () => {
  it("adds new jobs in arrival order and updates known ones in place", () => {
    let state = reduceJobs(emptyJobs, { type: "changed", job: job(2) });
    state = reduceJobs(state, { type: "changed", job: job(1) });
    state = reduceJobs(state, { type: "changed", job: job(2, { status: "running" }) });
    expect(state.order).toEqual([2, 1]);
    expect(state.byId[2]?.status).toBe("running");
  });

  it("applies progress without touching other fields", () => {
    let state = jobsFromList([job(1, { status: "running", label: "Export" })]);
    state = reduceJobs(state, {
      type: "progress",
      id: 1,
      progress: 0.4,
      etaSecs: 12,
      message: "Pass 1 of 2",
    });
    expect(state.byId[1]).toMatchObject({
      label: "Export",
      status: "running",
      progress: 0.4,
      etaSecs: 12,
      message: "Pass 1 of 2",
    });
  });

  it("ignores progress for unknown jobs", () => {
    const state = reduceJobs(emptyJobs, {
      type: "progress",
      id: 9,
      progress: 1,
      etaSecs: null,
      message: null,
    });
    expect(state).toBe(emptyJobs);
  });

  it("removes jobs", () => {
    let state = jobsFromList([job(1), job(2)]);
    state = reduceJobs(state, { type: "removed", id: 1 });
    expect(state.order).toEqual([2]);
    expect(state.byId[1]).toBeUndefined();
    expect(reduceJobs(state, { type: "removed", id: 1 })).toBe(state);
  });
});

describe("activeJobCount", () => {
  it("counts queued, running and paused jobs only", () => {
    const state = jobsFromList([
      job(1, { status: "running" }),
      job(2, { status: "paused" }),
      job(3, { status: "queued" }),
      job(4, { status: "completed" }),
      job(5, { status: "failed" }),
    ]);
    expect(activeJobCount(state)).toBe(3);
  });
});
