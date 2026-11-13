use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex, MutexGuard};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::{
    EtaEstimator, Job, JobError, JobEvent, JobId, JobKind, JobLimits, JobQueueError, JobResult,
    JobSnapshot, JobSpec, JobState, Priority,
};
use crate::event::EventBus;
use crate::time::unix_ms;

const CANCEL: u8 = 1;
const PAUSE: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobQueueConfig {
    pub limits: JobLimits,
    /// progress events for one job are at least this far apart
    pub progress_interval: Duration,
    /// finished jobs kept around for the queue panel before the oldest are dropped
    pub keep_finished: usize,
}

impl Default for JobQueueConfig {
    fn default() -> Self {
        Self {
            limits: JobLimits::default(),
            progress_interval: Duration::from_millis(100),
            keep_finished: 200,
        }
    }
}

/// every job plus the order the waiting ones will start in
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobsOverview {
    /// oldest first
    pub jobs: Vec<JobSnapshot>,
    pub pending: Vec<JobId>,
}

/// the part of a job's state its own thread reads without taking the queue lock
#[derive(Default)]
struct Control {
    flags: AtomicU8,
    /// wakes the job's thread out of a pause or a sleep
    wake: Condvar,
}

impl Control {
    fn has(&self, flag: u8) -> bool {
        self.flags.load(Ordering::Acquire) & flag != 0
    }

    fn set(&self, flag: u8) {
        self.flags.fetch_or(flag, Ordering::AcqRel);
    }

    fn clear(&self, flag: u8) {
        self.flags.fetch_and(!flag, Ordering::AcqRel);
    }
}

struct Entry {
    spec: JobSpec,
    state: JobState,
    /// here whenever the job is not running, so it can be started or retried
    job: Option<Box<dyn Job>>,
    control: Arc<Control>,
    /// the job's thread is alive and waiting inside a checkpoint
    parked: bool,
    holds_slot: bool,
    progress: Option<f32>,
    message: Option<String>,
    eta: EtaEstimator,
    attempt: u32,
    retry_at: Option<Instant>,
    error: Option<String>,
    output: Option<Value>,
    created_ms: u64,
    started_ms: Option<u64>,
    finished_ms: Option<u64>,
    active: Duration,
    running_since: Option<Instant>,
    /// has a change that must reach the ui without delay
    urgent: bool,
    /// has a change that can wait for the next progress tick
    dirty: bool,
    last_emit: Option<Instant>,
}

impl Entry {
    fn new(spec: JobSpec, job: Box<dyn Job>) -> Self {
        Self {
            spec,
            state: JobState::Queued,
            job: Some(job),
            control: Arc::default(),
            parked: false,
            holds_slot: false,
            progress: None,
            message: None,
            eta: EtaEstimator::default(),
            attempt: 0,
            retry_at: None,
            error: None,
            output: None,
            created_ms: unix_ms(),
            started_ms: None,
            finished_ms: None,
            active: Duration::ZERO,
            running_since: None,
            urgent: true,
            dirty: false,
            last_emit: None,
        }
    }

    fn active_time(&self, now: Instant) -> Duration {
        let current = self
            .running_since
            .map_or(Duration::ZERO, |since| now.duration_since(since));
        self.active + current
    }

    fn stop_clock(&mut self, now: Instant) {
        if let Some(since) = self.running_since.take() {
            self.active += now.duration_since(since);
        }
    }

    fn snapshot(&self, id: JobId, now: Instant) -> JobSnapshot {
        let active = self.active_time(now);
        let eta = match self.state {
            JobState::Running | JobState::Pausing => self.eta.estimate(active),
            _ => None,
        };
        JobSnapshot {
            id,
            name: self.spec.name.clone(),
            kind: self.spec.kind,
            category: self.spec.category.clone(),
            background: self.spec.background,
            priority: self.spec.priority,
            state: self.state,
            progress: self.progress,
            message: self.message.clone(),
            eta_ms: eta.map(|eta| eta.as_millis() as u64),
            elapsed_ms: active.as_millis() as u64,
            attempt: self.attempt,
            max_attempts: self.spec.retry.max_attempts,
            can_retry: self.job.is_some()
                && matches!(self.state, JobState::Failed | JobState::Cancelled),
            error: self.error.clone(),
            output: self.output.clone(),
            created_ms: self.created_ms,
            started_ms: self.started_ms,
            finished_ms: self.finished_ms,
        }
    }
}

