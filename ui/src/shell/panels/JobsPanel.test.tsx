import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { JobSnapshot } from "../../bindings/JobSnapshot";
import { TooltipProvider } from "../../components/Tooltip";
import { ipc } from "../../lib/ipc";
import { emptyJobs, fromSnapshots, mergeOverview, useJobs } from "../../state/jobs";
import { JobsPanel } from "./JobsPanel";

vi.mock("../../lib/ipc", () => {
  const ok = () => Promise.resolve(null);
  return {
    inDesktopApp: false,
    ipc: {
      pauseJob: vi.fn(ok),
      resumeJob: vi.fn(ok),
      cancelJob: vi.fn(ok),
      retryJob: vi.fn(ok),
      removeJob: vi.fn(ok),
      clearFinishedJobs: vi.fn(ok),
      moveJob: vi.fn(ok),
      openResult: vi.fn(ok),
    },
  };
});

function job(id: number, overrides: Partial<JobSnapshot>): JobSnapshot {
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
    background: false,
    owner: null,
    subject: null,
    error: null,
    result: null,
    createdAtMs: id,
    revision: id,
    ...overrides,
  };
}

function renderPanel() {
  render(
    <TooltipProvider>
      <JobsPanel />
    </TooltipProvider>,
  );
}

function row(title: string) {
  const item = screen.getByText(title).closest("li");
  if (!item) throw new Error(`no row for ${title}`);
  return within(item);
}

describe("JobsPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useJobs.setState(emptyJobs);
  });

  it("explains itself when empty", () => {
    renderPanel();
    expect(screen.getByText(/no jobs yet/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Clear finished" })).toBeDisabled();
  });

  it("shows progress and time left for a running job", () => {
    useJobs.setState(
      fromSnapshots([
        job(1, { title: "Export clip", state: "running", progress: 0.4, etaSecs: 90 }),
      ]),
    );
    renderPanel();
    const exportRow = row("Export clip");
    expect(exportRow.getByText("Running · CPU · 40% · 1 min 30 s left")).toBeInTheDocument();
    expect(exportRow.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "40");
  });

  it("only shows the unknown progress bar while running", () => {
    useJobs.setState(
      fromSnapshots([
        job(1, { title: "Thinking", state: "running" }),
        job(2, { title: "In line", state: "queued" }),
      ]),
    );
    renderPanel();
    expect(row("Thinking").getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");
    expect(row("In line").getByRole("progressbar")).toHaveAttribute("aria-valuenow", "0");
  });

  it("sends pause, resume and cancel to rust", async () => {
    const user = userEvent.setup();
    useJobs.setState(
      fromSnapshots([
        job(1, { title: "Running job", state: "running" }),
        job(2, { title: "Paused job", state: "paused" }),
      ]),
    );
    renderPanel();

    await user.click(row("Running job").getByRole("button", { name: "Pause" }));
    expect(ipc.pauseJob).toHaveBeenCalledWith(1);

    await user.click(row("Paused job").getByRole("button", { name: "Resume" }));
    expect(ipc.resumeJob).toHaveBeenCalledWith(2);

    await user.click(row("Paused job").getByRole("button", { name: "Cancel" }));
    expect(ipc.cancelJob).toHaveBeenCalledWith(2);
  });

  it("offers retry with a clear message when a job fails", async () => {
    const user = userEvent.setup();
    useJobs.setState(
      fromSnapshots([job(3, { title: "Upscale", state: "failed", error: "Out of memory" })]),
    );
    renderPanel();
    const failed = row("Upscale");
    expect(failed.getByText("Out of memory. Press Retry to run it again.")).toBeInTheDocument();
    expect(failed.queryByRole("button", { name: "Pause" })).not.toBeInTheDocument();

    await user.click(failed.getByRole("button", { name: "Retry" }));
    expect(ipc.retryJob).toHaveBeenCalledWith(3);
    await user.click(screen.getByRole("button", { name: "Clear finished" }));
    expect(ipc.clearFinishedJobs).toHaveBeenCalled();
  });

  it("shows an error when rust refuses an action", async () => {
    const user = userEvent.setup();
    vi.mocked(ipc.pauseJob).mockRejectedValueOnce("cannot pause job 1 while it is completed");
    useJobs.setState(fromSnapshots([job(1, { title: "Quick job", state: "running" })]));
    renderPanel();
    await user.click(row("Quick job").getByRole("button", { name: "Pause" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "cannot pause job 1 while it is completed",
    );
  });

  it("moves waiting jobs earlier and later in line", async () => {
    const user = userEvent.setup();
    const list = [1, 2, 3].map((id) => job(id, { title: `Waiting ${id}`, state: "queued" }));
    useJobs.setState(mergeOverview(emptyJobs, { jobs: list, waiting: [1, 2, 3], revision: 10 }));
    renderPanel();

    expect(row("Waiting 1").getByRole("button", { name: "Start sooner" })).toBeDisabled();
    await user.click(row("Waiting 1").getByRole("button", { name: "Start later" }));
    expect(ipc.moveJob).toHaveBeenCalledWith(1, 3);
    await user.click(row("Waiting 2").getByRole("button", { name: "Start later" }));
    expect(ipc.moveJob).toHaveBeenLastCalledWith(2, null);
    await user.click(row("Waiting 3").getByRole("button", { name: "Start sooner" }));
    expect(ipc.moveJob).toHaveBeenLastCalledWith(3, 2);
  });

  it("opens the result of a finished job", async () => {
    const user = userEvent.setup();
    useJobs.setState(
      fromSnapshots([
        job(4, { title: "Make GIF", state: "completed", result: { path: "/out/a.gif" } }),
      ]),
    );
    renderPanel();
    await user.click(row("Make GIF").getByRole("button", { name: "Open" }));
    expect(ipc.openResult).toHaveBeenCalledWith("/out/a.gif", false);
    await user.click(row("Make GIF").getByRole("button", { name: "Show in folder" }));
    expect(ipc.openResult).toHaveBeenLastCalledWith("/out/a.gif", true);
  });

  it("remembers to open a result when the job is done", async () => {
    const user = userEvent.setup();
    useJobs.setState({
      ...fromSnapshots([job(5, { title: "Long export", state: "running" })]),
      openWhenDone: {},
    });
    renderPanel();
    await user.click(row("Long export").getByRole("switch", { name: /open the result/i }));
    expect(useJobs.getState().openWhenDone[5]).toBe(true);
  });
});
