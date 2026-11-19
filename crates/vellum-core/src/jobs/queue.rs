use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use parking_lot::{Condvar, Mutex};

use super::{
    Control, Job, JobContext, JobError, JobEvent, JobId, JobInfo, JobKind, JobOutput, JobSpec,
    JobStatus, Priority,
};
use crate::events::{EventBus, VellumEvent};
use crate::settings::JobSettings;

#[derive(Clone, Debug)]
pub struct QueueConfig {
    /// how many jobs of each kind may run at once
    pub limits: HashMap<JobKind, usize>,
    /// minimum gap between progress events for one job
    pub progress_interval: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self::from_settings(&JobSettings::default())
    }
}

impl QueueConfig {
    pub fn from_settings(settings: &JobSettings) -> Self {
        let limits = HashMap::from([
            (JobKind::Cpu, settings.max_cpu_jobs as usize),
            (JobKind::Gpu, settings.max_gpu_jobs as usize),
            (JobKind::Io, settings.max_io_jobs as usize),
        ]);
        Self {
            limits,
            progress_interval: Duration::from_millis(100),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum JobQueueError {
    #[error("job {0} does not exist")]
    NotFound(JobId),
    #[error("job {id} cannot do that while it is {status:?}")]
    InvalidState { id: JobId, status: JobStatus },
}

struct Record {
    info: JobInfo,
    // present whenever the job is not running, so it can be started or retried
    job: Option<Box<dyn Job>>,
    control: Arc<Control>,
    seq: u64,
    max_attempts: u32,
    // attempts since the last manual retry, auto retry compares this to max_attempts
    round_attempts: u32,
    active_since: Option<Instant>,
    active_time: Duration,
    last_progress_event: Option<Instant>,
}

impl Record {
    fn is_running(&self) -> bool {
        self.job.is_none() && matches!(self.info.status, JobStatus::Running | JobStatus::Paused)
    }

    fn stop_clock(&mut self) {
        if let Some(since) = self.active_since.take() {
            self.active_time += since.elapsed();
        }
    }

    fn eta(&self) -> Option<f64> {
        let p = f64::from(self.info.progress);
        if p <= 0.0 || p >= 1.0 {
            return None;
        }
        let active = self.active_time + self.active_since.map_or(Duration::ZERO, |s| s.elapsed());
        Some(active.as_secs_f64() * (1.0 - p) / p)
    }
}

struct State {
    records: BTreeMap<JobId, Record>,
    next_id: JobId,
    next_seq: u64,
    running: HashMap<JobKind, usize>,
    limits: HashMap<JobKind, usize>,
    scheduling_paused: bool,
    shutting_down: bool,
    threads: Vec<JoinHandle<()>>,
}

pub(crate) struct Shared {
    state: Mutex<State>,
    finished: Condvar,
    // events go through a channel to a dispatcher thread so they are published in order and
    // never while the state lock is held (a subscriber may call back into the queue)
    events: Sender<JobEvent>,
    progress_interval: Duration,
}

impl Shared {
    fn send(&self, event: JobEvent) {
        // the dispatcher only stops once every sender is gone, so this cannot fail in practice
        let _ = self.events.send(event);
    }

    fn changed(&self, record: &Record) {
        self.send(JobEvent::Changed {
            job: record.info.clone(),
        });
    }

    pub(crate) fn report(&self, id: JobId, progress: Option<f32>, message: Option<String>) {
        let mut st = self.state.lock();
        let Some(rec) = st.records.get_mut(&id) else {
            return;
        };
        if !rec.is_running() {
            return;
        }
        if let Some(p) = progress.filter(|p| !p.is_nan()) {
            rec.info.progress = p.clamp(0.0, 1.0);
        }
        let has_message = message.is_some();
        if has_message {
            rec.info.message = message;
        }
        rec.info.eta_secs = rec.eta();

        let now = Instant::now();
        let due = has_message
            || rec.info.progress >= 1.0
            || rec
                .last_progress_event
                .is_none_or(|t| now.duration_since(t) >= self.progress_interval);
        if due {
            rec.last_progress_event = Some(now);
            self.send(JobEvent::Progress {
                id,
                progress: rec.info.progress,
                eta_secs: rec.info.eta_secs,
                message: rec.info.message.clone(),
            });
        }
    }
}

/// runs [`Job`]s on background threads with per kind limits, priorities, pause, cancel and retry.
pub struct JobQueue {
    shared: Arc<Shared>,
}

impl JobQueue {
    pub fn new(config: QueueConfig, bus: EventBus<VellumEvent>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<JobEvent>();
        // exits by itself once the queue and every job thread have dropped their sender
        thread::Builder::new()
            .name("vellum-job-events".into())
            .spawn(move || {
                for event in rx {
                    bus.publish(VellumEvent::Job(event));
                }
            })
            .map_err(|e| eprintln!("vellum: job events will not reach the ui: {e}"))
            .ok();

        let limits = config
            .limits
            .into_iter()
            .map(|(kind, n)| (kind, n.max(1)))
            .collect();
        let state = State {
            records: BTreeMap::new(),
            next_id: 1,
            next_seq: 0,
            running: HashMap::new(),
            limits,
            scheduling_paused: false,
            shutting_down: false,
            threads: Vec::new(),
        };
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                finished: Condvar::new(),
                events: tx,
                progress_interval: config.progress_interval,
            }),
        }
    }