#[derive(Default)]
struct Slots {
    cpu: usize,
    gpu: usize,
    io: usize,
}

impl Slots {
    fn of(&mut self, kind: JobKind) -> &mut usize {
        match kind {
            JobKind::Cpu => &mut self.cpu,
            JobKind::Gpu => &mut self.gpu,
            JobKind::Io => &mut self.io,
        }
    }
}

struct State {
    jobs: BTreeMap<JobId, Entry>,
    /// queued and paused jobs in the order they will get a slot
    pending: Vec<JobId>,
    /// finished jobs, oldest first
    finished: VecDeque<JobId>,
    /// jobs with a change the ui has not heard about yet
    changed: BTreeSet<JobId>,
    /// events waiting for the dispatcher thread to publish them
    outbox: Vec<JobEvent>,
    limits: JobLimits,
    in_use: Slots,
    next_id: u64,
    shutdown: bool,
}

impl State {
    fn entry(&mut self, id: JobId) -> Result<&mut Entry, JobQueueError> {
        self.jobs.get_mut(&id).ok_or(JobQueueError::NotFound(id))
    }

    fn mark(&mut self, id: JobId) {
        if let Some(entry) = self.jobs.get_mut(&id) {
            entry.urgent = true;
            self.changed.insert(id);
        }
    }

    /// puts a job in line behind everything of the same or higher priority,
    /// or ahead of its own priority level when `first` is set
    fn enqueue(&mut self, id: JobId, first: bool) {
        let Some(priority) = self.jobs.get(&id).map(|entry| entry.spec.priority) else {
            return;
        };
        let position = self
            .pending
            .iter()
            .position(|other| {
                let other = self
                    .jobs
                    .get(other)
                    .map_or(Priority::Low, |e| e.spec.priority);
                if first {
                    other <= priority
                } else {
                    other < priority
                }
            })
            .unwrap_or(self.pending.len());
        self.pending.insert(position, id);
        if position + 1 != self.pending.len() {
            self.announce_order();
        }
    }

    fn dequeue(&mut self, id: JobId) {
        self.pending.retain(|other| *other != id);
    }

    fn announce_order(&mut self) {
        self.outbox.push(JobEvent::Reordered {
            pending: self.pending.clone(),
        });
    }

    fn release_slot(&mut self, id: JobId) {
        if let Some(entry) = self.jobs.get_mut(&id)
            && std::mem::take(&mut entry.holds_slot)
        {
            let slot = self.in_use.of(entry.spec.kind);
            *slot = slot.saturating_sub(1);
        }
    }

    /// files a job under finished and forgets the oldest ones past the cap
    fn retire(&mut self, id: JobId, state: JobState, keep_finished: usize) {
        if let Some(entry) = self.jobs.get_mut(&id) {
            entry.state = state;
            entry.finished_ms = Some(unix_ms());
            entry.retry_at = None;
        }
        self.finished.push_back(id);
        self.mark(id);
        while self.finished.len() > keep_finished {
            if let Some(oldest) = self.finished.pop_front() {
                self.forget(oldest);
            }
        }
    }

    fn forget(&mut self, id: JobId) {
        if self.jobs.remove(&id).is_some() {
            self.changed.remove(&id);
            self.outbox.push(JobEvent::Removed { id });
        }
    }
}

struct Shared {
    state: Mutex<State>,
    /// wakes the dispatcher thread
    poke: Condvar,
    /// wakes threads blocked in `JobHandle::wait`
    done: Condvar,
    bus: EventBus,
    progress_interval: Duration,
    keep_finished: usize,
}

