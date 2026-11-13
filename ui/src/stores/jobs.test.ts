import { describe, expect, it } from "vitest";

import { job } from "../test/fakes";
import { orderJobs, useJobs } from "./jobs";

const names = () => {
  const { jobs, pending } = useJobs.getState();
  return orderJobs(jobs, pending).map((entry) => entry.name);
};

describe("jobs store", () => {
  it("adds, updates and removes jobs from events", () => {
    const { applyEvent } = useJobs.getState();
    applyEvent({ type: "updated", job: job({ id: 1, progress: 0.1 }) });
    applyEvent({ type: "updated", job: job({ id: 1, progress: 0.6 }) });
    expect(useJobs.getState().jobs[1]?.progress).toBe(0.6);

    applyEvent({ type: "reordered", pending: [1] });
    applyEvent({ type: "removed", id: 1 });
    expect(useJobs.getState()).toMatchObject({ jobs: {}, pending: [] });
  });

  it("replaces everything with an overview", () => {
    const { applyEvent, applyOverview } = useJobs.getState();
    applyEvent({ type: "updated", job: job({ id: 7 }) });
    applyOverview({ jobs: [job({ id: 2, state: "queued" })], pending: [2] });
    expect(Object.keys(useJobs.getState().jobs)).toEqual(["2"]);
    expect(useJobs.getState().pending).toEqual([2]);
  });

  it("orders running, then waiting in queue order, then finished newest first", () => {
    useJobs.getState().applyOverview({
      jobs: [
        job({ id: 1, name: "done early", state: "succeeded", finishedMs: 100 }),
        job({ id: 2, name: "running", state: "running" }),
        job({ id: 3, name: "waiting b", state: "queued" }),
        job({ id: 4, name: "waiting a", state: "paused" }),
        job({ id: 5, name: "failed late", state: "failed", finishedMs: 200 }),
        job({ id: 6, name: "not in the order yet", state: "queued" }),
      ],
      pending: [4, 3],
    });
    expect(names()).toEqual([
      "running",
      "waiting a",
      "waiting b",
      "not in the order yet",
      "failed late",
      "done early",
    ]);
  });
});