    pub fn submit(&self, spec: JobSpec, job: impl Job) -> JobId {
        let mut st = self.shared.state.lock();
        let id = st.next_id;
        st.next_id += 1;
        let seq = st.next_seq;
        st.next_seq += 1;

        let record = Record {
            info: JobInfo {
                id,
                label: spec.label,
                kind: spec.kind,
                priority: spec.priority,
                status: JobStatus::Queued,
                progress: 0.0,
                message: None,
                eta_secs: None,
                error: None,
                attempts: 0,
                result: None,
            },
            job: Some(Box::new(job)),
            control: Arc::new(Control::default()),
            seq,
            max_attempts: spec.max_attempts.max(1),
            round_attempts: 0,
            active_since: None,
            active_time: Duration::ZERO,
            last_progress_event: None,
        };
        self.shared.changed(&record);
        st.records.insert(id, record);
        pump(&self.shared, &mut st);
        id
    }

    pub fn get(&self, id: JobId) -> Option<JobInfo> {
        self.shared
            .state
            .lock()
            .records
            .get(&id)
            .map(|r| r.info.clone())
    }

    /// every job the queue knows about, oldest first.
    pub fn list(&self) -> Vec<JobInfo> {
        self.shared
            .state
            .lock()
            .records
            .values()
            .map(|r| r.info.clone())
            .collect()
    }

    /// queued jobs are cancelled straight away. running jobs stop at their next checkpoint.
    pub fn cancel(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut st = self.shared.state.lock();
        let rec = record(&mut st, id)?;
        if rec.info.status.is_finished() {
            return Err(invalid(rec));
        }
        if rec.is_running() {
            rec.control.cancel();
            rec.info.message = Some("Cancelling".into());
        } else {
            rec.info.status = JobStatus::Cancelled;
            self.shared.finished.notify_all();
        }
        self.shared.changed(rec);
        Ok(())
    }

    /// a queued job will not start until resumed. a running job blocks at its next checkpoint
    /// but keeps its worker slot, so resuming it is instant.
    pub fn pause(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut st = self.shared.state.lock();
        let rec = record(&mut st, id)?;
        match rec.info.status {
            JobStatus::Paused => return Ok(()),
            JobStatus::Queued => {}
            JobStatus::Running => {
                rec.control.set_paused(true);
                rec.stop_clock();
            }
            _ => return Err(invalid(rec)),
        }
        rec.info.status = JobStatus::Paused;
        rec.info.eta_secs = None;
        self.shared.changed(rec);
        Ok(())
    }

    pub fn resume(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut st = self.shared.state.lock();
        let rec = record(&mut st, id)?;
        match rec.info.status {
            JobStatus::Paused if rec.is_running() => {
                rec.info.status = JobStatus::Running;
                rec.active_since = Some(Instant::now());
                rec.control.set_paused(false);
            }
            JobStatus::Paused => rec.info.status = JobStatus::Queued,
            JobStatus::Queued | JobStatus::Running => return Ok(()),
            _ => return Err(invalid(rec)),
        }
        self.shared.changed(rec);
        pump(&self.shared, &mut st);
        Ok(())
    }

