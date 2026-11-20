//! the scheduler behind [`JobQueue`].
//!
//! all bookkeeping lives in one `State` behind one mutex, with one condvar that is notified
//! after every change. there is no scheduler thread: whoever changes the state calls `pump`,
//! which starts as many waiting jobs as the limits allow, each on its own named thread.
//! media jobs run for seconds to hours, so a thread per job costs nothing next to the work,
//! and it keeps a blocking ffmpeg call from starving other jobs the way a shared pool would.
//!
//! events are collected while the lock is held and sent after it is released, so a slow
//! listener can never stall the queue.

use std::collections::BTreeMap;
use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::{Condvar, Mutex, MutexGuard};

use super::eta::EtaEstimator;
use super::{
    JobError, JobEvent, JobId, JobLimits, JobOutput, JobPriority, JobQueueError, JobResource,
    JobSnapshot, JobSpec, JobState, JobTask, JobsOverview,
};
use crate::events::EventBus;

/// progress only updates are sent to the ui at most this often per job
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// longest automatic retry wait, however many retries
const MAX_RETRY_DELAY: Duration = Duration::from_secs(300);

/// flags a running job reads without taking the lock
#[derive(Debug, Default)]
struct Control {
    cancel: AtomicBool,
    pause: AtomicBool,
}

struct Job {
    spec: JobSpec,
    state: JobState,
    /// none while a worker thread owns it, and after the job completed
    task: Option<Box<dyn JobTask>>,
    control: Arc<Control>,
    /// a worker thread exists for this job, possibly parked in a checkpoint
    has_thread: bool,
    /// counts against the running limit of its resource
    holds_slot: bool,
    /// the thread is waiting in `waiting` for a slot after a pause
    parked: bool,
    /// `pump` gave a parked thread its slot back and it has not woken yet
    slot_granted: bool,
    progress: Option<f32>,
    message: Option<String>,
    error: Option<String>,
    result: Option<JobOutput>,
    attempt: u32,
    retry_at: Option<Instant>,
    eta: EtaEstimator,
    created_at_ms: u64,
    last_emit: Option<Instant>,
}

impl Job {
    fn snapshot(&self, id: JobId, revision: u64, now: Instant) -> JobSnapshot {
        let eta = match self.state {
            JobState::Running | JobState::Pausing => {
                self.eta.eta(now, self.progress).map(|d| d.as_secs_f64())
            }
            _ => None,
        };
        JobSnapshot {
            id,
            title: self.spec.title.clone(),
            resource: self.spec.resource,
            priority: self.spec.priority,
            state: self.state,
            background: self.spec.background,
            progress: self.progress,
            message: self.message.clone(),
            eta_secs: eta,
            elapsed_secs: self.eta.elapsed(now).as_secs_f64(),
            attempt: self.attempt,
            max_retries: self.spec.max_retries,
            error: self.error.clone(),
            result: self.result.clone(),
            created_at_ms: self.created_at_ms,
            revision,
        }
    }
}

struct State {
    jobs: BTreeMap<JobId, Job>,
    /// jobs waiting for a slot, in start order. new jobs go after everything of the same or
    /// higher priority, and the user can reorder freely.
    waiting: Vec<JobId>,
    running: [usize; 3],
    limits: JobLimits,
    next_id: u64,
    revision: u64,
    outbox: Vec<JobEvent>,
    /// retry timers to start once the lock is released
    wakeups: Vec<Duration>,
    order_dirty: bool,
}

impl State {
    fn job(&self, id: JobId) -> Result<&Job, JobQueueError> {
        self.jobs.get(&id).ok_or(JobQueueError::UnknownJob(id))
    }

    fn job_mut(&mut self, id: JobId) -> Result<&mut Job, JobQueueError> {
        self.jobs.get_mut(&id).ok_or(JobQueueError::UnknownJob(id))
    }

    /// queues an update event for one job
    fn touch(&mut self, id: JobId) {
        let now = Instant::now();
        self.revision += 1;
        let revision = self.revision;
        if let Some(job) = self.jobs.get_mut(&id) {
            job.last_emit = Some(now);
            let job = job.snapshot(id, revision, now);
            self.outbox.push(JobEvent::Updated { job });
        }
    }

    fn insert_waiting(&mut self, id: JobId, ahead_of_equals: bool) {
        let Some(priority) = self.jobs.get(&id).map(|job| job.spec.priority) else {
            return;
        };
        let jobs = &self.jobs;
        let goes_before = |other: &JobId| {
            let other = jobs
                .get(other)
                .map_or(JobPriority::Low, |j| j.spec.priority);
            if ahead_of_equals {
                other <= priority
            } else {
                other < priority
            }
        };
        let index = self
            .waiting
            .iter()
            .position(goes_before)
            .unwrap_or(self.waiting.len());
        self.waiting.insert(index, id);
        self.order_dirty = true;
    }

    fn remove_waiting(&mut self, id: JobId) {
        let before = self.waiting.len();
        self.waiting.retain(|other| *other != id);
        if self.waiting.len() != before {
            self.order_dirty = true;
        }
    }

    fn release_slot(&mut self, id: JobId) {
        if let Some(job) = self.jobs.get_mut(&id)
            && job.holds_slot
        {
            job.holds_slot = false;
            job.slot_granted = false;
            let slot = &mut self.running[job.spec.resource.index()];
            *slot = slot.saturating_sub(1);
        }
    }