impl Shared {
    fn submit(&self, spec: JobSpec, job: Box<dyn Job>) -> JobId {
        let mut state = self.state.lock();
        let id = JobId(state.next_id);
        state.next_id += 1;
        state.jobs.insert(id, Entry::new(spec, job));
        if state.shutdown {
            state.retire(id, JobState::Cancelled, self.keep_finished);
        } else {
            state.enqueue(id, false);
            state.mark(id);
        }
        self.poke.notify_one();
        id
    }

    fn pause(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut state = self.state.lock();
        let entry = state.entry(id)?;
        match entry.state {
            JobState::Queued => entry.state = JobState::Paused,
            JobState::Running => {
                entry.state = JobState::Pausing;
                entry.control.set(PAUSE);
                entry.control.wake.notify_all();
            }
            JobState::Pausing | JobState::Paused => return Ok(()),
            state => {
                return Err(JobQueueError::NotAllowed {
                    id,
                    action: "be paused",
                    state,
                });
            }
        }
        state.mark(id);
        self.poke.notify_one();
        Ok(())
    }

    fn resume(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut state = self.state.lock();
        let entry = state.entry(id)?;
        match entry.state {
            // it never reached a checkpoint, so it simply carries on
            JobState::Pausing => entry.state = JobState::Running,
            JobState::Paused => entry.state = JobState::Queued,
            JobState::Queued | JobState::Running => return Ok(()),
            state => {
                return Err(JobQueueError::NotAllowed {
                    id,
                    action: "be resumed",
                    state,
                });
            }
        }
        entry.control.clear(PAUSE);
        state.mark(id);
        self.poke.notify_one();
        Ok(())
    }

    fn cancel(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut state = self.state.lock();
        self.cancel_locked(&mut state, id)?;
        self.poke.notify_one();
        self.done.notify_all();
        Ok(())
    }

    fn cancel_locked(&self, state: &mut State, id: JobId) -> Result<(), JobQueueError> {
        let entry = state.entry(id)?;
        match entry.state {
            JobState::Queued | JobState::Paused if !entry.parked => {
                state.dequeue(id);
                state.retire(id, JobState::Cancelled, self.keep_finished);
            }
            JobState::Queued | JobState::Paused | JobState::Running | JobState::Pausing => {
                // its thread has to notice and unwind, `finish` does the rest
                entry.state = JobState::Cancelling;
                entry.control.set(CANCEL);
                entry.control.wake.notify_all();
                state.dequeue(id);
                state.mark(id);
            }
            JobState::Cancelling => {}
            state => {
                return Err(JobQueueError::NotAllowed {
                    id,
                    action: "be cancelled",
                    state,
                });
            }
        }
        Ok(())
    }

    fn retry(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut state = self.state.lock();
        let entry = state.entry(id)?;
        let retryable = matches!(entry.state, JobState::Failed | JobState::Cancelled);
        if !retryable || entry.job.is_none() {
            let state = entry.state;
            return Err(JobQueueError::NotAllowed {
                id,
                action: "be retried",
                state,
            });
        }
        entry.state = JobState::Queued;
        entry.attempt = 0;
        entry.error = None;
        entry.output = None;
        entry.progress = None;
        entry.message = None;
        entry.eta.reset();
        entry.active = Duration::ZERO;
        entry.started_ms = None;
        entry.finished_ms = None;
        entry.control.flags.store(0, Ordering::Release);

        state.finished.retain(|other| *other != id);
        state.enqueue(id, false);
        state.mark(id);
        self.poke.notify_one();
        Ok(())
    }

    fn remove(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut state = self.state.lock();
        let current = state.entry(id)?.state;
        if !current.is_finished() {
            return Err(JobQueueError::NotAllowed {
                id,
                action: "be removed",
                state: current,
            });
        }
        state.finished.retain(|other| *other != id);
        state.forget(id);
        self.poke.notify_one();
        Ok(())
    }

