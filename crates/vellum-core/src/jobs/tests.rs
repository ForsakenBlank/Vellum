use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::event::EventBus;

const TIMEOUT: Duration = Duration::from_secs(20);

struct Rig {
    queue: JobQueue,
    events: Arc<Mutex<Vec<JobEvent>>>,
}

fn rig(limits: JobLimits) -> Rig {
    rig_with(JobQueueConfig {
        limits,
        progress_interval: Duration::from_millis(5),
        keep_finished: 50,
    })
}

fn rig_with(config: JobQueueConfig) -> Rig {
    let bus = EventBus::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    bus.subscribe(move |event: &JobEvent| sink.lock().push(event.clone()))
        .detach();
    Rig {
        queue: JobQueue::new(bus, config).unwrap(),
        events,
    }
}

fn one_at_a_time() -> JobLimits {
    JobLimits {
        cpu: 1,
        gpu: 1,
        io: 1,
    }
}

impl Rig {
    fn updates_for(&self, id: JobId) -> Vec<JobSnapshot> {
        self.events
            .lock()
            .iter()
            .filter_map(|event| match event {
                JobEvent::Updated { job } if job.id == id => Some(job.clone()),
                _ => None,
            })
            .collect()
    }

    fn last_order(&self) -> Option<Vec<JobId>> {
        self.events
            .lock()
            .iter()
            .rev()
            .find_map(|event| match event {
                JobEvent::Reordered { pending } => Some(pending.clone()),
                _ => None,
            })
    }
}

/// holds jobs in place until the test lets them through
#[derive(Clone, Default)]
struct Gate(Arc<AtomicBool>);

impl Gate {
    fn open(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn pass(&self, ctx: &JobContext) -> Result<(), JobError> {
        while !self.0.load(Ordering::SeqCst) {
            ctx.sleep(Duration::from_millis(1))?;
        }
        Ok(())
    }

    fn job(&self) -> impl Job {
        let gate = self.clone();
        move |ctx: &JobContext| {
            gate.pass(ctx)?;
            Ok(Value::Null)
        }
    }
}

fn eventually(what: &str, check: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

fn state(handle: &JobHandle) -> JobState {
    handle.snapshot().unwrap().state
}

fn until_state(handle: &JobHandle, wanted: JobState) {
    eventually(&format!("job is {wanted:?}"), || state(handle) == wanted);
}

fn finish(handle: &JobHandle) -> JobSnapshot {
    handle
        .wait_timeout(TIMEOUT)
        .unwrap()
        .expect("the job should have finished by now")
}

#[test]
fn a_job_runs_and_hands_back_its_output() {
    let rig = rig(JobLimits::default());
    let spec = JobSpec::cpu("Export clip.mp4").category("export");
    let handle = rig.queue.submit(spec, |ctx: &JobContext| {
        ctx.message("Encoding");
        ctx.progress(0.5);
        Ok(json!({ "path": "clip.mp4" }))
    });

    let done = finish(&handle);
    assert_eq!(done.state, JobState::Succeeded);
    assert_eq!(done.output, Some(json!({ "path": "clip.mp4" })));
    assert_eq!(done.progress, Some(1.0));
    assert_eq!(done.message.as_deref(), Some("Encoding"));
    assert_eq!(
        (done.attempt, done.category.as_deref()),
        (1, Some("export"))
    );
    assert!(done.started_ms.is_some() && done.finished_ms.is_some());
    assert!(!done.can_retry);
}

#[test]
fn the_ui_hears_about_a_job_from_start_to_finish() {
    let rig = rig(JobLimits::default());
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Quick"), |_: &JobContext| Ok(Value::Null));
    finish(&handle);

    eventually("the final update arrives", || {
        rig.updates_for(handle.id())
            .last()
            .is_some_and(|job| job.state == JobState::Succeeded)
    });
    let updates = rig.updates_for(handle.id());
    assert_eq!(updates[0].name, "Quick");
    assert!(updates.iter().all(|job| job.id == handle.id()));
}

#[test]
fn progress_events_are_rate_limited() {
    let rig = rig_with(JobQueueConfig {
        progress_interval: Duration::from_millis(40),
        ..JobQueueConfig::default()
    });
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Chatty"), |ctx: &JobContext| {
            for step in 0..2000 {
                ctx.progress(step as f32 / 2000.0);
                if step % 100 == 0 {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            Ok(Value::Null)
        });
    finish(&handle);
    eventually("the final update arrives", || {
        rig.updates_for(handle.id())
            .last()
            .is_some_and(|job| job.state == JobState::Succeeded)
    });

    let updates = rig.updates_for(handle.id());
    assert!(
        updates.len() < 60,
        "2000 progress reports became {} events",
        updates.len()
    );
    assert_eq!(updates.last().unwrap().progress, Some(1.0));
}

#[test]
fn a_failure_keeps_its_message_and_can_be_retried_by_hand() {
    let rig = rig(JobLimits::default());
    let handle = rig.queue.submit(JobSpec::cpu("Export"), |_: &JobContext| {
        Err(JobError::failed(
            "The disk is full. Free some space and try again.",
        ))
    });

    let failed = finish(&handle);
    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(
        failed.error.as_deref(),
        Some("The disk is full. Free some space and try again.")
    );
    assert!(failed.can_retry);
}

#[test]
fn a_manual_retry_runs_the_same_job_again() {
    let rig = rig(JobLimits::default());
    let runs = Arc::new(AtomicU32::new(0));
    let counter = runs.clone();
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Flaky"), move |_: &JobContext| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(JobError::failed("first run fails"));
            }
            Ok(Value::Null)
        });
    assert_eq!(finish(&handle).state, JobState::Failed);

    rig.queue.retry(handle.id()).unwrap();
    let done = finish(&handle);
    assert_eq!(done.state, JobState::Succeeded);
    assert_eq!(done.error, None);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[test]