    /// starts or wakes as many waiting jobs as the limits allow
    fn pump(&mut self, shared: &Arc<Shared>) {
        let now = Instant::now();
        let mut index = 0;
        while index < self.waiting.len() {
            let id = self.waiting[index];
            let Some(job) = self.jobs.get(&id) else {
                self.waiting.remove(index);
                continue;
            };
            let resource = job.spec.resource.index();
            let not_yet = job.retry_at.is_some_and(|at| at > now);
            if not_yet || self.running[resource] >= self.limits.get(job.spec.resource) {
                index += 1;
                continue;
            }
            self.waiting.remove(index);
            self.order_dirty = true;
            self.running[resource] += 1;

            let Some(job) = self.jobs.get_mut(&id) else {
                continue;
            };
            job.holds_slot = true;
            job.retry_at = None;
            if job.parked {
                // the parked thread notices on the next notify and carries on
                job.slot_granted = true;
                continue;
            }
            let Some(task) = job.task.take() else {
                // nothing to run, should not happen but must not wedge the slot
                job.state = JobState::Failed;
                job.error = Some("the job had nothing to run".into());
                self.release_slot(id);
                self.touch(id);
                continue;
            };
            job.state = JobState::Running;
            job.has_thread = true;
            job.eta.start(now);
            let control = job.control.clone();
            let attempt = job.attempt;
            self.touch(id);
            self.spawn_worker(shared, id, task, control, attempt);
        }
    }

    fn spawn_worker(
        &mut self,
        shared: &Arc<Shared>,
        id: JobId,
        task: Box<dyn JobTask>,
        control: Arc<Control>,
        attempt: u32,
    ) {
        // the task sits in a shared cell so it can be put back if the thread fails to start
        let cell = Arc::new(Mutex::new(Some(task)));
        let worker_cell = cell.clone();
        let worker_shared = shared.clone();
        let spawned = thread::Builder::new()
            .name(format!("vellum-job-{}", id.0))
            .spawn(move || {
                if let Some(task) = worker_cell.lock().take() {
                    run_worker(worker_shared, id, task, control, attempt);
                }
            });
        if let Err(err) = spawned {
            tracing::error!(%id, %err, "could not start a worker thread");
            let task = cell.lock().take();
            self.release_slot(id);
            if let Some(job) = self.jobs.get_mut(&id) {
                job.task = task;
                job.has_thread = false;
                job.state = JobState::Failed;
                job.error = Some(format!("could not start the job: {err}"));
            }
            self.touch(id);
        }
    }

    /// records how a worker's run ended
    fn finish(
        &mut self,
        id: JobId,
        task: Box<dyn JobTask>,
        result: Result<JobOutput, JobError>,
        control: &Control,
    ) {
        self.release_slot(id);
        let now = Instant::now();
        let Some(job) = self.jobs.get_mut(&id) else {
            return;
        };
        job.has_thread = false;
        job.parked = false;
        job.eta.stop(now);
        let mut requeue = false;
        match result {
            Ok(output) => {
                job.state = JobState::Completed;
                job.progress = Some(1.0);
                job.result = Some(output);
                job.error = None;
            }
            Err(JobError::Cancelled) => {
                job.state = JobState::Cancelled;
                job.task = Some(task);
            }
            Err(JobError::Failed { message, retryable }) => {
                job.task = Some(task);
                job.error = Some(message);
                let cancelled = control.cancel.load(Ordering::SeqCst);
                if retryable && !cancelled && job.attempt < job.spec.max_retries {
                    job.attempt += 1;
                    job.progress = None;
                    job.eta.reset();
                    if control.pause.swap(false, Ordering::SeqCst) {
                        // the user paused it mid run, so wait for them before retrying
                        job.state = JobState::Paused;
                    } else {
                        let doublings = (job.attempt - 1).min(16);
                        let delay = job
                            .spec
                            .retry_delay
                            .saturating_mul(1 << doublings)
                            .min(MAX_RETRY_DELAY);
                        job.state = JobState::WaitingToRetry;
                        job.retry_at = Some(now + delay);
                        self.wakeups.push(delay);
                        requeue = true;
                    }
                } else if cancelled {
                    job.state = JobState::Cancelled;
                } else {
                    job.state = JobState::Failed;
                }
            }
        }
        let drop_now = job.spec.background && job.state.is_finished();
        if requeue {
            self.insert_waiting(id, true);
        }
        self.touch(id);
        if drop_now {
            // listeners still get the final snapshot (with the result) before it goes
            let _ = self.remove(id);
        }
    }

    fn cancel(&mut self, id: JobId) -> Result<(), JobQueueError> {
        let job = self.job_mut(id)?;
        match job.state {
            JobState::Cancelling | JobState::Cancelled => return Ok(()),
            state if state.is_finished() => {
                return Err(JobQueueError::NotAllowed {
                    id,
                    action: "cancel",
                    state,
                });
            }
            _ => {}
        }
        if job.has_thread {
            job.control.cancel.store(true, Ordering::SeqCst);
            job.state = JobState::Cancelling;
        } else {
            job.state = JobState::Cancelled;
            job.retry_at = None;
            self.remove_waiting(id);
        }
        self.touch(id);
        Ok(())
    }

    fn pause(&mut self, id: JobId) -> Result<(), JobQueueError> {
        let job = self.job_mut(id)?;
        let state = job.state;
        match (state, job.has_thread) {
            (JobState::Pausing | JobState::Paused, _) => return Ok(()),
            (JobState::Running | JobState::Queued, true) => {
                job.control.pause.store(true, Ordering::SeqCst);
                job.state = JobState::Pausing;
            }
            (JobState::Queued | JobState::WaitingToRetry, false) => {
                job.state = JobState::Paused;
                self.remove_waiting(id);
            }
            _ => {
                return Err(JobQueueError::NotAllowed {
                    id,
                    action: "pause",
                    state,
                });
            }
        }
        self.touch(id);
        Ok(())
    }

