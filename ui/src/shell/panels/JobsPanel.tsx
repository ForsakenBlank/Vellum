import {
  ArrowDown,
  ArrowUp,
  Check,
  ExternalLink,
  FolderOpen,
  GripVertical,
  ListChecks,
  Pause,
  Play,
  RotateCcw,
  Trash,
  X,
} from "lucide-react";
import { useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import type { JobId } from "../../bindings/JobId";
import type { JobResource } from "../../bindings/JobResource";
import type { JobSnapshot } from "../../bindings/JobSnapshot";
import type { JobState } from "../../bindings/JobState";
import { Button, IconButton } from "../../components/Button";
import { EmptyState } from "../../components/EmptyState";
import { ProgressBar } from "../../components/ProgressBar";
import { Switch } from "../../components/Switch";
import { formatDuration, formatPercent } from "../../lib/format";
import { inDesktopApp, ipc } from "../../lib/ipc";
import { orderedJobs, resultPath, useJobs } from "../../state/jobs";

const stateLabels: Record<JobState, string> = {
  queued: "Waiting",
  running: "Running",
  pausing: "Pausing",
  paused: "Paused",
  cancelling: "Cancelling",
  waitingToRetry: "Retrying soon",
  completed: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

const resourceLabels: Record<JobResource, string> = {
  cpu: "CPU",
  gpu: "GPU",
  io: "Disk and network",
};

type Run = (action: () => Promise<unknown>) => void;

export function JobsPanel() {
  const jobs = useJobs((s) => s.jobs);
  const waiting = useJobs((s) => s.waiting);
  const list = useMemo(
    () => orderedJobs({ jobs, waiting, orderRevision: 0, removedAt: {} }),
    [jobs, waiting],
  );
  const anyFinished = list.some((job) => isFinished(job.state));
  const [error, setError] = useState<string | null>(null);

  const run: Run = (action) => {
    setError(null);
    action().catch((err: unknown) => {
      setError(String(err));
    });
  };

  return (
    <section aria-label="Jobs" className="flex h-full min-h-0 flex-col">
      <div className="flex items-center justify-between gap-2 px-3 py-1.5">
        <span className="text-xs text-fg-subtle tabular">
          {list.length === 0 ? "Nothing running" : summary(list)}
        </span>
        <Button
          variant="ghost"
          size="sm"
          tooltip="Remove finished jobs from this list"
          command="jobs.clear_finished"
          disabled={!anyFinished}
          onClick={() => {
            run(ipc.clearFinishedJobs);
          }}
        >
          Clear finished
        </Button>
      </div>

      {error && (
        <p
          role="alert"
          className="mx-3 mb-2 rounded-md bg-danger/10 px-2 py-1.5 text-xs text-danger"
        >
          {error}
        </p>
      )}

      {list.length === 0 ? (
        <EmptyState icon={ListChecks} title="No jobs yet" compact>
          Exports, conversions and AI tools show up here while they run.
        </EmptyState>
      ) : (
        <JobList list={list} waiting={waiting} run={run} />
      )}

      {import.meta.env.DEV && inDesktopApp && <SampleJobs run={run} />}
    </section>
  );
}

function summary(list: JobSnapshot[]): string {
  const running = list.filter((j) => j.state === "running").length;
  const queued = list.filter((j) => j.state === "queued").length;
  const parts = [];
  if (running) parts.push(`${running} running`);
  if (queued) parts.push(`${queued} waiting`);
  return parts.join(", ") || `${list.length} finished`;
}

interface JobListProps {
  list: JobSnapshot[];
  waiting: JobId[];
  run: Run;
}

/** the list, with waiting jobs draggable by their handle to change the start order */
function JobList({ list, waiting, run }: JobListProps) {
  const listRef = useRef<HTMLOListElement>(null);
  const [drag, setDrag] = useState<{ id: JobId; before: JobId | null } | null>(null);

  const targetFor = (clientY: number, id: JobId): JobId | null => {
    const rows = listRef.current?.querySelectorAll<HTMLElement>("[data-waiting-job]") ?? [];
    for (const el of rows) {
      const rect = el.getBoundingClientRect();
      const rowId = Number(el.dataset.waitingJob);
      if (clientY < rect.top + rect.height / 2) return rowId === id ? id : rowId;
    }
    return null;
  };

  const startDrag = (id: JobId, event: ReactPointerEvent) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    setDrag({ id, before: id });
  };
  const moveDrag = (event: ReactPointerEvent) => {
    if (!drag) return;
    setDrag({ id: drag.id, before: targetFor(event.clientY, drag.id) });
  };
  const endDrag = () => {
    if (!drag) return;
    const { id, before } = drag;
    setDrag(null);
    const at = waiting.indexOf(id);
    // dropping in its own spot does nothing
    if (before === id || (before !== null && waiting[at + 1] === before)) return;
    if (before === null && at === waiting.length - 1) return;
    run(() => ipc.moveJob(id, before));
  };

  return (
    <ol ref={listRef} className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
      {list.map((job) => (
        <JobRow
          key={job.id}
          job={job}
          run={run}
          waitingIndex={waiting.indexOf(job.id)}
          waiting={waiting}
          dragging={drag?.id === job.id}
          dropBefore={drag !== null && drag.before === job.id && drag.id !== job.id}
          onGrab={startDrag}
          onDrag={moveDrag}
          onDrop={endDrag}
        />
      ))}
    </ol>
  );
}

/** the moving "working on it" bar only makes sense while a job actually runs */
function progressToShow(job: JobSnapshot): number | null {
  if (job.state === "completed") return 1;
  if (job.progress !== null) return job.progress;
  return job.state === "running" ? null : 0;
}

function isFinished(state: JobState) {
  return state === "completed" || state === "failed" || state === "cancelled";
}

interface JobRowProps {
  job: JobSnapshot;
  run: Run;
  waiting: JobId[];
  waitingIndex: number;
  dragging: boolean;
  dropBefore: boolean;
  onGrab: (id: JobId, event: ReactPointerEvent) => void;
  onDrag: (event: ReactPointerEvent) => void;
  onDrop: () => void;
}

function JobRow({
  job,
  run,
  waiting,
  waitingIndex,
  dragging,
  dropBefore,
  onGrab,
  onDrag,
  onDrop,
}: JobRowProps) {
  const { id, state } = job;
  const openWhenDone = useJobs((s) => s.openWhenDone[id] === true);
  const toggleOpenWhenDone = useJobs((s) => s.toggleOpenWhenDone);
  const canPause = state === "queued" || state === "running" || state === "waitingToRetry";
  const canResume = state === "paused" || state === "pausing";
  const canCancel = !isFinished(state) && state !== "cancelling";
  const canRetry = state === "failed" || state === "cancelled";
  const isWaiting = waitingIndex !== -1;
  const path = resultPath(job);

  const details = [stateLabels[state], resourceLabels[job.resource]];
  if (job.progress !== null && !isFinished(state)) details.push(formatPercent(job.progress));
  if (job.etaSecs !== null) details.push(`${formatDuration(job.etaSecs)} left`);
  if (state === "completed") details.push(`took ${formatDuration(job.elapsedSecs)}`);
  if (job.attempt > 0 && !isFinished(state)) details.push(`try ${job.attempt + 1}`);

  const tone =
    state === "failed"
      ? "danger"
      : state === "completed"
        ? "success"
        : state === "paused" || state === "cancelled"
          ? "muted"
          : "accent";

  const move = (offset: -1 | 1) => {
    // before the job two places down moves one down; before the previous one moves up
    const before = offset === -1 ? waiting[waitingIndex - 1] : waiting[waitingIndex + 2];
    run(() => ipc.moveJob(id, before ?? null));
  };

  return (
    <li
      data-waiting-job={isWaiting ? id : undefined}
      className={`group relative mb-1 flex flex-col gap-1.5 rounded-lg border border-transparent px-2 py-2 transition-colors duration-(--motion-fast) hover:border-line hover:bg-raised ${dragging ? "opacity-50" : ""}`}
    >
      {dropBefore && <div className="absolute inset-x-2 -top-1 h-0.5 rounded-full bg-accent" />}
      <div className="flex items-center gap-1.5">
        {isWaiting && (
          <span
            role="button"
            tabIndex={-1}
            aria-label={`Drag ${job.title} to change when it starts`}
            className="-ml-1 cursor-grab touch-none text-fg-subtle hover:text-fg active:cursor-grabbing"
            onPointerDown={(event) => {
              onGrab(id, event);
            }}
            onPointerMove={onDrag}
            onPointerUp={onDrop}
            onPointerCancel={onDrop}
          >
            <GripVertical className="size-3.5" />
          </span>
        )}
        <span className="min-w-0 flex-1 truncate text-sm" title={job.title}>
          {job.title}
        </span>
        {state === "completed" && <Check className="size-3.5 text-success" aria-hidden />}
      </div>
      <ProgressBar value={progressToShow(job)} label={`${job.title} progress`} tone={tone} />
      <div className="text-xs text-fg-subtle tabular">{details.join(" · ")}</div>
      {job.message && !isFinished(state) && (
        <div className="text-xs text-fg-muted">{job.message}</div>
      )}
      {state === "failed" && job.error && (
        <div className="text-xs text-danger">{job.error}. Press Retry to run it again.</div>
      )}
      <div className="flex flex-wrap items-center gap-0.5">
        {canPause && (
          <IconButton
            size="sm"
            label="Pause"
            icon={<Pause className="size-3.5" />}
            onClick={() => {
              run(() => ipc.pauseJob(id));
            }}
          />
        )}
        {canResume && (
          <IconButton
            size="sm"
            label="Resume"
            icon={<Play className="size-3.5" />}
            onClick={() => {
              run(() => ipc.resumeJob(id));
            }}
          />
        )}
        {canRetry && (
          <IconButton
            size="sm"
            label="Retry"
            icon={<RotateCcw className="size-3.5" />}
            onClick={() => {
              run(() => ipc.retryJob(id));
            }}
          />
        )}
        {isWaiting && (
          <>
            <IconButton
              size="sm"
              label="Start sooner"
              icon={<ArrowUp className="size-3.5" />}
              disabled={waitingIndex === 0}
              onClick={() => {
                move(-1);
              }}
            />
            <IconButton
              size="sm"
              label="Start later"
              icon={<ArrowDown className="size-3.5" />}
              disabled={waitingIndex === waiting.length - 1}
              onClick={() => {
                move(1);
              }}
            />
          </>
        )}
        {state === "completed" && path && (
          <>
            <Button
              size="sm"
              variant="secondary"
              tooltip="Open the result with its usual app"
              icon={<ExternalLink className="size-3.5" />}
              onClick={() => {
                run(() => ipc.openResult(path, false));
              }}
            >
              Open
            </Button>
            <IconButton
              size="sm"
              label="Show in folder"
              icon={<FolderOpen className="size-3.5" />}
              onClick={() => {
                run(() => ipc.openResult(path, true));
              }}
            />
          </>
        )}
        <span className="flex-1" />
        {!isFinished(state) && (
          <label className="mr-1 flex items-center gap-1.5 text-2xs text-fg-subtle">
            Open when done
            <Switch
              label="Open the result when this job is done"
              checked={openWhenDone}
              onCheckedChange={() => {
                toggleOpenWhenDone(id);
              }}
            />
          </label>
        )}
        {canCancel && (
          <IconButton
            size="sm"
            label="Cancel"
            icon={<X className="size-3.5" />}
            className="hover:text-danger"
            onClick={() => {
              run(() => ipc.cancelJob(id));
            }}
          />
        )}
        {isFinished(state) && (
          <IconButton
            size="sm"
            label="Remove from this list"
            icon={<Trash className="size-3.5" />}
            onClick={() => {
              run(() => ipc.removeJob(id));
            }}
          />
        )}
      </div>
    </li>
  );
}

function SampleJobs({ run }: { run: Run }) {
  return (
    <footer className="flex flex-col gap-1.5 border-t border-line p-3">
      <h3 className="text-2xs font-medium tracking-wide text-fg-subtle uppercase">
        Sample jobs (development only)
      </h3>
      <div className="flex flex-wrap gap-1.5">
        <Button
          size="sm"
          tooltip="Start a 10 second job on a CPU slot"
          onClick={() => {
            run(() => ipc.startSampleJob(10, "cpu", false));
          }}
        >
          CPU job
        </Button>
        <Button
          size="sm"
          tooltip="Start a 10 second job on a GPU slot"
          onClick={() => {
            run(() => ipc.startSampleJob(10, "gpu", false));
          }}
        >
          GPU job
        </Button>
        <Button
          size="sm"
          tooltip="Start a job that fails halfway once, then retries by itself"
          onClick={() => {
            run(() => ipc.startSampleJob(6, "cpu", true));
          }}
        >
          Flaky job
        </Button>
      </div>
    </footer>
  );
}