fn retryable_failures_are_retried_automatically() {
    let rig = rig(JobLimits::default());
    let policy = RetryPolicy::attempts(3).backoff(Duration::from_millis(5));
    let handle = rig.queue.submit(
        JobSpec::io("Download model").retry(policy),
        |ctx: &JobContext| {
            if ctx.attempt() < 3 {
                return Err(JobError::retryable("connection dropped"));
            }
            Ok(Value::Null)
        },
    );

    let done = finish(&handle);
    assert_eq!(done.state, JobState::Succeeded);
    assert_eq!((done.attempt, done.max_attempts), (3, 3));
}

#[test]
fn retries_stop_at_the_limit_and_skip_errors_that_will_not_go_away() {
    let rig = rig(JobLimits::default());
    let policy = RetryPolicy::attempts(2).backoff(Duration::ZERO);

    let always = rig
        .queue
        .submit(JobSpec::io("Offline").retry(policy), |_: &JobContext| {
            Err(JobError::retryable("no connection"))
        });
    let gave_up = finish(&always);
    assert_eq!((gave_up.state, gave_up.attempt), (JobState::Failed, 2));

    let fatal = rig
        .queue
        .submit(JobSpec::io("Bad file").retry(policy), |_: &JobContext| {
            Err(JobError::failed("not a video file"))
        });
    assert_eq!(finish(&fatal).attempt, 1);
}

#[test]
fn a_retry_waits_for_its_backoff() {
    let rig = rig(JobLimits::default());
    let policy = RetryPolicy::attempts(2).backoff(Duration::from_millis(80));
    let started = Instant::now();
    let handle = rig.queue.submit(
        JobSpec::io("Slow retry").retry(policy),
        |ctx: &JobContext| {
            if ctx.attempt() == 1 {
                Err(JobError::retryable("try later"))
            } else {
                Ok(Value::Null)
            }
        },
    );
    assert_eq!(finish(&handle).state, JobState::Succeeded);
    assert!(started.elapsed() >= Duration::from_millis(80));
}