    fn resume(&mut self, id: JobId) -> Result<(), JobQueueError> {
        let job = self.job_mut(id)?;
        let state = job.state;
        match (state, job.has_thread) {
            (JobState::Queued | JobState::Running | JobState::WaitingToRetry, _) => return Ok(()),
            (JobState::Pausing, true) => {
                job.control.pause.store(false, Ordering::SeqCst);
                job.state = if job.parked {
                    JobState::Queued
                } else {
                    JobState::Running
                };
            }
            (JobState::Paused, true) => {
                // the parked thread wakes up and queues itself for a slot
                job.control.pause.store(false, Ordering::SeqCst);
                return Ok(());
            }
            (JobState::Paused, false) => {
                job.control.pause.store(false, Ordering::SeqCst);
                job.state = JobState::Queued;
                job.retry_at = None;
                self.insert_waiting(id, false);
            }
            _ => {
                return Err(JobQueueError::NotAllowed {
                    id,
                    action: "resume",
                    state,
                });
            }
        }
        self.touch(id);
        Ok(())
    }

    fn retry(&mut self, id: JobId) -> Result<(), JobQueueError> {
        let job = self.job_mut(id)?;
        let state = job.state;
        if !matches!(state, JobState::Failed | JobState::Cancelled)
            || job.has_thread
            || job.task.is_none()
        {
            return Err(JobQueueError::NotAllowed {
                id,
                action: "retry",
                state,
            });
        }
        job.state = JobState::Queued;
        job.control = Arc::new(Control::default());
        job.attempt = 0;
        job.retry_at = None;
        job.progress = None;
        job.message = None;
        job.error = None;
        job.result = None;
        job.eta.reset();
        self.insert_waiting(id, false);
        self.touch(id);
        Ok(())
    }

    fn set_priority(&mut self, id: JobId, priority: JobPriority) -> Result<(), JobQueueError> {
        self.job_mut(id)?.spec.priority = priority;
        if self.waiting.contains(&id) {
            self.remove_waiting(id);
            self.insert_waiting(id, false);
        }
        self.touch(id);
        Ok(())
    }

    fn move_before(&mut self, id: JobId, before: Option<JobId>) -> Result<(), JobQueueError> {
        let state = self.job(id)?.state;
        if !self.waiting.contains(&id) {
            return Err(JobQueueError::NotAllowed {
                id,
                action: "reorder",
                state,
            });
        }
        self.remove_waiting(id);
        let index = before
            .and_then(|b| self.waiting.iter().position(|other| *other == b))
            .unwrap_or(self.waiting.len());
        self.waiting.insert(index, id);
        self.order_dirty = true;
        Ok(())
    }

    fn remove(&mut self, id: JobId) -> Result<(), JobQueueError> {
        let state = self.job(id)?.state;
        if !state.is_finished() {
            return Err(JobQueueError::NotAllowed {
                id,
                action: "remove",
                state,
            });
        }
        self.jobs.remove(&id);
        self.revision += 1;
        let revision = self.revision;
        self.outbox.push(JobEvent::Removed { id, revision });
        Ok(())
    }
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    bus: EventBus,
}

/// sends the queued events and starts retry timers with the lock released, then takes it back
fn flush(shared: &Arc<Shared>, guard: &mut MutexGuard<'_, State>) {
    if guard.order_dirty {
        guard.order_dirty = false;
        guard.revision += 1;
        let event = JobEvent::Order {
            waiting: guard.waiting.clone(),
            revision: guard.revision,
        };
        guard.outbox.push(event);
    }
    let events = mem::take(&mut guard.outbox);
    let wakeups = mem::take(&mut guard.wakeups);
    shared.changed.notify_all();
    if events.is_empty() && wakeups.is_empty() {
        return;
    }
    MutexGuard::unlocked(guard, || {
        for delay in wakeups {
            start_wakeup(Arc::downgrade(shared), delay);
        }
        for event in events {
            shared.bus.emit(event);
        }
    });
}

/// runs `f` on the state, then starts whatever can start and sends the events
fn transact<R>(shared: &Arc<Shared>, f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = shared.state.lock();
    let result = f(&mut guard);
    guard.pump(shared);
    flush(shared, &mut guard);
    result
}

/// pumps the queue after a retry delay so the waiting job can start
fn start_wakeup(shared: Weak<Shared>, delay: Duration) {
    let spawned = thread::Builder::new()
        .name("vellum-job-retry".into())
        .spawn(move || {
            thread::sleep(delay);
            if let Some(shared) = shared.upgrade() {
                transact(&shared, |_| ());
            }
        });
    if let Err(err) = spawned {
        tracing::error!(%err, "could not start a retry timer");
    }
}

fn run_worker(
    shared: Arc<Shared>,
    id: JobId,
    mut task: Box<dyn JobTask>,
    control: Arc<Control>,
    attempt: u32,
) {
    let ctx = JobContext {
        id,
        attempt,
        shared: shared.clone(),
        control: control.clone(),
    };
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| task.run(&ctx)));
    let result = outcome.unwrap_or_else(|payload| {
        let reason = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown reason".to_owned());
        tracing::error!(%id, %reason, "job panicked");
        Err(JobError::fatal(format!("the job crashed: {reason}")))
    });
    transact(&shared, |state| state.finish(id, task, result, &control));
}

/// handed to every running job to report progress and to honour pause and cancel.
pub struct JobContext {
    id: JobId,
    attempt: u32,
    shared: Arc<Shared>,
    control: Arc<Control>,
}

impl JobContext {
    pub fn id(&self) -> JobId {
        self.id
    }

    /// 0 on the first run, then 1, 2, ... for automatic retries
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    pub fn is_cancelled(&self) -> bool {
        self.control.cancel.load(Ordering::SeqCst)
    }

    /// true while the user wants this job paused. jobs that wait on a child process use it
    /// to suspend the child before blocking in [`Self::checkpoint`].
    pub fn is_pause_requested(&self) -> bool {
        self.control.pause.load(Ordering::SeqCst)
    }

    /// sets progress from 0 to 1. cheap enough to call per frame, the ui only hears about
    /// it every 100 ms.
    pub fn set_progress(&self, progress: f32) {
        if progress.is_nan() {
            return;
        }
        let progress = progress.clamp(0.0, 1.0);
        self.update(|job, now| {
            job.progress = Some(progress);
            job.eta.sample(now, progress);
            job.last_emit
                .is_none_or(|last| now.saturating_duration_since(last) >= PROGRESS_INTERVAL)
        });
    }