    fn set_priority(&self, id: JobId, priority: Priority) -> Result<(), JobQueueError> {
        let mut state = self.state.lock();
        let entry = state.entry(id)?;
        if entry.spec.priority == priority {
            return Ok(());
        }
        entry.spec.priority = priority;
        if state.pending.contains(&id) {
            state.dequeue(id);
            state.enqueue(id, false);
            // enqueue stays quiet when the job lands at the end, but it still moved
            if state.pending.last() == Some(&id) {
                state.announce_order();
            }
        }
        state.mark(id);
        self.poke.notify_one();
        Ok(())
    }

    fn move_to(&self, id: JobId, index: usize) -> Result<(), JobQueueError> {
        let mut state = self.state.lock();
        let current = state.entry(id)?.state;
        let Some(from) = state.pending.iter().position(|other| *other == id) else {
            return Err(JobQueueError::NotAllowed {
                id,
                action: "be reordered",
                state: current,
            });
        };
        let to = index.min(state.pending.len() - 1);
        if from != to {
            state.pending.remove(from);
            state.pending.insert(to, id);
            state.announce_order();
            self.poke.notify_one();
        }
        Ok(())
    }

    fn shut_down(&self) {
        let mut state = self.state.lock();
        if std::mem::replace(&mut state.shutdown, true) {
            return;
        }
        let live: Vec<JobId> = state
            .jobs
            .iter()
            .filter(|(_, entry)| !entry.state.is_finished())
            .map(|(id, _)| *id)
            .collect();
        for id in live {
            // only finished jobs are refused, and those were filtered out
            let _ = self.cancel_locked(&mut state, id);
        }
        self.poke.notify_all();
        self.done.notify_all();
    }

    /// the dispatcher thread: starts jobs when slots free up, wakes up for
    /// retries that are due, and publishes events. doing all publishing from
    /// this one thread keeps events in order and means a handler can call
    /// straight back into the queue
    fn dispatch(self: Arc<Self>) {
        let mut state = self.state.lock();
        loop {
            let now = Instant::now();
            let retry_due = self.start_ready(&mut state, now);
            let flush_due = self.collect_updates(&mut state, now);

            if !state.outbox.is_empty() {
                let events = std::mem::take(&mut state.outbox);
                MutexGuard::unlocked(&mut state, || {
                    for event in events {
                        self.bus.publish(event);
                    }
                });
                // things may have changed while the lock was released
                continue;
            }
            if state.shutdown {
                return;
            }
            match [retry_due, flush_due].into_iter().flatten().min() {
                Some(deadline) => {
                    self.poke.wait_until(&mut state, deadline);
                }
                None => self.poke.wait(&mut state),
            }
        }
    }

    /// hands free slots to waiting jobs in queue order. returns when the next
    /// delayed retry becomes due, if there is one
    fn start_ready(self: &Arc<Self>, state: &mut State, now: Instant) -> Option<Instant> {
        let mut next_retry: Option<Instant> = None;
        let mut index = 0;
        while index < state.pending.len() {
            let id = state.pending[index];
            index += 1;
            let Some(entry) = state.jobs.get_mut(&id) else {
                continue;
            };
            if entry.state != JobState::Queued {
                continue;
            }
            if let Some(at) = entry.retry_at.filter(|at| *at > now) {
                next_retry = Some(next_retry.map_or(at, |next| next.min(at)));
                continue;
            }
            let kind = entry.spec.kind;
            if *state.in_use.of(kind) >= state.limits.of(kind) {
                continue;
            }

            index -= 1;
            state.pending.remove(index);
            *state.in_use.of(kind) += 1;
            entry.holds_slot = true;
            entry.retry_at = None;
            entry.state = JobState::Running;
            entry.running_since = Some(now);

            if entry.parked {
                entry.control.wake.notify_all();
            } else if let Some(job) = entry.job.take() {
                entry.attempt += 1;
                entry.error = None;
                entry.started_ms.get_or_insert_with(unix_ms);
                let control = entry.control.clone();
                let attempt = entry.attempt;
                if let Err(error) = self.spawn(id, job, control, attempt) {
                    self.fail_to_start(state, id, error);
                    continue;
                }
            }
            state.mark(id);
        }
        next_retry
    }