    /// queues a failed or cancelled job again.
    pub fn retry(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut st = self.shared.state.lock();
        // retried jobs go to the back of their priority band
        let seq = st.next_seq;
        let rec = record(&mut st, id)?;
        let can_retry = matches!(rec.info.status, JobStatus::Failed | JobStatus::Cancelled);
        if !can_retry || rec.job.is_none() {
            return Err(invalid(rec));
        }
        rec.info.status = JobStatus::Queued;
        rec.info.progress = 0.0;
        rec.info.error = None;
        rec.info.message = None;
        rec.round_attempts = 0;
        rec.seq = seq;
        self.shared.changed(rec);
        st.next_seq += 1;
        pump(&self.shared, &mut st);
        Ok(())
    }

    pub fn set_priority(&self, id: JobId, priority: Priority) -> Result<(), JobQueueError> {
        let mut st = self.shared.state.lock();
        let rec = record(&mut st, id)?;
        if rec.info.priority != priority {
            rec.info.priority = priority;
            self.shared.changed(rec);
        }
        Ok(())
    }

    /// removes a finished job from the list.
    pub fn remove(&self, id: JobId) -> Result<(), JobQueueError> {
        let mut st = self.shared.state.lock();
        let rec = record(&mut st, id)?;
        if !rec.info.status.is_finished() {
            return Err(invalid(rec));
        }
        st.records.remove(&id);
        self.shared.send(JobEvent::Removed { id });
        Ok(())
    }

    pub fn clear_finished(&self) {
        let mut st = self.shared.state.lock();
        let done: Vec<JobId> = st
            .records
            .values()
            .filter(|r| r.info.status.is_finished())
            .map(|r| r.info.id)
            .collect();
        for id in done {
            st.records.remove(&id);
            self.shared.send(JobEvent::Removed { id });
        }
    }

    pub fn set_limit(&self, kind: JobKind, max_parallel: usize) {
        let mut st = self.shared.state.lock();
        st.limits.insert(kind, max_parallel.max(1));
        pump(&self.shared, &mut st);
    }

    /// stops new jobs from starting. running jobs carry on.
    pub fn set_scheduling_paused(&self, paused: bool) {
        let mut st = self.shared.state.lock();
        st.scheduling_paused = paused;
        pump(&self.shared, &mut st);
    }

    /// blocks until the job finishes or `timeout` passes. returns the final status, or `None`
    /// on timeout or if the job does not exist.
    pub fn wait(&self, id: JobId, timeout: Duration) -> Option<JobStatus> {
        let deadline = Instant::now() + timeout;
        let mut st = self.shared.state.lock();
        loop {
            let status = st.records.get(&id)?.info.status;
            if status.is_finished() {
                return Some(status);
            }
            if self
                .shared
                .finished
                .wait_until(&mut st, deadline)
                .timed_out()
            {
                let status = st.records.get(&id)?.info.status;
                return status.is_finished().then_some(status);
            }
        }
    }

    /// cancels everything and waits for running jobs to stop. called on drop.
    pub fn shutdown(&self) {
        let threads = {
            let mut st = self.shared.state.lock();
            st.shutting_down = true;
            for rec in st.records.values_mut() {
                if rec.is_running() {
                    rec.control.cancel();
                } else if !rec.info.status.is_finished() {
                    rec.info.status = JobStatus::Cancelled;
                    self.shared.changed(rec);
                }
            }
            std::mem::take(&mut st.threads)
        };
        for handle in threads {
            // a panicking job is already caught inside the thread, so join errors are not expected
            let _ = handle.join();
        }
    }
}

