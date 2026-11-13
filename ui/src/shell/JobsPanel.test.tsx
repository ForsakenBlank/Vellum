import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Tooltip } from "radix-ui";
import { describe, expect, it } from "vitest";

import { createApi } from "../lib/api";
import { useApp } from "../stores/app";
import { useJobs } from "../stores/jobs";
import { FakeBackend, job } from "../test/fakes";
import { JobsPanel } from "./JobsPanel";

function show(...jobs: ReturnType<typeof job>[]) {
  const backend = new FakeBackend();
  useApp.setState({ api: createApi(backend), status: "ready" });
  useJobs.getState().applyOverview({ jobs, pending: [] });
  render(
    <Tooltip.Provider>
      <JobsPanel />
    </Tooltip.Provider>,
  );
  return backend;
}

const row = (name: string) => {
  const item = screen.getByText(name).closest("li");
  if (!item) throw new Error(`no row for ${name}`);
  return within(item);
};

describe("JobsPanel", () => {
  it("explains itself when there is nothing to show", () => {
    show();
    expect(screen.getByText("Nothing running")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Clear finished" })).not.toBeInTheDocument();
  });

  it("shows progress and lets a running job be paused or cancelled", async () => {
    const backend = show(job({ id: 4, progress: 0.3, etaMs: 20_000 }));
    const running = row("Export clip.mp4");
    expect(running.getByText("30% · about 20 s left")).toBeInTheDocument();
    expect(running.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "30");

    await userEvent.click(running.getByRole("button", { name: "Pause" }));
    await userEvent.click(running.getByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("job_pause")).toEqual([{ id: 4 }]);
    expect(backend.callsTo("job_cancel")).toEqual([{ id: 4 }]);
  });

  it("offers resume for a paused job", async () => {
    const backend = show(job({ id: 2, state: "paused", progress: 0.5 }));
    await userEvent.click(row("Export clip.mp4").getByRole("button", { name: "Resume" }));
    expect(backend.callsTo("job_resume")).toEqual([{ id: 2 }]);
  });

  it("shows why a job failed and offers to try again", async () => {
    const failed = job({
      id: 3,
      state: "failed",
      error: "The disk is full. Free some space and try again.",
      canRetry: true,
      finishedMs: 5,
    });
    const backend = show(failed);
    const item = row("Export clip.mp4");
    expect(item.getByText(/The disk is full/)).toBeInTheDocument();

    await userEvent.click(item.getByRole("button", { name: "Try again" }));
    await userEvent.click(item.getByRole("button", { name: "Remove from list" }));
    expect(backend.callsTo("job_retry")).toEqual([{ id: 3 }]);
    expect(backend.callsTo("job_remove")).toEqual([{ id: 3 }]);
  });

  it("hides retry when the job cannot run again", () => {
    show(job({ state: "failed", error: "This job crashed.", canRetry: false }));
    expect(screen.queryByRole("button", { name: "Try again" })).not.toBeInTheDocument();
  });

  it("clears finished jobs in one go", async () => {
    const backend = show(job({ state: "succeeded", progress: 1, finishedMs: 5 }));
    await userEvent.click(screen.getByRole("button", { name: "Clear finished" }));
    expect(backend.callsTo("jobs_clear_finished")).toHaveLength(1);
  });

  it("keeps housekeeping jobs out of the list", () => {
    show(job({ name: "Thumbnails", background: true }));
    expect(screen.queryByText("Thumbnails")).not.toBeInTheDocument();
  });

  it("moves waiting jobs up and down the queue", async () => {
    const backend = show(
      job({ id: 1, name: "First", state: "queued" }),
      job({ id: 2, name: "Hidden", state: "queued", background: true }),
      job({ id: 3, name: "Second", state: "queued" }),
    );
    expect(row("First").getByRole("button", { name: "Move up the queue" })).toBeDisabled();
    await userEvent.click(row("Second").getByRole("button", { name: "Move up the queue" }));
    // it lands where its visible neighbour is, past the hidden job
    expect(backend.callsTo("job_move")).toEqual([{ id: 3, index: 0 }]);
  });

  it("opens or shows a finished job's file", async () => {
    const backend = show(
      job({ id: 6, state: "succeeded", output: { path: "/out/clip.gif" }, finishedMs: 5 }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Open" }));
    await userEvent.click(screen.getByRole("button", { name: "Show in folder" }));
    expect(backend.callsTo("path_open")).toEqual([{ path: "/out/clip.gif" }]);
    expect(backend.callsTo("path_reveal")).toEqual([{ path: "/out/clip.gif" }]);
  });

  it("remembers to open a running job's result when it is done", async () => {
    show(job({ id: 5 }));
    await userEvent.click(screen.getByRole("button", { name: "Open the result when done" }));
    expect(useJobs.getState().openWhenDone).toEqual([5]);
    expect(
      screen.getByRole("button", { name: "Do not open the result when done" }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  it("puts a failed action in the notice bar", async () => {
    const backend = show(job({ id: 8 }));
    backend.fail("job_pause", {
      code: "job.not_found",
      message: "That job is no longer in the queue.",
    });
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(useApp.getState().notice).toBe("That job is no longer in the queue.");
  });
});