    fn spawn(
        self: &Arc<Self>,
        id: JobId,
        mut job: Box<dyn Job>,
        control: Arc<Control>,
        attempt: u32,
    ) -> io::Result<()> {
        let shared = self.clone();
        thread::Builder::new()
            .name(format!("vellum-job-{}", id.0))
            .spawn(move || {
                let ctx = JobContext {
                    shared: shared.clone(),
                    id,
                    control,
                    attempt,
                };
                let result = panic::catch_unwind(AssertUnwindSafe(|| job.run(&ctx)));
                shared.finish(id, job, result);
            })?;
        Ok(())
    }

    fn fail_to_start(&self, state: &mut State, id: JobId, error: io::Error) {
        state.release_slot(id);
        if let Some(entry) = state.jobs.get_mut(&id) {
            entry.running_since = None;
            entry.error = Some(format!("Could not start a thread for this job: {error}"));
        }
        state.retire(id, JobState::Failed, self.keep_finished);
        self.done.notify_all();
    }

    /// turns pending changes into `Updated` events. urgent ones go now, progress
    /// waits until the job's last event is old enough. returns when the next
    /// held back update becomes due
    fn collect_updates(&self, state: &mut State, now: Instant) -> Option<Instant> {
        let mut next_flush: Option<Instant> = None;
        let State {
            jobs,
            changed,
            outbox,
            ..
        } = state;
        changed.retain(|id| {
            let Some(entry) = jobs.get_mut(id) else {
                return false;
            };
            let due = entry.last_emit.map(|last| last + self.progress_interval);
            if let Some(due) = due.filter(|due| !entry.urgent && *due > now) {
                next_flush = Some(next_flush.map_or(due, |next| next.min(due)));
                return true;
            }
            entry.urgent = false;
            entry.dirty = false;
            entry.last_emit = Some(now);
            outbox.push(JobEvent::Updated {
                job: entry.snapshot(*id, now),
            });
            false
        });
        next_flush
    }

    /// runs on the job's own thread once `run` has returned or panicked
    fn finish(&self, id: JobId, job: Box<dyn Job>, result: thread::Result<JobResult>) {
        let now = Instant::now();
        // a job can own files and buffers, so it is dropped after the lock is released
        let mut spent = None;
        {
            let mut state = self.state.lock();
            state.release_slot(id);
            let shutdown = state.shutdown;
            let Some(entry) = state.jobs.get_mut(&id) else {
                return;
            };
            entry.stop_clock(now);
            entry.parked = false;
            let cancelled = entry.control.has(CANCEL);
            entry.control.flags.store(0, Ordering::Release);

            let outcome = match result {
                Ok(Ok(output)) => {
                    entry.output = Some(output);
                    entry.progress = Some(1.0);
                    entry.error = None;
                    spent = Some(job);
                    JobState::Succeeded
                }
                Ok(Err(JobError::Cancelled)) => {
                    entry.job = Some(job);
                    JobState::Cancelled
                }
                // an error that surfaces while cancelling is the cancel taking effect
                Ok(Err(JobError::Failed { .. })) if cancelled => {
                    entry.job = Some(job);
                    JobState::Cancelled
                }
                Ok(Err(JobError::Failed { message, retryable })) => {
                    let tries_left = entry.attempt < entry.spec.retry.max_attempts;
                    entry.job = Some(job);
                    entry.error = Some(message);
                    if retryable && tries_left && !shutdown {
                        entry.retry_at = Some(now + entry.spec.retry.delay_after(entry.attempt));
                        entry.progress = None;
                        entry.eta.reset();
                        JobState::Queued
                    } else {
                        JobState::Failed
                    }
                }
                Err(panic) => {
                    // whatever state the job is in after a panic, it is not safe to run again
                    spent = Some(job);
                    entry.error = Some(format!("This job crashed: {}", panic_message(&*panic)));
                    JobState::Failed
                }
            };

            if outcome == JobState::Queued {
                entry.state = outcome;
                state.enqueue(id, false);
                state.mark(id);
            } else {
                state.retire(id, outcome, self.keep_finished);
            }
            self.done.notify_all();
            self.poke.notify_one();
        }
        drop(spent);
    }