#[test]
fn the_backoff_doubles_and_is_capped() {
    let policy = RetryPolicy::attempts(10).backoff(Duration::from_secs(1));
    assert_eq!(policy.delay_after(1), Duration::from_secs(1));
    assert_eq!(policy.delay_after(2), Duration::from_secs(2));
    assert_eq!(policy.delay_after(4), Duration::from_secs(8));
    assert_eq!(policy.delay_after(30), Duration::from_secs(60));
}

#[test]
fn a_panic_fails_the_job_without_taking_the_queue_down() {
    let rig = rig(one_at_a_time());
    let crashed = rig
        .queue
        .submit(JobSpec::cpu("Crash"), |_: &JobContext| -> JobResult {
            panic!("index out of range");
        });
    let failed = finish(&crashed);
    assert_eq!(failed.state, JobState::Failed);
    assert!(failed.error.unwrap().contains("index out of range"));
    assert!(!failed.can_retry);

    let next = rig
        .queue
        .submit(JobSpec::cpu("After"), |_: &JobContext| Ok(Value::Null));
    assert_eq!(finish(&next).state, JobState::Succeeded);
}

#[test]
fn each_kind_has_its_own_limit() {
    let rig = rig(JobLimits {
        cpu: 2,
        gpu: 1,
        io: 1,
    });
    let gate = Gate::default();
    let running = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let peak = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);

    let handles: Vec<_> = [JobKind::Cpu; 5]
        .into_iter()
        .chain([JobKind::Gpu; 3])
        .map(|kind| {
            let (gate, running, peak) = (gate.clone(), running.clone(), peak.clone());
            let slot = usize::from(kind == JobKind::Gpu);
            rig.queue
                .submit(JobSpec::new("Work", kind), move |ctx: &JobContext| {
                    let now = running[slot].fetch_add(1, Ordering::SeqCst) + 1;
                    peak[slot].fetch_max(now, Ordering::SeqCst);
                    let passed = gate.pass(ctx);
                    running[slot].fetch_sub(1, Ordering::SeqCst);
                    passed.map(|()| Value::Null)
                })
        })
        .collect();

    // cpu and gpu work side by side, each up to its own limit
    eventually("three jobs are running", || {
        running[0].load(Ordering::SeqCst) == 2 && running[1].load(Ordering::SeqCst) == 1
    });
    gate.open();
    for handle in &handles {
        assert_eq!(finish(handle).state, JobState::Succeeded);
    }
    assert_eq!(peak[0].load(Ordering::SeqCst), 2);
    assert_eq!(peak[1].load(Ordering::SeqCst), 1);
}

fn logging_job(log: &Arc<Mutex<Vec<&'static str>>>, name: &'static str) -> impl Job {
    let log = log.clone();
    move |_: &JobContext| {
        log.lock().push(name);
        Ok(Value::Null)
    }
}

#[test]
fn higher_priority_goes_first_and_equal_priority_keeps_its_order() {
    let rig = rig(one_at_a_time());
    let gate = Gate::default();
    let blocker = rig.queue.submit(JobSpec::cpu("Blocker"), gate.job());
    until_state(&blocker, JobState::Running);

    let log = Arc::new(Mutex::new(Vec::new()));
    let submit = |name: &'static str, priority| {
        rig.queue.submit(
            JobSpec::cpu(name).priority(priority),
            logging_job(&log, name),
        )
    };
    let handles = [
        submit("low", Priority::Low),
        submit("normal a", Priority::Normal),
        submit("urgent", Priority::Urgent),
        submit("normal b", Priority::Normal),
        submit("high", Priority::High),
    ];

    gate.open();
    for handle in &handles {
        finish(handle);
    }
    assert_eq!(
        *log.lock(),
        ["urgent", "high", "normal a", "normal b", "low"]
    );
}

#[test]
fn background_jobs_drop_to_low_priority() {
    let spec = JobSpec::cpu("Thumbnails").background();
    assert!(spec.background);
    assert_eq!(spec.priority, Priority::Low);
}

