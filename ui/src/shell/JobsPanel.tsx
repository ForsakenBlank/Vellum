import { clsx } from "clsx";
import {
  ArrowDown,
  ArrowUp,
  ExternalLink,
  FolderOpen,
  ListChecks,
  Pause,
  Play,
  RotateCcw,
  SquareArrowOutUpRight,
  Trash2,
  X,
} from "lucide-react";
import { useMemo } from "react";

import type { JobSnapshot } from "../bindings/JobSnapshot";
import { Button, IconButton } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { ProgressBar } from "../components/ProgressBar";
import { perform, useApp } from "../stores/app";
import { isFinished, isWaiting, orderJobs, useJobs } from "../stores/jobs";
import { outputPath } from "../stores/notifications";
import { describeJob } from "./jobText";

/** the job queue as a dockable panel */
export function JobsPanel() {
  const jobs = useJobs((state) => state.jobs);
  const pending = useJobs((state) => state.pending);
  const debugBuild = useApp((state) => state.info?.debugBuild ?? false);
  const ordered = useMemo(() => orderJobs(jobs, pending), [jobs, pending]);
  // housekeeping jobs stay out of the panel
  const visible = ordered.filter((job) => !job.background);
  const anyFinished = visible.some(isFinished);

  // moving a job up or down puts it where its visible neighbour is in the
  // whole queue, hidden housekeeping jobs included
  const queue = ordered.filter(isWaiting);
  const shown = visible.filter(isWaiting);
  const moves = (job: JobSnapshot) => {
    const at = shown.indexOf(job);
    if (at < 0) return null;
    const before = shown[at - 1];
    const after = shown[at + 1];
    return {
      up: before ? queue.indexOf(before) : null,
      down: after ? queue.indexOf(after) : null,
    };
  };

  return (
    <div className="flex h-full flex-col">
      {visible.length === 0 ? (
        <EmptyState
          icon={ListChecks}
          title="Nothing running"
          body="Exports, conversions and other slow work show up here, with progress and time left."
          compact
        />
      ) : (
        <ul aria-label="Jobs" className="flex-1 overflow-y-auto">
          {visible.map((job) => (
            <JobRow key={job.id} job={job} moves={moves(job)} />
          ))}
        </ul>
      )}
      {(anyFinished || debugBuild) && (
        <footer className="flex shrink-0 flex-wrap items-center gap-2 border-t border-line px-3 py-2">
          {anyFinished && (
            <Button
              tip="Remove finished jobs from this list"
              className="h-7 px-2 text-xs"
              onClick={() => void perform((api) => api.jobs.clearFinished())}
            >
              Clear finished
            </Button>
          )}
          {debugBuild && (
            <>
              <Button
                tip="Add a pretend job that runs for 8 seconds, for trying the queue"
                className="h-7 px-2 text-xs"
                onClick={() => void perform((api) => api.jobs.runSample(8, false))}
              >
                Sample job
              </Button>
              <Button
                tip="Add a pretend job that stops with an error half way"
                className="h-7 px-2 text-xs"
                onClick={() => void perform((api) => api.jobs.runSample(4, true))}
              >
                One that fails
              </Button>
            </>
          )}
        </footer>
      )}
    </div>
  );
}

interface Moves {
  /** where moving up or down puts the job in the queue, null at either end */
  up: number | null;
  down: number | null;
}

function JobRow({ job, moves }: { job: JobSnapshot; moves: Moves | null }) {
  const tone =
    job.state === "succeeded"
      ? "good"
      : job.state === "failed"
        ? "bad"
        : job.state === "running"
          ? "accent"
          : "idle";
  const showBar = !(job.state === "queued" && job.progress === null) && job.state !== "cancelled";

  return (
    <li className="border-b border-line px-3 py-2.5">
      <div className="flex items-center gap-0.5">
        <span className="min-w-0 flex-1 truncate text-sm" title={job.name}>
          {job.name}
        </span>
        <JobActions job={job} moves={moves} />
      </div>
      <p
        className={clsx(
          "mt-0.5 text-xs select-text",
          showBar && "mb-2",
          job.state === "failed" ? "text-bad" : "text-ink-soft",
        )}
      >
        {describeJob(job)}
      </p>
      {showBar && <ProgressBar value={job.progress} label={`${job.name} progress`} tone={tone} />}
    </li>
  );
}

function JobActions({ job, moves }: { job: JobSnapshot; moves: Moves | null }) {
  const { id } = job;
  const openWhenDone = useJobs((state) => state.openWhenDone.includes(id));
  const toggleOpenWhenDone = useJobs((state) => state.toggleOpenWhenDone);
  const path = outputPath(job);

  const pause = (
    <IconButton label="Pause" onClick={() => void perform((api) => api.jobs.pause(id))}>
      <Pause className="size-3.5" />
    </IconButton>
  );
  const resume = (
    <IconButton label="Resume" onClick={() => void perform((api) => api.jobs.resume(id))}>
      <Play className="size-3.5" />
    </IconButton>
  );
  const cancel = (
    <IconButton label="Cancel" onClick={() => void perform((api) => api.jobs.cancel(id))}>
      <X className="size-3.5" />
    </IconButton>
  );
  const retry = job.canRetry && (
    <IconButton label="Try again" onClick={() => void perform((api) => api.jobs.retry(id))}>
      <RotateCcw className="size-3.5" />
    </IconButton>
  );
  const remove = (
    <IconButton label="Remove from list" onClick={() => void perform((api) => api.jobs.remove(id))}>
      <Trash2 className="size-3.5" />
    </IconButton>
  );
  const whenDone = (
    <IconButton
      label={openWhenDone ? "Do not open the result when done" : "Open the result when done"}
      active={openWhenDone}
      aria-pressed={openWhenDone}
      className={clsx(openWhenDone && "text-accent")}
      onClick={() => {
        toggleOpenWhenDone(id);
      }}
    >
      <SquareArrowOutUpRight className="size-3.5" />
    </IconButton>
  );
  // waiting jobs can be moved up and down the line
  const moveTo = (index: number | null) => {
    if (index !== null) void perform((api) => api.jobs.moveTo(id, index));
  };
  const order = moves && (
    <>
      <IconButton
        label="Move up the queue"
        disabled={moves.up === null}
        onClick={() => {
          moveTo(moves.up);
        }}
      >
        <ArrowUp className="size-3.5" />
      </IconButton>
      <IconButton
        label="Move down the queue"
        disabled={moves.down === null}
        onClick={() => {
          moveTo(moves.down);
        }}
      >
        <ArrowDown className="size-3.5" />
      </IconButton>
    </>
  );

  switch (job.state) {
    case "queued":
      return (
        <>
          {order}
          {whenDone}
          {pause}
          {cancel}
        </>
      );
    case "running":
      return (
        <>
          {whenDone}
          {pause}
          {cancel}
        </>
      );
    case "paused":
      return (
        <>
          {order}
          {resume}
          {cancel}
        </>
      );
    case "pausing":
      return cancel;
    case "cancelling":
      return null;
    case "succeeded":
      return (
        <>
          {path && (
            <>
              <IconButton label="Open" onClick={() => void perform((api) => api.files.open(path))}>
                <ExternalLink className="size-3.5" />
              </IconButton>
              <IconButton
                label="Show in folder"
                onClick={() => void perform((api) => api.files.reveal(path))}
              >
                <FolderOpen className="size-3.5" />
              </IconButton>
            </>
          )}
          {remove}
        </>
      );
    case "failed":
    case "cancelled":
      return (
        <>
          {retry}
          {remove}
        </>
      );
  }
}