    fn snapshot(&self, id: JobId) -> Option<JobSnapshot> {
        let state = self.state.lock();
        state
            .jobs
            .get(&id)
            .map(|entry| entry.snapshot(id, Instant::now()))
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "no details available"
    }
}

/// stops the dispatcher and cancels what is left once the last `JobQueue` clone goes away
struct Owner {
    shared: Arc<Shared>,
    dispatcher: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.shared.shut_down();
        if let Some(dispatcher) = self.dispatcher.lock().take() {
            // an event handler can hold the last clone, and a thread cannot join itself
            if dispatcher.thread().id() != thread::current().id() {
                let _ = dispatcher.join();
            }
        }
    }
}

#[derive(Clone)]
pub struct JobQueue {
    owner: Arc<Owner>,
}

impl JobQueue {
    pub fn new(bus: EventBus, config: JobQueueConfig) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                jobs: BTreeMap::new(),
                pending: Vec::new(),
                finished: VecDeque::new(),
                changed: BTreeSet::new(),
                outbox: Vec::new(),
                limits: config.limits,
                in_use: Slots::default(),
                next_id: 1,
                shutdown: false,
            }),
            poke: Condvar::new(),
            done: Condvar::new(),
            bus,
            progress_interval: config.progress_interval,
            keep_finished: config.keep_finished,
        });
        let dispatcher = thread::Builder::new()
            .name("vellum-jobs".to_owned())
            .spawn({
                let shared = shared.clone();
                move || shared.dispatch()
            })?;
        Ok(Self {
            owner: Arc::new(Owner {
                shared,
                dispatcher: Mutex::new(Some(dispatcher)),
            }),
        })
    }

    fn shared(&self) -> &Arc<Shared> {
        &self.owner.shared
    }

    /// queues a job. it starts as soon as a slot of its kind is free
    pub fn submit(&self, spec: JobSpec, job: impl Job) -> JobHandle {
        let id = self.shared().submit(spec, Box::new(job));
        JobHandle {
            shared: self.shared().clone(),
            id,
        }
    }

    /// a handle to a job that is still known to the queue
    pub fn handle(&self, id: JobId) -> Option<JobHandle> {
        let known = self.shared().state.lock().jobs.contains_key(&id);
        known.then(|| JobHandle {
            shared: self.shared().clone(),
            id,
        })
    }

    /// a running job pauses at its next checkpoint and gives up its slot.
    /// a queued one just will not start
    pub fn pause(&self, id: JobId) -> Result<(), JobQueueError> {
        self.shared().pause(id)
    }

    /// a job that was paused mid run gets back in line ahead of others of its priority
    pub fn resume(&self, id: JobId) -> Result<(), JobQueueError> {
        self.shared().resume(id)
    }

    pub fn cancel(&self, id: JobId) -> Result<(), JobQueueError> {
        self.shared().cancel(id)
    }

    /// runs a failed or cancelled job again from the start
    pub fn retry(&self, id: JobId) -> Result<(), JobQueueError> {
        self.shared().retry(id)
    }

    /// drops a finished job from the list
    pub fn remove(&self, id: JobId) -> Result<(), JobQueueError> {
        self.shared().remove(id)
    }

    pub fn clear_finished(&self) {
        let mut state = self.shared().state.lock();
        for id in std::mem::take(&mut state.finished) {
            state.forget(id);
        }
        self.shared().poke.notify_one();
    }

    pub fn pause_all(&self) {
        for id in self.live_jobs() {
            let _ = self.pause(id);
        }
    }

    pub fn resume_all(&self) {
        for id in self.live_jobs() {
            let _ = self.resume(id);
        }
    }

    fn live_jobs(&self) -> Vec<JobId> {
        let state = self.shared().state.lock();
        state
            .jobs
            .iter()
            .filter(|(_, entry)| !entry.state.is_finished())
            .map(|(id, _)| *id)
            .collect()
    }

    /// changes a job's priority, which also moves it in the queue if it is waiting
    pub fn set_priority(&self, id: JobId, priority: Priority) -> Result<(), JobQueueError> {
        self.shared().set_priority(id, priority)
    }

    /// moves a waiting job to a position in the queue, 0 being next to run.
    /// a manual move wins over priorities
    pub fn move_to(&self, id: JobId, index: usize) -> Result<(), JobQueueError> {
        self.shared().move_to(id, index)
    }

    pub fn limits(&self) -> JobLimits {
        self.shared().state.lock().limits
    }

    /// takes effect for jobs that have not started. running jobs are never interrupted
    pub fn set_limits(&self, limits: JobLimits) {
        self.shared().state.lock().limits = limits;
        self.shared().poke.notify_one();
    }

    pub fn snapshot(&self, id: JobId) -> Option<JobSnapshot> {
        self.shared().snapshot(id)
    }

    pub fn overview(&self) -> JobsOverview {
        let now = Instant::now();
        let state = self.shared().state.lock();
        JobsOverview {
            jobs: state
                .jobs
                .iter()
                .map(|(id, entry)| entry.snapshot(*id, now))
                .collect(),
            pending: state.pending.clone(),
        }
    }

    /// cancels everything and stops accepting work. also happens when the last clone is dropped
    pub fn shut_down(&self) {
        self.shared().shut_down();
    }
}