#[test]
fn waiting_jobs_can_be_reordered_by_hand() {
    let rig = rig(one_at_a_time());
    let gate = Gate::default();
    let blocker = rig.queue.submit(JobSpec::cpu("Blocker"), gate.job());
    until_state(&blocker, JobState::Running);

    let log = Arc::new(Mutex::new(Vec::new()));
    let a = rig.queue.submit(JobSpec::cpu("a"), logging_job(&log, "a"));
    let b = rig.queue.submit(JobSpec::cpu("b"), logging_job(&log, "b"));
    let c = rig.queue.submit(JobSpec::cpu("c"), logging_job(&log, "c"));

    rig.queue.move_to(c.id(), 0).unwrap();
    assert_eq!(rig.queue.overview().pending, [c.id(), a.id(), b.id()]);
    rig.queue.set_priority(b.id(), Priority::High).unwrap();
    assert_eq!(rig.queue.overview().pending, [b.id(), c.id(), a.id()]);
    eventually("the ui is told the new order", || {
        rig.last_order() == Some(vec![b.id(), c.id(), a.id()])
    });

    let running = rig.queue.move_to(blocker.id(), 0);
    assert!(matches!(running, Err(JobQueueError::NotAllowed { .. })));

    gate.open();
    for handle in [&a, &b, &c] {
        finish(handle);
    }
    assert_eq!(*log.lock(), ["b", "c", "a"]);
}

#[test]
fn cancelling_a_queued_job_means_it_never_runs() {
    let rig = rig(one_at_a_time());
    let gate = Gate::default();
    let blocker = rig.queue.submit(JobSpec::cpu("Blocker"), gate.job());
    until_state(&blocker, JobState::Running);

    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let queued = rig
        .queue
        .submit(JobSpec::cpu("Queued"), move |_: &JobContext| {
            flag.store(true, Ordering::SeqCst);
            Ok(Value::Null)
        });
    queued.cancel().unwrap();
    assert_eq!(state(&queued), JobState::Cancelled);

    gate.open();
    finish(&blocker);
    let after = rig
        .queue
        .submit(JobSpec::cpu("After"), |_: &JobContext| Ok(Value::Null));
    finish(&after);
    assert!(!ran.load(Ordering::SeqCst));
    assert!(matches!(
        queued.cancel(),
        Err(JobQueueError::NotAllowed { .. })
    ));
}

#[test]
fn cancelling_a_running_job_stops_it_at_the_next_checkpoint() {
    let rig = rig(one_at_a_time());
    let cleaned_up = Arc::new(AtomicBool::new(false));
    let flag = cleaned_up.clone();
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Long export"), move |ctx: &JobContext| {
            loop {
                if let Err(cancelled) = ctx.sleep(Duration::from_millis(1)) {
                    flag.store(true, Ordering::SeqCst);
                    return Err(cancelled);
                }
            }
        });
    until_state(&handle, JobState::Running);

    handle.cancel().unwrap();
    let done = finish(&handle);
    assert_eq!(done.state, JobState::Cancelled);
    assert!(cleaned_up.load(Ordering::SeqCst));
    assert!(done.can_retry);

    // the slot is free again
    let next = rig
        .queue
        .submit(JobSpec::cpu("Next"), |_: &JobContext| Ok(Value::Null));
    assert_eq!(finish(&next).state, JobState::Succeeded);
}

#[test]
fn an_error_raised_because_of_a_cancel_counts_as_cancelled() {
    let rig = rig(one_at_a_time());
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Encoder"), |ctx: &JobContext| {
            while !ctx.is_cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            Err(JobError::failed("encoder was interrupted"))
        });
    until_state(&handle, JobState::Running);
    handle.cancel().unwrap();
    assert_eq!(finish(&handle).state, JobState::Cancelled);
}