    /// progress as `done` out of `total` steps
    pub fn set_progress_steps(&self, done: u64, total: u64) {
        if total > 0 {
            self.set_progress((done as f64 / total as f64) as f32);
        }
    }

    /// for jobs that cannot tell how far along they are
    pub fn set_indeterminate(&self) {
        self.update(|job, _| {
            job.progress = None;
            true
        });
    }

    /// a short status line, eg "Pass 2 of 2"
    pub fn set_message(&self, message: impl Into<String>) {
        let message = message.into();
        self.update(|job, _| {
            job.message = Some(message);
            true
        });
    }

    /// call this often in loops. returns `Err(Cancelled)` when the job should stop (pass it
    /// on with `?`) and blocks while the job is paused. while paused the job gives up its
    /// slot, so it may wait for one when resumed.
    pub fn checkpoint(&self) -> Result<(), JobError> {
        if self.control.cancel.load(Ordering::SeqCst) {
            return Err(JobError::Cancelled);
        }
        if !self.control.pause.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.wait_while_paused()
    }

    fn update(&self, f: impl FnOnce(&mut Job, Instant) -> bool) {
        let mut guard = self.shared.state.lock();
        let now = Instant::now();
        let Some(job) = guard.jobs.get_mut(&self.id) else {
            return;
        };
        if f(job, now) {
            guard.touch(self.id);
            flush(&self.shared, &mut guard);
        }
    }

    fn wait_while_paused(&self) -> Result<(), JobError> {
        let id = self.id;
        let shared = &self.shared;
        let mut guard = shared.state.lock();
        loop {
            let now = Instant::now();
            let state = &mut *guard;
            let Some(job) = state.jobs.get_mut(&id) else {
                return Err(JobError::Cancelled);
            };

            if self.control.cancel.load(Ordering::SeqCst) {
                job.slot_granted = false;
                if job.parked {
                    job.parked = false;
                    state.remove_waiting(id);
                }
                flush(shared, &mut guard);
                return Err(JobError::Cancelled);
            }

            if self.control.pause.load(Ordering::SeqCst) {
                if job.state != JobState::Paused {
                    let was_parked = mem::replace(&mut job.parked, false);
                    job.state = JobState::Paused;
                    job.eta.stop(now);
                    if was_parked {
                        state.remove_waiting(id);
                    }
                    state.release_slot(id);
                    state.touch(id);
                    state.pump(shared);
                    flush(shared, &mut guard);
                    continue;
                }
                shared.changed.wait(&mut guard);
                continue;
            }

            // resumed: carry on once we hold a slot again
            if job.holds_slot {
                job.slot_granted = false;
                job.parked = false;
                if job.state != JobState::Running {
                    job.state = JobState::Running;
                    job.eta.start(now);
                    state.touch(id);
                    flush(shared, &mut guard);
                }
                return Ok(());
            }
            if !job.parked {
                job.parked = true;
                job.state = JobState::Queued;
                state.insert_waiting(id, true);
                state.touch(id);
                state.pump(shared);
                flush(shared, &mut guard);
                continue;
            }
            shared.changed.wait(&mut guard);
        }
    }
}

/// the job queue. cheap to clone, every clone shares the same queue.
#[derive(Clone)]
pub struct JobQueue {
    shared: Arc<Shared>,
}