/// a submitted job, for the code that started it
#[derive(Clone)]
pub struct JobHandle {
    shared: Arc<Shared>,
    id: JobId,
}

impl JobHandle {
    pub fn id(&self) -> JobId {
        self.id
    }

    pub fn snapshot(&self) -> Option<JobSnapshot> {
        self.shared.snapshot(self.id)
    }

    pub fn cancel(&self) -> Result<(), JobQueueError> {
        self.shared.cancel(self.id)
    }

    pub fn pause(&self) -> Result<(), JobQueueError> {
        self.shared.pause(self.id)
    }

    pub fn resume(&self) -> Result<(), JobQueueError> {
        self.shared.resume(self.id)
    }

    /// blocks until the job has succeeded, failed for good or been cancelled.
    /// never call this from the ui thread
    pub fn wait(&self) -> Result<JobSnapshot, JobQueueError> {
        let mut state = self.shared.state.lock();
        loop {
            let entry = state
                .jobs
                .get(&self.id)
                .ok_or(JobQueueError::NotFound(self.id))?;
            if entry.state.is_finished() {
                return Ok(entry.snapshot(self.id, Instant::now()));
            }
            self.shared.done.wait(&mut state);
        }
    }

    /// like `wait`, but gives up after `timeout` and returns none
    pub fn wait_timeout(&self, timeout: Duration) -> Result<Option<JobSnapshot>, JobQueueError> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        loop {
            let entry = state
                .jobs
                .get(&self.id)
                .ok_or(JobQueueError::NotFound(self.id))?;
            if entry.state.is_finished() {
                return Ok(Some(entry.snapshot(self.id, Instant::now())));
            }
            if self
                .shared
                .done
                .wait_until(&mut state, deadline)
                .timed_out()
            {
                return Ok(None);
            }
        }
    }
}

/// lets code outside the job's own thread (worker pools, decoder callbacks)
/// see that the job was cancelled
#[derive(Clone)]
pub struct CancelToken {
    control: Arc<Control>,
}

impl CancelToken {
    pub fn is_cancelled(&self) -> bool {
        self.control.has(CANCEL)
    }
}

/// what a running job uses to report back and to cooperate with cancel and pause
pub struct JobContext {
    shared: Arc<Shared>,
    id: JobId,
    control: Arc<Control>,
    attempt: u32,
}