#[test]
fn a_cancel_token_works_from_another_thread() {
    let rig = rig(one_at_a_time());
    let handle = rig.queue.submit(JobSpec::cpu("Pool"), |ctx: &JobContext| {
        let token = ctx.cancel_token();
        let worker = thread::spawn(move || {
            while !token.is_cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
        });
        worker
            .join()
            .map_err(|_| JobError::failed("worker crashed"))?;
        ctx.checkpoint()?;
        Ok(Value::Null)
    });
    until_state(&handle, JobState::Running);
    handle.cancel().unwrap();
    assert_eq!(finish(&handle).state, JobState::Cancelled);
}

#[test]
fn a_paused_job_gives_up_its_slot_and_picks_up_where_it_left_off() {
    let rig = rig(one_at_a_time());
    let gate = Gate::default();
    let steps = Arc::new(AtomicU32::new(0));
    let counter = steps.clone();
    let long_gate = gate.clone();
    let long = rig
        .queue
        .submit(JobSpec::cpu("Long"), move |ctx: &JobContext| {
            counter.fetch_add(1, Ordering::SeqCst);
            long_gate.pass(ctx)?;
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Null)
        });
    until_state(&long, JobState::Running);

    long.pause().unwrap();
    until_state(&long, JobState::Paused);

    // with the long job parked, a short one gets the only slot
    let short = rig
        .queue
        .submit(JobSpec::cpu("Short"), |_: &JobContext| Ok(Value::Null));
    assert_eq!(finish(&short).state, JobState::Succeeded);
    assert_eq!(state(&long), JobState::Paused);
    assert_eq!(steps.load(Ordering::SeqCst), 1);

    gate.open();
    long.resume().unwrap();
    let done = finish(&long);
    assert_eq!(done.state, JobState::Succeeded);
    assert_eq!(done.attempt, 1, "resuming is not a retry");
    assert_eq!(steps.load(Ordering::SeqCst), 2);
}

#[test]
fn a_resumed_job_waits_for_a_slot_ahead_of_newer_work() {
    let rig = rig(one_at_a_time());
    let (first_gate, second_gate) = (Gate::default(), Gate::default());
    let first = rig.queue.submit(JobSpec::cpu("First"), first_gate.job());
    until_state(&first, JobState::Running);
    first.pause().unwrap();
    until_state(&first, JobState::Paused);

    let second = rig.queue.submit(JobSpec::cpu("Second"), second_gate.job());
    until_state(&second, JobState::Running);
    let third = rig
        .queue
        .submit(JobSpec::cpu("Third"), |_: &JobContext| Ok(Value::Null));

    // no slot yet: resumed, but queued behind the running job
    first.resume().unwrap();
    assert_eq!(state(&first), JobState::Queued);
    assert_eq!(rig.queue.overview().pending, [first.id(), third.id()]);

    first_gate.open();
    second_gate.open();
    finish(&second);
    finish(&first);
    let first_done = first.snapshot().unwrap().finished_ms;
    let third_done = finish(&third).finished_ms;
    assert!(first_done <= third_done);
}

#[test]
fn pausing_a_queued_job_holds_it_back_until_resumed() {
    let rig = rig(one_at_a_time());
    let gate = Gate::default();
    let blocker = rig.queue.submit(JobSpec::cpu("Blocker"), gate.job());
    until_state(&blocker, JobState::Running);
    let held = rig
        .queue
        .submit(JobSpec::cpu("Held"), |_: &JobContext| Ok(Value::Null));
    held.pause().unwrap();

    gate.open();
    finish(&blocker);
    let other = rig
        .queue
        .submit(JobSpec::cpu("Other"), |_: &JobContext| Ok(Value::Null));
    finish(&other);
    assert_eq!(state(&held), JobState::Paused);

    held.resume().unwrap();
    assert_eq!(finish(&held).state, JobState::Succeeded);
}

