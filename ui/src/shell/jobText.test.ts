import { describe, expect, it } from "vitest";

import { job } from "../test/fakes";
import { describeJob } from "./jobText";

describe("describeJob", () => {
  it("combines progress, message and time left while running", () => {
    const running = job({ progress: 0.42, message: "Pass 1 of 2", etaMs: 12_000 });
    expect(describeJob(running)).toBe("42% · Pass 1 of 2 · about 12 s left");
    expect(describeJob(job())).toBe("Running");
  });

  it("says where a paused job stopped", () => {
    expect(describeJob(job({ state: "paused", progress: 0.5 }))).toBe("Paused at 50%");
    expect(describeJob(job({ state: "paused" }))).toBe("Paused");
  });

  it("tells a first wait from a retry", () => {
    expect(describeJob(job({ state: "queued", attempt: 0 }))).toBe("Waiting");
    expect(describeJob(job({ state: "queued", attempt: 1, maxAttempts: 3 }))).toBe(
      "Waiting to try again (1 of 3)",
    );
  });

  it("shows the error of a failed job and the time of a finished one", () => {
    expect(describeJob(job({ state: "failed", error: "The disk is full." }))).toBe(
      "The disk is full.",
    );
    expect(describeJob(job({ state: "succeeded", elapsedMs: 65_000 }))).toBe("Done in 1 min 5 s");
  });
});