impl JobContext {
    pub fn id(&self) -> JobId {
        self.id
    }

    /// 1 on the first run, 2 on the first retry, and so on
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// call this often, at least a few times a second. it returns an error once
    /// the job is cancelled, which the job should pass straight up with `?`,
    /// and it blocks while the job is paused. when nothing is going on it costs
    /// one atomic load
    pub fn checkpoint(&self) -> Result<(), JobError> {
        if self.control.flags.load(Ordering::Acquire) == 0 {
            return Ok(());
        }
        self.checkpoint_slow()
    }

    #[cold]
    fn checkpoint_slow(&self) -> Result<(), JobError> {
        let mut state = self.shared.state.lock();
        loop {
            if self.control.has(CANCEL) {
                return Err(JobError::Cancelled);
            }
            let current = state.jobs.get(&self.id).map(|entry| entry.state);
            match current {
                Some(JobState::Pausing) => self.park(&mut state),
                Some(JobState::Paused | JobState::Queued) => {}
                // running again, or the queue no longer knows this job
                _ => {
                    if let Some(entry) = state.jobs.get_mut(&self.id) {
                        entry.parked = false;
                    }
                    return Ok(());
                }
            }
            self.control.wake.wait(&mut state);
        }
    }

    /// gives the slot back and gets in line again, at the front of this job's
    /// priority level so it continues before fresh work starts
    fn park(&self, state: &mut State) {
        let now = Instant::now();
        state.release_slot(self.id);
        if let Some(entry) = state.jobs.get_mut(&self.id) {
            entry.state = JobState::Paused;
            entry.parked = true;
            entry.stop_clock(now);
        }
        state.enqueue(self.id, true);
        state.mark(self.id);
        self.shared.poke.notify_one();
    }

    pub fn is_cancelled(&self) -> bool {
        self.control.has(CANCEL)
    }

    pub fn cancel_token(&self) -> CancelToken {
        CancelToken {
            control: self.control.clone(),
        }
    }

    /// waits without holding up a cancel or a pause
    pub fn sleep(&self, duration: Duration) -> Result<(), JobError> {
        let deadline = Instant::now() + duration;
        loop {
            self.checkpoint()?;
            let mut state = self.shared.state.lock();
            // cancel and pause set their flag under this lock, so none can slip past here
            if self.control.flags.load(Ordering::Acquire) != 0 {
                continue;
            }
            if self
                .control
                .wake
                .wait_until(&mut state, deadline)
                .timed_out()
            {
                return Ok(());
            }
        }
    }

    /// reports how far along the job is, from 0 to 1
    pub fn progress(&self, fraction: f32) {
        if !fraction.is_finite() {
            return;
        }
        let fraction = fraction.clamp(0.0, 1.0);
        self.report(|entry, active| {
            if entry.progress == Some(fraction) {
                return false;
            }
            entry.progress = Some(fraction);
            entry.eta.record(active, fraction);
            true
        });
    }

    /// for stretches where the job cannot tell how long it will take
    pub fn indeterminate(&self) {
        self.report(|entry, _| {
            entry.eta.reset();
            entry.progress.take().is_some()
        });
    }

    /// a short note on what is happening, such as "Pass 1 of 2"
    pub fn message(&self, message: impl Into<String>) {
        let message = Some(message.into());
        self.report(|entry, _| {
            if entry.message == message {
                return false;
            }
            entry.message = message;
            true
        });
    }

    fn report(&self, change: impl FnOnce(&mut Entry, Duration) -> bool) {
        let now = Instant::now();
        let mut state = self.shared.state.lock();
        let Some(entry) = state.jobs.get_mut(&self.id) else {
            return;
        };
        let active = entry.active_time(now);
        if !change(entry, active) || std::mem::replace(&mut entry.dirty, true) {
            return;
        }
        state.changed.insert(self.id);
        // the dispatcher only needs a nudge when this is the first unsent change
        self.shared.poke.notify_one();
    }
}