impl JobQueue {
    pub fn new(limits: JobLimits, bus: EventBus) -> Self {
        let state = State {
            jobs: BTreeMap::new(),
            waiting: Vec::new(),
            running: [0; 3],
            limits,
            next_id: 1,
            revision: 0,
            outbox: Vec::new(),
            wakeups: Vec::new(),
            order_dirty: false,
        };
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                changed: Condvar::new(),
                bus,
            }),
        }
    }

    pub fn submit(&self, spec: JobSpec, task: impl JobTask) -> JobId {
        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default();
        transact(&self.shared, |state| {
            let id = JobId(state.next_id);
            state.next_id += 1;
            state.jobs.insert(
                id,
                Job {
                    spec,
                    state: JobState::Queued,
                    task: Some(Box::new(task)),
                    control: Arc::new(Control::default()),
                    has_thread: false,
                    holds_slot: false,
                    parked: false,
                    slot_granted: false,
                    progress: None,
                    message: None,
                    error: None,
                    result: None,
                    attempt: 0,
                    retry_at: None,
                    eta: EtaEstimator::default(),
                    created_at_ms,
                    last_emit: None,
                },
            );
            state.insert_waiting(id, false);
            state.touch(id);
            id
        })
    }

    pub fn cancel(&self, id: JobId) -> Result<(), JobQueueError> {
        transact(&self.shared, |state| state.cancel(id))
    }

    /// cancels everything that has not finished, eg on shutdown
    pub fn cancel_all(&self) {
        transact(&self.shared, |state| {
            let ids: Vec<_> = state.jobs.keys().copied().collect();
            for id in ids {
                // finished jobs refuse, which is fine here
                let _ = state.cancel(id);
            }
        });
    }

    pub fn pause(&self, id: JobId) -> Result<(), JobQueueError> {
        transact(&self.shared, |state| state.pause(id))
    }

    pub fn resume(&self, id: JobId) -> Result<(), JobQueueError> {
        transact(&self.shared, |state| state.resume(id))
    }

    /// runs a failed or cancelled job again from the start
    pub fn retry(&self, id: JobId) -> Result<(), JobQueueError> {
        transact(&self.shared, |state| state.retry(id))
    }

    pub fn set_priority(&self, id: JobId, priority: JobPriority) -> Result<(), JobQueueError> {
        transact(&self.shared, |state| state.set_priority(id, priority))
    }

    /// moves a waiting job in front of `before`, or to the back with `None`
    pub fn move_before(&self, id: JobId, before: Option<JobId>) -> Result<(), JobQueueError> {
        transact(&self.shared, |state| state.move_before(id, before))
    }

    /// forgets a finished job
    pub fn remove(&self, id: JobId) -> Result<(), JobQueueError> {
        transact(&self.shared, |state| state.remove(id))
    }

    pub fn clear_finished(&self) {
        transact(&self.shared, |state| {
            let done: Vec<_> = state
                .jobs
                .iter()
                .filter(|(_, job)| job.state.is_finished())
                .map(|(id, _)| *id)
                .collect();
            for id in done {
                let _ = state.remove(id);
            }
        });
    }

    pub fn limits(&self) -> JobLimits {
        self.shared.state.lock().limits
    }

    /// lowering a limit never stops running jobs, it just holds back new ones
    pub fn set_limits(&self, limits: JobLimits) {
        transact(&self.shared, |state| state.limits = limits);
    }

    pub fn snapshot(&self, id: JobId) -> Option<JobSnapshot> {
        let state = self.shared.state.lock();
        let job = state.jobs.get(&id)?;
        Some(job.snapshot(id, state.revision, Instant::now()))
    }

    /// every job plus the waiting order, taken under one lock so they agree
    pub fn overview(&self) -> JobsOverview {
        let state = self.shared.state.lock();
        let now = Instant::now();
        JobsOverview {
            jobs: state
                .jobs
                .iter()
                .map(|(id, job)| job.snapshot(*id, state.revision, now))
                .collect(),
            waiting: state.waiting.clone(),
            revision: state.revision,
        }
    }

    /// every job, oldest first
    pub fn list(&self) -> Vec<JobSnapshot> {
        let state = self.shared.state.lock();
        let now = Instant::now();
        state
            .jobs
            .iter()
            .map(|(id, job)| job.snapshot(*id, state.revision, now))
            .collect()
    }

    /// the waiting jobs in the order they will start
    pub fn waiting(&self) -> Vec<JobId> {
        self.shared.state.lock().waiting.clone()
    }

    pub fn running_count(&self, resource: JobResource) -> usize {
        self.shared.state.lock().running[resource.index()]
    }

    /// blocks until the job finishes or the timeout passes. for the cli and tests, never
    /// call it on the ui thread.
    pub fn wait(&self, id: JobId, timeout: Duration) -> Option<JobSnapshot> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        loop {
            let job = state.jobs.get(&id)?;
            if job.state.is_finished() {
                return Some(job.snapshot(id, state.revision, Instant::now()));
            }
            if self
                .shared
                .changed
                .wait_until(&mut state, deadline)
                .timed_out()
            {
                return None;
            }
        }
    }

    /// blocks until `check` returns true for the job or the timeout passes
    pub fn wait_for(
        &self,
        id: JobId,
        timeout: Duration,
        check: impl Fn(&JobSnapshot) -> bool,
    ) -> Option<JobSnapshot> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        loop {
            let snapshot = state
                .jobs
                .get(&id)?
                .snapshot(id, state.revision, Instant::now());
            if check(&snapshot) {
                return Some(snapshot);
            }
            if self
                .shared
                .changed
                .wait_until(&mut state, deadline)
                .timed_out()
            {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobPriority;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    const WAIT: Duration = Duration::from_secs(10);

    fn queue(cpu: usize, gpu: usize) -> JobQueue {
        JobQueue::new(JobLimits { cpu, gpu, io: 1 }, EventBus::new())
    }

    /// a job that blocks until the returned sender sends or is dropped
    fn gated(
        log: Option<Arc<Mutex<Vec<&'static str>>>>,
        name: &'static str,
    ) -> (mpsc::Sender<()>, impl JobTask) {
        let (tx, rx) = mpsc::channel::<()>();
        let task = move |_: &JobContext| {
            let _ = rx.recv();
            if let Some(log) = &log {
                log.lock().push(name);
            }
            Ok(json!(name))
        };
        (tx, task)
    }

    /// a job that loops on checkpoints until told to finish
    fn looping(stop: Arc<AtomicBool>, ticks: Arc<AtomicUsize>) -> impl JobTask {
        move |ctx: &JobContext| {
            while !stop.load(Ordering::SeqCst) {
                ctx.checkpoint()?;
                ticks.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(2));
            }
            Ok(json!("done"))
        }
    }

    fn state_is(state: JobState) -> impl Fn(&JobSnapshot) -> bool {
        move |job| job.state == state
    }

    #[test]
    fn runs_a_job_and_returns_its_output() {
        let q = queue(1, 1);
        let id = q.submit(JobSpec::cpu("count"), |ctx: &JobContext| {
            for i in 0..=10 {
                ctx.checkpoint()?;
                ctx.set_progress_steps(i, 10);
            }
            ctx.set_message("all done");
            Ok(json!({ "count": 10 }))
        });
        let job = q.wait(id, WAIT).unwrap();
        assert_eq!(job.state, JobState::Completed);
        assert_eq!(job.result, Some(json!({ "count": 10 })));
        assert_eq!(job.progress, Some(1.0));
        assert_eq!(job.message.as_deref(), Some("all done"));
        assert_eq!(q.running_count(JobResource::Cpu), 0);
    }

    #[test]
    fn respects_the_limit_per_resource() {
        let q = queue(2, 1);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let ids: Vec<_> = (0..6)
            .map(|_| {
                let active = active.clone();
                let peak = peak.clone();
                q.submit(JobSpec::cpu("busy"), move |_: &JobContext| {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(30));
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(JobOutput::Null)
                })
            })
            .collect();
        for id in ids {
            assert_eq!(q.wait(id, WAIT).unwrap().state, JobState::Completed);
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn gpu_and_cpu_jobs_do_not_block_each_other() {
        let q = queue(1, 1);
        let (cpu_gate, cpu_task) = gated(None, "cpu");
        let (gpu_gate, gpu_task) = gated(None, "gpu");
        let cpu = q.submit(JobSpec::cpu("encode"), cpu_task);
        let gpu = q.submit(JobSpec::gpu("upscale"), gpu_task);
        assert!(q.wait_for(cpu, WAIT, state_is(JobState::Running)).is_some());
        assert!(q.wait_for(gpu, WAIT, state_is(JobState::Running)).is_some());

        // a second gpu job has to wait for the first
        let (_gate, second_task) = gated(None, "second");
        let second = q.submit(JobSpec::gpu("another"), second_task);
        assert_eq!(q.snapshot(second).unwrap().state, JobState::Queued);

        drop(cpu_gate);
        drop(gpu_gate);
        assert_eq!(q.wait(cpu, WAIT).unwrap().state, JobState::Completed);
        assert!(
            q.wait_for(second, WAIT, state_is(JobState::Running))
                .is_some()
        );
        q.cancel_all();
    }

    #[test]
    fn higher_priority_starts_first() {
        let q = queue(1, 1);
        let log = Arc::new(Mutex::new(Vec::new()));
        let (gate, blocker) = gated(Some(log.clone()), "blocker");
        let first = q.submit(JobSpec::cpu("blocker"), blocker);
        q.wait_for(first, WAIT, state_is(JobState::Running))
            .unwrap();

        let mut ids = Vec::new();
        for (name, priority) in [
            ("low", JobPriority::Low),
            ("normal", JobPriority::Normal),
            ("urgent", JobPriority::Urgent),
            ("high", JobPriority::High),
            ("normal2", JobPriority::Normal),
        ] {
            let log = log.clone();
            ids.push(q.submit(
                JobSpec::cpu(name).with_priority(priority),
                move |_: &JobContext| {
                    log.lock().push(name);
                    Ok(JobOutput::Null)
                },
            ));
        }
        drop(gate);
        for id in ids {
            q.wait(id, WAIT).unwrap();
        }
        assert_eq!(
            *log.lock(),
            ["blocker", "urgent", "high", "normal", "normal2", "low"]
        );
    }

    #[test]
    fn waiting_jobs_can_be_reordered_and_reprioritised() {
        let q = queue(1, 1);
        let (gate, blocker) = gated(None, "blocker");
        let first = q.submit(JobSpec::cpu("blocker"), blocker);
        q.wait_for(first, WAIT, state_is(JobState::Running))
            .unwrap();
        let a = q.submit(JobSpec::cpu("a"), |_: &JobContext| Ok(JobOutput::Null));
        let b = q.submit(JobSpec::cpu("b"), |_: &JobContext| Ok(JobOutput::Null));
        let c = q.submit(JobSpec::cpu("c"), |_: &JobContext| Ok(JobOutput::Null));
        assert_eq!(q.waiting(), [a, b, c]);
        let overview = q.overview();
        assert_eq!(overview.jobs.len(), 4);
        assert_eq!(overview.waiting, [a, b, c]);
        assert!(
            overview
                .jobs
                .iter()
                .all(|j| j.revision == overview.revision)
        );

        q.move_before(c, Some(a)).unwrap();
        assert_eq!(q.waiting(), [c, a, b]);
        q.move_before(c, None).unwrap();
        assert_eq!(q.waiting(), [a, b, c]);
        q.set_priority(b, JobPriority::High).unwrap();
        assert_eq!(q.waiting(), [b, a, c]);

        assert!(matches!(
            q.move_before(first, None),
            Err(JobQueueError::NotAllowed { .. })
        ));
        drop(gate);
    }

    #[test]
    fn cancel_a_queued_job_and_a_running_job() {
        let q = queue(1, 1);
        let stop = Arc::new(AtomicBool::new(false));
        let ticks = Arc::new(AtomicUsize::new(0));
        let running = q.submit(JobSpec::cpu("loop"), looping(stop.clone(), ticks.clone()));
        let queued = q.submit(JobSpec::cpu("never"), |_: &JobContext| Ok(JobOutput::Null));
        q.wait_for(running, WAIT, state_is(JobState::Running))
            .unwrap();

        q.cancel(queued).unwrap();
        assert_eq!(q.snapshot(queued).unwrap().state, JobState::Cancelled);

        q.cancel(running).unwrap();
        let job = q.wait(running, WAIT).unwrap();
        assert_eq!(job.state, JobState::Cancelled);
        assert_eq!(q.running_count(JobResource::Cpu), 0);

        // cancelling again is harmless, cancelling a completed job is not allowed
        q.cancel(running).unwrap();
        let done = q.submit(JobSpec::cpu("quick"), |_: &JobContext| Ok(JobOutput::Null));
        q.wait(done, WAIT).unwrap();
        assert!(matches!(
            q.cancel(done),
            Err(JobQueueError::NotAllowed { .. })
        ));
    }

    #[test]
    fn pause_stops_progress_and_frees_the_slot() {
        let q = queue(1, 1);
        let stop = Arc::new(AtomicBool::new(false));
        let ticks = Arc::new(AtomicUsize::new(0));
        let looper = q.submit(JobSpec::cpu("loop"), looping(stop.clone(), ticks.clone()));
        q.wait_for(looper, WAIT, state_is(JobState::Running))
            .unwrap();

        q.pause(looper).unwrap();
        q.wait_for(looper, WAIT, state_is(JobState::Paused))
            .unwrap();
        let frozen = ticks.load(Ordering::SeqCst);

        // the freed slot lets another job run to completion while the first is paused
        let other = q.submit(JobSpec::cpu("other"), |_: &JobContext| Ok(json!(1)));
        assert_eq!(q.wait(other, WAIT).unwrap().state, JobState::Completed);
        assert_eq!(ticks.load(Ordering::SeqCst), frozen);

        q.resume(looper).unwrap();
        q.wait_for(looper, WAIT, state_is(JobState::Running))
            .unwrap();
        stop.store(true, Ordering::SeqCst);
        assert_eq!(q.wait(looper, WAIT).unwrap().state, JobState::Completed);
        assert!(ticks.load(Ordering::SeqCst) > frozen);
    }

    #[test]
    fn resumed_job_waits_for_a_slot() {
        let q = queue(1, 1);
        let stop = Arc::new(AtomicBool::new(false));
        let ticks = Arc::new(AtomicUsize::new(0));
        let looper = q.submit(JobSpec::cpu("loop"), looping(stop.clone(), ticks.clone()));
        q.wait_for(looper, WAIT, state_is(JobState::Running))
            .unwrap();
        q.pause(looper).unwrap();
        q.wait_for(looper, WAIT, state_is(JobState::Paused))
            .unwrap();

        let (gate, blocker) = gated(None, "blocker");
        let other = q.submit(JobSpec::cpu("blocker"), blocker);
        q.wait_for(other, WAIT, state_is(JobState::Running))
            .unwrap();

        q.resume(looper).unwrap();
        q.wait_for(looper, WAIT, state_is(JobState::Queued))
            .unwrap();
        assert_eq!(q.waiting(), [looper]);

        drop(gate);
        q.wait_for(looper, WAIT, state_is(JobState::Running))
            .unwrap();
        stop.store(true, Ordering::SeqCst);
        assert_eq!(q.wait(looper, WAIT).unwrap().state, JobState::Completed);
    }

    #[test]
    fn paused_jobs_can_be_cancelled() {
        let q = queue(1, 1);
        let stop = Arc::new(AtomicBool::new(false));
        let ticks = Arc::new(AtomicUsize::new(0));
        let looper = q.submit(JobSpec::cpu("loop"), looping(stop, ticks));
        q.wait_for(looper, WAIT, state_is(JobState::Running))
            .unwrap();
        q.pause(looper).unwrap();
        q.wait_for(looper, WAIT, state_is(JobState::Paused))
            .unwrap();
        q.cancel(looper).unwrap();
        assert_eq!(q.wait(looper, WAIT).unwrap().state, JobState::Cancelled);
        assert_eq!(q.running_count(JobResource::Cpu), 0);
    }

    #[test]
    fn queued_jobs_pause_and_resume_without_running() {
        let q = queue(1, 1);
        let (gate, blocker) = gated(None, "blocker");
        let first = q.submit(JobSpec::cpu("blocker"), blocker);
        q.wait_for(first, WAIT, state_is(JobState::Running))
            .unwrap();
        let second = q.submit(JobSpec::cpu("second"), |_: &JobContext| Ok(JobOutput::Null));
        q.pause(second).unwrap();
        assert_eq!(q.snapshot(second).unwrap().state, JobState::Paused);
        assert!(q.waiting().is_empty());

        drop(gate);
        q.wait(first, WAIT).unwrap();
        assert_eq!(q.snapshot(second).unwrap().state, JobState::Paused);

        q.resume(second).unwrap();
        assert_eq!(q.wait(second, WAIT).unwrap().state, JobState::Completed);
    }

    #[test]
    fn retryable_failures_retry_automatically() {
        let q = queue(1, 1);
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let seen = attempts.clone();
        let id = q.submit(
            JobSpec::io("flaky download").with_retries(3, Duration::from_millis(10)),
            move |ctx: &JobContext| {
                seen.lock().push(ctx.attempt());
                if ctx.attempt() < 2 {
                    Err(JobError::failed("connection reset"))
                } else {
                    Ok(json!("ok"))
                }
            },
        );
        let job = q.wait(id, WAIT).unwrap();
        assert_eq!(job.state, JobState::Completed);
        assert_eq!(job.attempt, 2);
        assert_eq!(*attempts.lock(), [0, 1, 2]);
    }

    #[test]
    fn fatal_failures_and_exhausted_retries_fail() {
        let q = queue(1, 1);
        let fatal = q.submit(
            JobSpec::cpu("missing file").with_retries(3, Duration::from_millis(1)),
            |_: &JobContext| Err(JobError::fatal("file not found")),
        );
        let job = q.wait(fatal, WAIT).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.attempt, 0);
        assert_eq!(job.error.as_deref(), Some("file not found"));

        let flaky = q.submit(
            JobSpec::cpu("always fails").with_retries(2, Duration::from_millis(1)),
            |_: &JobContext| Err(JobError::failed("still broken")),
        );
        let job = q.wait(flaky, WAIT).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.attempt, 2);
    }

    #[test]
    fn manual_retry_runs_a_failed_job_again() {
        let q = queue(1, 1);
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = runs.clone();
        let id = q.submit(JobSpec::cpu("second time lucky"), move |_: &JobContext| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(JobError::fatal("first run fails"))
            } else {
                Ok(json!("fine"))
            }
        });
        assert_eq!(q.wait(id, WAIT).unwrap().state, JobState::Failed);
        q.retry(id).unwrap();
        let job = q.wait(id, WAIT).unwrap();
        assert_eq!(job.state, JobState::Completed);
        assert_eq!(job.error, None);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        // completed jobs cannot be retried
        assert!(q.retry(id).is_err());
    }

    #[test]
    fn panics_become_failures() {
        let q = queue(1, 1);
        let id = q.submit(
            JobSpec::cpu("explodes"),
            |_: &JobContext| -> Result<JobOutput, JobError> { panic!("kaboom") },
        );
        let job = q.wait(id, WAIT).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert!(job.error.unwrap().contains("kaboom"));

        // the queue still works afterwards
        let next = q.submit(JobSpec::cpu("fine"), |_: &JobContext| Ok(JobOutput::Null));
        assert_eq!(q.wait(next, WAIT).unwrap().state, JobState::Completed);
    }

    #[test]
    fn raising_the_limit_starts_waiting_jobs() {
        let q = queue(1, 1);
        let (gate_a, task_a) = gated(None, "a");
        let (gate_b, task_b) = gated(None, "b");
        let a = q.submit(JobSpec::cpu("a"), task_a);
        let b = q.submit(JobSpec::cpu("b"), task_b);
        q.wait_for(a, WAIT, state_is(JobState::Running)).unwrap();
        assert_eq!(q.snapshot(b).unwrap().state, JobState::Queued);
        q.set_limits(JobLimits {
            cpu: 2,
            ..q.limits()
        });
        q.wait_for(b, WAIT, state_is(JobState::Running)).unwrap();
        drop((gate_a, gate_b));
    }

    #[test]
    fn finished_jobs_can_be_removed() {
        let q = queue(1, 1);
        let id = q.submit(JobSpec::cpu("quick"), |_: &JobContext| Ok(JobOutput::Null));
        let (_gate, task) = gated(None, "slow");
        let slow = q.submit(JobSpec::cpu("slow"), task);
        q.wait(id, WAIT).unwrap();
        assert!(q.remove(slow).is_err());
        q.clear_finished();
        assert!(q.snapshot(id).is_none());
        assert!(q.snapshot(slow).is_some());
        assert_eq!(q.remove(id), Err(JobQueueError::UnknownJob(id)));
        q.cancel_all();
    }

    #[test]
    fn background_jobs_hand_over_their_result_then_leave_the_queue() {
        let bus = EventBus::new();
        let events = bus.subscribe::<JobEvent>();
        let q = JobQueue::new(JobLimits::default(), bus);
        let id = q.submit(
            JobSpec::cpu("thumbnail").in_background(),
            |_: &JobContext| Ok(json!({ "path": "thumb.jpg" })),
        );
        let mut final_result = None;
        let mut removed = false;
        while let Some(event) = events.recv_timeout(Duration::from_millis(500)) {
            match event {
                JobEvent::Updated { job } if job.state == JobState::Completed => {
                    assert!(job.background);
                    final_result = job.result;
                }
                JobEvent::Removed { id: gone, .. } if gone == id => {
                    removed = true;
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(final_result, Some(json!({ "path": "thumb.jpg" })));
        assert!(removed);
        assert!(q.snapshot(id).is_none());
    }

    #[test]
    fn events_tell_the_whole_story_in_order() {
        let bus = EventBus::new();
        let events = bus.subscribe::<JobEvent>();
        let q = JobQueue::new(JobLimits::default(), bus);
        let id = q.submit(JobSpec::cpu("story"), |ctx: &JobContext| {
            ctx.set_progress(0.5);
            Ok(json!("end"))
        });
        q.wait(id, WAIT).unwrap();
        q.remove(id).unwrap();

        // events from different threads can arrive out of order, the revision puts them back
        let mut updates = Vec::new();
        let mut removed = false;
        while let Some(event) = events.recv_timeout(Duration::from_millis(200)) {
            match event {
                JobEvent::Updated { job } => updates.push(job),
                JobEvent::Removed { id: gone, .. } => {
                    assert_eq!(gone, id);
                    removed = true;
                }
                JobEvent::Order { .. } => {}
            }
        }
        updates.sort_by_key(|job| job.revision);
        let revisions: Vec<_> = updates.iter().map(|job| job.revision).collect();
        let mut unique = revisions.clone();
        unique.dedup();
        assert_eq!(revisions, unique, "revisions must never repeat");

        let mut states = Vec::new();
        for job in &updates {
            if states.last() != Some(&job.state) {
                states.push(job.state);
            }
        }
        assert_eq!(updates.last().map(|job| job.progress), Some(Some(1.0)));
        assert_eq!(
            states,
            [JobState::Queued, JobState::Running, JobState::Completed]
        );
        assert!(removed);
    }

    #[test]
    fn progress_events_are_throttled() {
        let bus = EventBus::new();
        let events = bus.subscribe::<JobEvent>();
        let q = JobQueue::new(JobLimits::default(), bus);
        let id = q.submit(JobSpec::cpu("chatty"), |ctx: &JobContext| {
            for i in 0..10_000 {
                ctx.set_progress_steps(i, 10_000);
            }
            Ok(JobOutput::Null)
        });
        q.wait(id, WAIT).unwrap();
        let updates = events
            .drain()
            .into_iter()
            .filter(|e| matches!(e, JobEvent::Updated { .. }))
            .count();
        assert!(updates < 100, "{updates} updates for one quick job");
    }

    #[test]
    fn eta_shows_up_while_running() {
        let q = queue(1, 1);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let id = q.submit(JobSpec::cpu("steady"), move |ctx: &JobContext| {
            let mut i = 0u64;
            while !flag.load(Ordering::SeqCst) {
                ctx.checkpoint()?;
                ctx.set_progress_steps(i.min(900), 1000);
                i += 1;
                thread::sleep(Duration::from_millis(5));
            }
            Ok(JobOutput::Null)
        });
        let job = q.wait_for(id, WAIT, |job| job.eta_secs.is_some()).unwrap();
        assert!(job.eta_secs.unwrap() > 0.0);
        assert!(job.elapsed_secs >= 1.0);
        stop.store(true, Ordering::SeqCst);
        q.wait(id, WAIT).unwrap();
    }

    #[test]
    fn snapshots_serialise_for_the_ui() {
        let q = queue(1, 1);
        let id = q.submit(
            JobSpec::gpu("Remove background").with_priority(JobPriority::High),
            |_: &JobContext| Ok(json!({ "path": "out.png" })),
        );
        let job = q.wait(id, WAIT).unwrap();
        let value = serde_json::to_value(JobEvent::Updated { job }).unwrap();
        assert_eq!(value["type"], "updated");
        assert_eq!(value["job"]["resource"], "gpu");
        assert_eq!(value["job"]["priority"], "high");
        assert_eq!(value["job"]["state"], "completed");
        assert_eq!(value["job"]["result"]["path"], "out.png");
        assert!(value["job"]["createdAtMs"].as_u64().unwrap() > 0);
    }
}