#[test]
fn resuming_before_the_job_reaches_a_checkpoint_just_carries_on() {
    let rig = rig(one_at_a_time());
    let release = Arc::new(AtomicBool::new(false));
    let flag = release.clone();
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Busy"), move |ctx: &JobContext| {
            // deliberately no checkpoint in this loop
            while !flag.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
            ctx.checkpoint()?;
            Ok(Value::Null)
        });
    until_state(&handle, JobState::Running);

    handle.pause().unwrap();
    assert_eq!(state(&handle), JobState::Pausing);
    handle.resume().unwrap();
    assert_eq!(state(&handle), JobState::Running);

    release.store(true, Ordering::SeqCst);
    assert_eq!(finish(&handle).state, JobState::Succeeded);
}

#[test]
fn a_paused_job_can_be_cancelled() {
    let rig = rig(one_at_a_time());
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Paused"), Gate::default().job());
    until_state(&handle, JobState::Running);
    handle.pause().unwrap();
    until_state(&handle, JobState::Paused);

    handle.cancel().unwrap();
    assert_eq!(finish(&handle).state, JobState::Cancelled);
    assert!(rig.queue.overview().pending.is_empty());
}

#[test]
fn pause_all_and_resume_all_cover_running_and_queued_jobs() {
    let rig = rig(one_at_a_time());
    let gate = Gate::default();
    let running = rig.queue.submit(JobSpec::cpu("Running"), gate.job());
    until_state(&running, JobState::Running);
    let queued = rig.queue.submit(JobSpec::cpu("Queued"), gate.job());

    rig.queue.pause_all();
    until_state(&running, JobState::Paused);
    assert_eq!(state(&queued), JobState::Paused);

    gate.open();
    rig.queue.resume_all();
    assert_eq!(finish(&running).state, JobState::Succeeded);
    assert_eq!(finish(&queued).state, JobState::Succeeded);
}

#[test]
fn time_spent_paused_does_not_count_as_running_time() {
    let rig = rig(one_at_a_time());
    let gate = Gate::default();
    let handle = rig.queue.submit(JobSpec::cpu("Timed"), gate.job());
    until_state(&handle, JobState::Running);
    handle.pause().unwrap();
    until_state(&handle, JobState::Paused);

    let before = handle.snapshot().unwrap().elapsed_ms;
    thread::sleep(Duration::from_millis(60));
    assert_eq!(handle.snapshot().unwrap().elapsed_ms, before);

    gate.open();
    handle.resume().unwrap();
    finish(&handle);
}

#[test]
fn raising_a_limit_starts_waiting_jobs() {
    let rig = rig(one_at_a_time());
    let gate = Gate::default();
    let first = rig.queue.submit(JobSpec::cpu("First"), gate.job());
    let second = rig.queue.submit(JobSpec::cpu("Second"), gate.job());
    until_state(&first, JobState::Running);
    assert_eq!(state(&second), JobState::Queued);

    rig.queue.set_limits(JobLimits {
        cpu: 2,
        ..one_at_a_time()
    });
    until_state(&second, JobState::Running);
    assert_eq!(rig.queue.limits().cpu, 2);
    gate.open();
    finish(&first);
    finish(&second);
}

#[test]
fn finished_jobs_can_be_removed_and_the_ui_is_told() {
    let rig = rig(JobLimits::default());
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Done"), |_: &JobContext| Ok(Value::Null));
    finish(&handle);
    let id = handle.id();

    rig.queue.remove(id).unwrap();
    assert!(rig.queue.snapshot(id).is_none());
    assert_eq!(rig.queue.remove(id), Err(JobQueueError::NotFound(id)));
    assert_eq!(handle.wait(), Err(JobQueueError::NotFound(id)));
    eventually("the removal is announced", || {
        rig.events.lock().contains(&JobEvent::Removed { id })
    });
}

#[test]
fn a_running_job_cannot_be_removed() {
    let rig = rig(JobLimits::default());
    let gate = Gate::default();
    let handle = rig.queue.submit(JobSpec::cpu("Busy"), gate.job());
    until_state(&handle, JobState::Running);
    assert!(matches!(
        rig.queue.remove(handle.id()),
        Err(JobQueueError::NotAllowed { .. })
    ));
    assert!(matches!(
        rig.queue.retry(handle.id()),
        Err(JobQueueError::NotAllowed { .. })
    ));
    gate.open();
    finish(&handle);
}