impl Drop for JobQueue {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn record(st: &mut State, id: JobId) -> Result<&mut Record, JobQueueError> {
    st.records.get_mut(&id).ok_or(JobQueueError::NotFound(id))
}

fn invalid(rec: &Record) -> JobQueueError {
    JobQueueError::InvalidState {
        id: rec.info.id,
        status: rec.info.status,
    }
}

/// starts as many queued jobs as the limits allow, highest priority first, then oldest first.
fn pump(shared: &Arc<Shared>, st: &mut State) {
    if st.shutting_down || st.scheduling_paused {
        return;
    }
    st.threads.retain(|h| !h.is_finished());
    loop {
        let next = st
            .records
            .values()
            .filter(|r| r.info.status == JobStatus::Queued && r.job.is_some())
            .filter(|r| {
                let running = st.running.get(&r.info.kind).copied().unwrap_or(0);
                let limit = st.limits.get(&r.info.kind).copied().unwrap_or(1);
                running < limit
            })
            .max_by_key(|r| (r.info.priority, Reverse(r.seq)))
            .map(|r| r.info.id);
        match next {
            Some(id) => start(shared, st, id),
            None => break,
        }
    }
}

fn start(shared: &Arc<Shared>, st: &mut State, id: JobId) {
    let Some(rec) = st.records.get_mut(&id) else {
        return;
    };
    let Some(mut job) = rec.job.take() else {
        return;
    };
    rec.control.reset();
    rec.info.status = JobStatus::Running;
    rec.info.attempts += 1;
    rec.round_attempts += 1;
    rec.info.progress = 0.0;
    rec.info.eta_secs = None;
    rec.info.result = None;
    rec.active_since = Some(Instant::now());
    rec.active_time = Duration::ZERO;
    rec.last_progress_event = None;
    shared.changed(rec);

    let kind = rec.info.kind;
    let ctx = JobContext {
        id,
        control: Arc::clone(&rec.control),
        queue: Arc::clone(shared),
        started: Instant::now(),
    };
    *st.running.entry(kind).or_default() += 1;

    let worker = Arc::clone(shared);
    let spawned = thread::Builder::new()
        .name(format!("vellum-job-{id}"))
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| job.run(&ctx)));
            drop(ctx);
            finish(&worker, id, kind, job, outcome);
        });

    match spawned {
        Ok(handle) => st.threads.push(handle),
        Err(e) => {
            // the job moved into the closure that failed to spawn, so it cannot be retried
            if let Some(n) = st.running.get_mut(&kind) {
                *n = n.saturating_sub(1);
            }
            if let Some(rec) = st.records.get_mut(&id) {
                rec.info.status = JobStatus::Failed;
                rec.info.error = Some(format!("could not start a worker thread: {e}"));
                shared.changed(rec);
            }
            shared.finished.notify_all();
        }
    }
}

type Outcome = std::thread::Result<Result<JobOutput, JobError>>;

fn finish(shared: &Arc<Shared>, id: JobId, kind: JobKind, job: Box<dyn Job>, outcome: Outcome) {
    let mut guard = shared.state.lock();
    let st = &mut *guard;
    if let Some(n) = st.running.get_mut(&kind) {
        *n = n.saturating_sub(1);
    }
    let shutting_down = st.shutting_down;
    if let Some(rec) = st.records.get_mut(&id) {
        rec.job = Some(job);
        rec.stop_clock();
        rec.info.eta_secs = None;
        let cancelled = rec.control.is_cancelled();

        let failure = match outcome {
            Ok(Ok(output)) => {
                rec.info.status = JobStatus::Completed;
                rec.info.progress = 1.0;
                rec.info.result = output;
                rec.info.message = None;
                None
            }
            Ok(Err(JobError::Cancelled)) => Some(None),
            Ok(Err(JobError::Failed(msg))) => Some(Some(msg)),
            Err(panic) => Some(Some(format!("job crashed: {}", panic_message(&panic)))),
        };

        match failure {
            None => {}
            Some(_) if cancelled || shutting_down => {
                rec.info.status = JobStatus::Cancelled;
                rec.info.message = None;
            }
            Some(None) => {
                rec.info.status = JobStatus::Cancelled;
                rec.info.message = None;
            }
            Some(Some(msg)) if rec.round_attempts < rec.max_attempts => {
                rec.info.status = JobStatus::Queued;
                rec.info.message = Some(format!("Retrying after: {msg}"));
                rec.info.error = Some(msg);
            }
            Some(Some(msg)) => {
                rec.info.status = JobStatus::Failed;
                rec.info.message = None;
                rec.info.error = Some(msg);
            }
        }
        shared.changed(rec);
    }
    shared.finished.notify_all();
    pump(shared, st);
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}
