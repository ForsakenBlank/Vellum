import { create } from "zustand";

import type { JobEvent } from "../bindings/JobEvent";
import type { JobId } from "../bindings/JobId";
import type { JobSnapshot } from "../bindings/JobSnapshot";
import type { JobsOverview } from "../bindings/JobsOverview";

interface JobsState {
  jobs: Record<JobId, JobSnapshot>;
  /** waiting jobs in the order the queue last told us about */
  pending: JobId[];
  /** jobs whose result opens by itself when they finish. kept in this window only */
  openWhenDone: JobId[];
  applyOverview: (overview: JobsOverview) => void;
  applyEvent: (event: JobEvent) => void;
  toggleOpenWhenDone: (id: JobId) => void;
}

export const useJobs = create<JobsState>()((set) => ({
  jobs: {},
  pending: [],
  openWhenDone: [],

  toggleOpenWhenDone: (id) => {
    set((state) => ({
      openWhenDone: state.openWhenDone.includes(id)
        ? state.openWhenDone.filter((other) => other !== id)
        : [...state.openWhenDone, id],
    }));
  },

  applyOverview: (overview) => {
    set({ jobs: byId(overview.jobs), pending: overview.pending });
  },

  applyEvent: (event) => {
    switch (event.type) {
      case "updated":
        set((state) => ({ jobs: { ...state.jobs, [event.job.id]: event.job } }));
        break;
      case "removed":
        set((state) => ({
          jobs: byId(Object.values(state.jobs).filter((job) => job.id !== event.id)),
          pending: state.pending.filter((id) => id !== event.id),
        }));
        break;
      case "reordered":
        set({ pending: event.pending });
        break;
    }
  },
}));

function byId(list: JobSnapshot[]): Record<JobId, JobSnapshot> {
  const jobs: Record<JobId, JobSnapshot> = {};
  for (const job of list) jobs[job.id] = job;
  return jobs;
}

const WAITING = new Set<JobSnapshot["state"]>(["queued", "paused"]);
const FINISHED = new Set<JobSnapshot["state"]>(["succeeded", "failed", "cancelled"]);

export function isFinished(job: JobSnapshot): boolean {
  return FINISHED.has(job.state);
}

export function isWaiting(job: JobSnapshot): boolean {
  return WAITING.has(job.state);
}

/**
 * jobs that were known and unfinished in `before` and are finished in `after`.
 * jobs seen for the first time do not count, so loading the queue at startup
 * does not announce work that finished long ago
 */
export function newlyFinished(
  before: Record<JobId, JobSnapshot>,
  after: Record<JobId, JobSnapshot>,
): JobSnapshot[] {
  return Object.values(after).filter((job) => {
    const previous = before[job.id];
    return previous !== undefined && !isFinished(previous) && isFinished(job);
  });
}

/**
 * jobs in the order the queue panel shows them: what is running, then what
 * is waiting in the order it will start, then what has finished, newest first
 */
export function orderJobs(jobs: Record<JobId, JobSnapshot>, pending: JobId[]): JobSnapshot[] {
  const all = Object.values(jobs);
  // the queue only announces an order when a job lands away from the end,
  // so a waiting job it has not mentioned yet goes last, oldest first
  const place = new Map(pending.map((id, index) => [id, index]));
  const position = (job: JobSnapshot) => place.get(job.id) ?? pending.length + job.id;

  const active = all.filter((job) => !WAITING.has(job.state) && !isFinished(job));
  const waiting = all.filter((job) => WAITING.has(job.state));
  const finished = all.filter(isFinished);

  active.sort((a, b) => a.id - b.id);
  waiting.sort((a, b) => position(a) - position(b));
  finished.sort((a, b) => (b.finishedMs ?? 0) - (a.finishedMs ?? 0) || b.id - a.id);
  return [...active, ...waiting, ...finished];
}
