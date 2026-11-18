import { create } from "zustand";

import type { JobEvent } from "../bindings/JobEvent";
import type { JobId } from "../bindings/JobId";
import type { JobSnapshot } from "../bindings/JobSnapshot";
import type { JobState } from "../bindings/JobState";
import type { JobsOverview } from "../bindings/JobsOverview";

export interface JobsData {
  jobs: Record<JobId, JobSnapshot>;
  /** waiting jobs in start order, as rust last reported it */
  waiting: JobId[];
  orderRevision: number;
  /** revision each removed job was removed at, so late updates cannot bring it back */
  removedAt: Record<JobId, number>;
}

export const emptyJobs: JobsData = { jobs: {}, waiting: [], orderRevision: 0, removedAt: {} };

/**
 * folds one event into the data. events can arrive out of order because rust sends them
 * from several threads, so anything older than what we already have is ignored.
 */
export function applyJobEvent(data: JobsData, event: JobEvent): JobsData {
  switch (event.type) {
    case "updated": {
      const { job } = event;
      const current = data.jobs[job.id];
      const removedAt = data.removedAt[job.id];
      if ((current && current.revision >= job.revision) || (removedAt ?? -1) >= job.revision) {
        return data;
      }
      return { ...data, jobs: { ...data.jobs, [job.id]: job } };
    }
    case "removed": {
      const jobs = { ...data.jobs };
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete
      delete jobs[event.id];
      return { ...data, jobs, removedAt: { ...data.removedAt, [event.id]: event.revision } };
    }
    case "order": {
      if (event.revision <= data.orderRevision) {
        return data;
      }
      return { ...data, waiting: event.waiting, orderRevision: event.revision };
    }
  }
}

/**
 * folds a full overview from rust into the data. events that arrived while the overview
 * was on its way are newer than it, so they win.
 */
export function mergeOverview(data: JobsData, overview: JobsOverview): JobsData {
  const listed = new Set(overview.jobs.map((job) => job.id));
  const jobs: Record<JobId, JobSnapshot> = {};
  for (const job of Object.values(data.jobs)) {
    // keep jobs the overview does not know about only if we heard of them after it was made
    if (listed.has(job.id) || job.revision > overview.revision) {
      jobs[job.id] = job;
    }
  }
  let next: JobsData = { ...data, jobs };
  for (const job of overview.jobs) {
    next = applyJobEvent(next, { type: "updated", job });
  }
  return applyJobEvent(next, {
    type: "order",
    waiting: overview.waiting,
    revision: overview.revision,
  });
}

/** builds data straight from snapshots, mainly for tests */
export function fromSnapshots(list: JobSnapshot[]): JobsData {
  const revision = Math.max(0, ...list.map((job) => job.revision));
  return mergeOverview(emptyJobs, { jobs: list, waiting: [], revision });
}

const activeStates: JobState[] = ["running", "pausing", "cancelling"];

/**
 * the order the job panel shows: running jobs, then waiting jobs in start order, then
 * paused ones, then finished jobs newest first
 */
export function orderedJobs(data: JobsData): JobSnapshot[] {
  const all = Object.values(data.jobs);
  const waitingIndex = new Map(data.waiting.map((id, index) => [id, index]));
  const group = (job: JobSnapshot) => {
    if (activeStates.includes(job.state)) return 0;
    if (waitingIndex.has(job.id)) return 1;
    if (job.state === "queued" || job.state === "waitingToRetry") return 2;
    if (job.state === "paused") return 3;
    return 4;
  };
  return all.sort((a, b) => {
    const byGroup = group(a) - group(b);
    if (byGroup !== 0) return byGroup;
    if (group(a) === 1) {
      return (waitingIndex.get(a.id) ?? 0) - (waitingIndex.get(b.id) ?? 0);
    }
    if (group(a) === 4) return b.createdAtMs - a.createdAtMs || b.id - a.id;
    return a.id - b.id;
  });
}

interface JobsStore extends JobsData {
  apply: (event: JobEvent) => void;
  merge: (overview: JobsOverview) => void;
}

export const useJobs = create<JobsStore>()((set) => ({
  ...emptyJobs,
  apply: (event) => {
    set((state) => applyJobEvent(state, event));
  },
  merge: (overview) => {
    set((state) => mergeOverview(state, overview));
  },
}));