#[test]
fn only_the_newest_finished_jobs_are_kept() {
    let rig = rig_with(JobQueueConfig {
        keep_finished: 3,
        ..JobQueueConfig::default()
    });
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let handle = rig
                .queue
                .submit(JobSpec::cpu("Small"), |_: &JobContext| Ok(Value::Null));
            let _ = handle.wait_timeout(TIMEOUT);
            handle
        })
        .collect();

    eventually("old jobs are dropped", || {
        rig.queue.overview().jobs.len() == 3
    });
    let kept: Vec<_> = rig.queue.overview().jobs.iter().map(|job| job.id).collect();
    assert_eq!(kept, [handles[3].id(), handles[4].id(), handles[5].id()]);

    rig.queue.clear_finished();
    assert!(rig.queue.overview().jobs.is_empty());
}

#[test]
fn an_event_handler_can_call_back_into_the_queue() {
    let bus = EventBus::new();
    let queue = JobQueue::new(bus.clone(), JobQueueConfig::default()).unwrap();
    let cleaner = queue.clone();
    bus.subscribe(move |event: &JobEvent| {
        if let JobEvent::Updated { job } = event
            && job.state == JobState::Succeeded
        {
            let _ = cleaner.remove(job.id);
        }
    })
    .detach();

    let handle = queue.submit(JobSpec::cpu("Self cleaning"), |_: &JobContext| {
        Ok(Value::Null)
    });
    eventually("the handler removed the job", || {
        queue.snapshot(handle.id()).is_none()
    });
}

#[test]
fn waiting_with_a_timeout_gives_up_on_a_job_that_is_still_going() {
    let rig = rig(JobLimits::default());
    let gate = Gate::default();
    let handle = rig.queue.submit(JobSpec::cpu("Slow"), gate.job());
    assert_eq!(handle.wait_timeout(Duration::from_millis(20)), Ok(None));
    gate.open();
    assert_eq!(handle.wait().unwrap().state, JobState::Succeeded);
}

#[test]
fn shutting_down_cancels_everything_and_refuses_new_work() {
    let rig = rig(one_at_a_time());
    let running = rig
        .queue
        .submit(JobSpec::cpu("Running"), Gate::default().job());
    until_state(&running, JobState::Running);
    let queued = rig
        .queue
        .submit(JobSpec::cpu("Queued"), Gate::default().job());

    rig.queue.shut_down();
    assert_eq!(finish(&running).state, JobState::Cancelled);
    assert_eq!(state(&queued), JobState::Cancelled);

    let late = rig
        .queue
        .submit(JobSpec::cpu("Late"), |_: &JobContext| Ok(Value::Null));
    assert_eq!(state(&late), JobState::Cancelled);
}

#[test]
fn dropping_the_queue_stops_its_jobs() {
    let rig = rig(one_at_a_time());
    let handle = rig
        .queue
        .submit(JobSpec::cpu("Orphan"), Gate::default().job());
    until_state(&handle, JobState::Running);
    drop(rig);
    assert_eq!(finish(&handle).state, JobState::Cancelled);
}

#[test]
fn snapshots_serialise_the_way_the_ui_expects() {
    let rig = rig(JobLimits::default());
    let handle = rig
        .queue
        .submit(JobSpec::gpu("Upscale"), |_: &JobContext| Ok(json!(1)));
    let json = serde_json::to_value(finish(&handle)).unwrap();
    assert_eq!(json["state"], "succeeded");
    assert_eq!(json["kind"], "gpu");
    assert_eq!(json["priority"], "normal");
    assert_eq!(json["maxAttempts"], 1);
    assert!(json["elapsedMs"].is_u64());

    let event = serde_json::to_value(JobEvent::Removed { id: handle.id() }).unwrap();
    assert_eq!(event["type"], "removed");
    assert!(event["id"].is_u64());
}
