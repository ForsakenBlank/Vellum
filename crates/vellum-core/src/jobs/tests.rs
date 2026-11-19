use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;

use super::*;
use crate::events::{EventBus, VellumEvent};

const WAIT: Duration = Duration::from_secs(5);

fn queue_with(cpu: usize, gpu: usize) -> (JobQueue, Receiver<VellumEvent>) {
    let bus = EventBus::new();
    let rx = bus.subscribe();
    let mut config = QueueConfig::default();
    config.limits.insert(JobKind::Cpu, cpu);
    config.limits.insert(JobKind::Gpu, gpu);
    config.progress_interval = Duration::ZERO;
    (JobQueue::new(config, bus), rx)
}

/// a job that waits on a channel so tests decide exactly when it finishes
fn gated(gate: Receiver<()>) -> impl Job {
    move |ctx: &JobContext| loop {
        ctx.checkpoint()?;
        match gate.recv_timeout(Duration::from_millis(5)) {
            Ok(()) => return Ok(None),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(_) => return Err(JobError::failed("gate dropped")),
        }
    }
}

fn gate() -> (Sender<()>, Receiver<()>) {
    crossbeam_channel::unbounded()
}

fn wait_for_status(queue: &JobQueue, id: JobId, status: JobStatus) {
    let deadline = std::time::Instant::now() + WAIT;
    while queue.get(id).map(|j| j.status) != Some(status) {
        assert!(
            std::time::Instant::now() < deadline,
            "job {id} never reached {status:?}, it is {:?}",
            queue.get(id).map(|j| j.status)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn job_runs_and_returns_its_result() {
    let (queue, _rx) = queue_with(1, 1);
    let id = queue.submit(JobSpec::new("hello", JobKind::Cpu), |ctx: &JobContext| {
        ctx.set_progress(0.5);
        Ok(Some(serde_json::json!({ "path": "out.gif" })))
    });
    assert_eq!(queue.wait(id, WAIT), Some(JobStatus::Completed));
    let info = queue.get(id).unwrap();
    assert_eq!(info.progress, 1.0);
    assert_eq!(info.attempts, 1);
    assert_eq!(info.result.unwrap()["path"], "out.gif");
}

#[test]
fn limits_are_per_kind() {
    let (queue, _rx) = queue_with(1, 1);
    let (cpu_tx, cpu_rx) = gate();
    let (gpu_tx, gpu_rx) = gate();
    let cpu_a = queue.submit(JobSpec::new("a", JobKind::Cpu), gated(cpu_rx.clone()));
    let cpu_b = queue.submit(JobSpec::new("b", JobKind::Cpu), gated(cpu_rx));
    let gpu = queue.submit(JobSpec::new("g", JobKind::Gpu), gated(gpu_rx));

    wait_for_status(&queue, cpu_a, JobStatus::Running);
    wait_for_status(&queue, gpu, JobStatus::Running);
    assert_eq!(queue.get(cpu_b).unwrap().status, JobStatus::Queued);

    cpu_tx.send(()).unwrap();
    queue.wait(cpu_a, WAIT);
    wait_for_status(&queue, cpu_b, JobStatus::Running);
    cpu_tx.send(()).unwrap();
    gpu_tx.send(()).unwrap();
    assert_eq!(queue.wait(cpu_b, WAIT), Some(JobStatus::Completed));
    assert_eq!(queue.wait(gpu, WAIT), Some(JobStatus::Completed));
}

#[test]
fn higher_priority_starts_first_then_oldest_first() {
    let (queue, _rx) = queue_with(1, 1);
    let order = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = gate();
    let blocker = queue.submit(JobSpec::new("blocker", JobKind::Cpu), gated(rx));
    wait_for_status(&queue, blocker, JobStatus::Running);

    let mut ids = Vec::new();
    for (name, priority) in [
        ("low", Priority::Low),
        ("normal-1", Priority::Normal),
        ("high", Priority::High),
        ("normal-2", Priority::Normal),
    ] {
        let order = Arc::clone(&order);
        let spec = JobSpec::new(name, JobKind::Cpu).priority(priority);
        ids.push(queue.submit(spec, move |_: &JobContext| {
            order.lock().push(name);
            Ok(None)
        }));
    }
    // bump the low job above normal while it waits
    queue.set_priority(ids[0], Priority::High).unwrap();

    tx.send(()).unwrap();
    for id in ids {
        assert_eq!(queue.wait(id, WAIT), Some(JobStatus::Completed));
    }
    assert_eq!(*order.lock(), ["low", "high", "normal-1", "normal-2"]);
}

#[test]
fn cancel_queued_and_running_jobs() {
    let (queue, _rx) = queue_with(1, 1);
    let (_tx, rx) = gate();
    let running = queue.submit(JobSpec::new("running", JobKind::Cpu), gated(rx.clone()));
    let queued = queue.submit(JobSpec::new("queued", JobKind::Cpu), gated(rx));
    wait_for_status(&queue, running, JobStatus::Running);

    queue.cancel(queued).unwrap();
    assert_eq!(queue.get(queued).unwrap().status, JobStatus::Cancelled);

    queue.cancel(running).unwrap();
    assert_eq!(queue.wait(running, WAIT), Some(JobStatus::Cancelled));
    assert!(matches!(
        queue.cancel(running),
        Err(JobQueueError::InvalidState { .. })
    ));
    assert_eq!(queue.cancel(999), Err(JobQueueError::NotFound(999)));
}

#[test]
fn pause_blocks_at_the_next_checkpoint_and_resume_continues() {
    let (queue, _rx) = queue_with(1, 1);
    let steps = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = gate();
    let counter = Arc::clone(&steps);
    let id = queue.submit(
        JobSpec::new("steps", JobKind::Cpu),
        move |ctx: &JobContext| {
            for _ in 0..1000 {
                ctx.checkpoint()?;
                counter.fetch_add(1, Ordering::SeqCst);
                // wait for the test to let each step through
                if rx.recv_timeout(WAIT).is_err() {
                    return Err(JobError::failed("test gave up"));
                }
            }
            Ok(None)
        },
    );
    wait_for_status(&queue, id, JobStatus::Running);

    queue.pause(id).unwrap();
    // let the in flight step finish, after that the job must sit at the checkpoint
    tx.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let frozen = steps.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        steps.load(Ordering::SeqCst),
        frozen,
        "paused job kept working"
    );
    assert_eq!(queue.get(id).unwrap().status, JobStatus::Paused);

    queue.resume(id).unwrap();
    for _ in 0..1000 {
        let _ = tx.send(());
    }
    assert_eq!(queue.wait(id, WAIT), Some(JobStatus::Completed));
}

#[test]
fn cancel_wakes_a_paused_job() {
    let (queue, _rx) = queue_with(1, 1);
    let (_tx, rx) = gate();
    let id = queue.submit(JobSpec::new("paused", JobKind::Cpu), gated(rx));
    wait_for_status(&queue, id, JobStatus::Running);
    queue.pause(id).unwrap();
    queue.cancel(id).unwrap();
    assert_eq!(queue.wait(id, WAIT), Some(JobStatus::Cancelled));
}

#[test]
fn paused_queued_job_does_not_start_until_resumed() {
    let (queue, _rx) = queue_with(1, 1);
    let (tx, rx) = gate();
    let blocker = queue.submit(JobSpec::new("blocker", JobKind::Cpu), gated(rx));
    let waiting = queue.submit(JobSpec::new("waiting", JobKind::Cpu), |_: &JobContext| {
        Ok(None)
    });
    queue.pause(waiting).unwrap();

    tx.send(()).unwrap();
    queue.wait(blocker, WAIT);
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(queue.get(waiting).unwrap().status, JobStatus::Paused);

    queue.resume(waiting).unwrap();
    assert_eq!(queue.wait(waiting, WAIT), Some(JobStatus::Completed));
}

#[test]
fn failures_auto_retry_then_can_be_retried_by_hand() {
    let (queue, _rx) = queue_with(1, 1);
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&runs);
    let spec = JobSpec::new("flaky", JobKind::Io).max_attempts(2);
    let id = queue.submit(spec, move |_: &JobContext| {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err(JobError::failed(format!("attempt {n} broke")))
        } else {
            Ok(None)
        }
    });

    assert_eq!(queue.wait(id, WAIT), Some(JobStatus::Failed));
    let info = queue.get(id).unwrap();
    assert_eq!(info.attempts, 2);
    assert_eq!(info.error.as_deref(), Some("attempt 1 broke"));

    queue.retry(id).unwrap();
    assert_eq!(queue.wait(id, WAIT), Some(JobStatus::Completed));
    assert_eq!(queue.get(id).unwrap().attempts, 3);
    assert!(queue.retry(id).is_err(), "completed jobs cannot be retried");
}

#[test]
fn a_panicking_job_fails_without_taking_the_queue_down() {
    let (queue, _rx) = queue_with(1, 1);
    let id = queue.submit(JobSpec::new("boom", JobKind::Cpu), |_: &JobContext| {
        panic!("decoder exploded")
    });
    assert_eq!(queue.wait(id, WAIT), Some(JobStatus::Failed));
    let error = queue.get(id).unwrap().error.unwrap();
    assert!(error.contains("decoder exploded"), "{error}");

    let after = queue.submit(JobSpec::new("after", JobKind::Cpu), |_: &JobContext| {
        Ok(None)
    });
    assert_eq!(queue.wait(after, WAIT), Some(JobStatus::Completed));
}

#[test]
fn progress_and_status_events_reach_the_bus_in_order() {
    let (queue, rx) = queue_with(1, 1);
    let id = queue.submit(JobSpec::new("events", JobKind::Cpu), |ctx: &JobContext| {
        ctx.set_progress(0.25);
        ctx.set_message("halfway");
        ctx.set_progress(0.5);
        Ok(None)
    });
    queue.wait(id, WAIT);

    let mut statuses = Vec::new();
    let mut progress = Vec::new();
    while let Ok(event) = rx.recv_timeout(Duration::from_millis(500)) {
        match event {
            VellumEvent::Job(JobEvent::Changed { job }) => statuses.push(job.status),
            VellumEvent::Job(JobEvent::Progress { progress: p, .. }) => progress.push(p),
            _ => {}
        }
        if statuses.last() == Some(&JobStatus::Completed) {
            break;
        }
    }
    assert_eq!(
        statuses,
        [JobStatus::Queued, JobStatus::Running, JobStatus::Completed]
    );
    assert_eq!(progress, [0.25, 0.25, 0.5]);
}

#[test]
fn progress_events_are_throttled() {
    let bus = EventBus::new();
    let rx = bus.subscribe();
    let config = QueueConfig {
        progress_interval: Duration::from_secs(60),
        ..QueueConfig::default()
    };
    let queue = JobQueue::new(config, bus);
    let id = queue.submit(JobSpec::new("chatty", JobKind::Cpu), |ctx: &JobContext| {
        for i in 0..1000 {
            ctx.set_progress(i as f32 / 1000.0);
        }
        Ok(None)
    });
    queue.wait(id, WAIT);
    drop(queue);
    let progress_events = rx
        .try_iter()
        .filter(|e| matches!(e, VellumEvent::Job(JobEvent::Progress { .. })))
        .count();
    assert_eq!(progress_events, 1, "only the first update gets through");
}

#[test]
fn eta_is_estimated_from_progress() {
    let (queue, _rx) = queue_with(1, 1);
    let (tx, rx) = gate();
    let id = queue.submit(
        JobSpec::new("eta", JobKind::Cpu),
        move |ctx: &JobContext| {
            std::thread::sleep(Duration::from_millis(40));
            ctx.set_progress(0.5);
            rx.recv_timeout(WAIT).ok();
            Ok(None)
        },
    );
    let deadline = std::time::Instant::now() + WAIT;
    let eta = loop {
        if let Some(eta) = queue.get(id).and_then(|j| j.eta_secs) {
            break eta;
        }
        assert!(std::time::Instant::now() < deadline, "no eta reported");
        std::thread::sleep(Duration::from_millis(2));
    };
    // half done after roughly 40ms means roughly 40ms left
    assert!(eta > 0.02 && eta < 2.0, "eta was {eta}");
    tx.send(()).unwrap();
    queue.wait(id, WAIT);
    assert_eq!(queue.get(id).unwrap().eta_secs, None);
}

#[test]
fn finished_jobs_can_be_removed() {
    let (queue, rx) = queue_with(1, 1);
    let (_tx, gate_rx) = gate();
    let done = queue.submit(JobSpec::new("done", JobKind::Cpu), |_: &JobContext| {
        Ok(None)
    });
    let busy = queue.submit(JobSpec::new("busy", JobKind::Gpu), gated(gate_rx));
    queue.wait(done, WAIT);
    wait_for_status(&queue, busy, JobStatus::Running);

    assert!(queue.remove(busy).is_err());
    queue.clear_finished();
    assert!(queue.get(done).is_none());
    assert_eq!(queue.list().len(), 1);
    drop(queue);
    assert!(rx
        .try_iter()
        .any(|e| e == VellumEvent::Job(JobEvent::Removed { id: done })));
}

#[test]
fn raising_a_limit_starts_waiting_jobs() {
    let (queue, _rx) = queue_with(1, 1);
    let (tx, rx) = gate();
    let a = queue.submit(JobSpec::new("a", JobKind::Cpu), gated(rx.clone()));
    let b = queue.submit(JobSpec::new("b", JobKind::Cpu), gated(rx));
    wait_for_status(&queue, a, JobStatus::Running);
    queue.set_limit(JobKind::Cpu, 2);
    wait_for_status(&queue, b, JobStatus::Running);
    tx.send(()).unwrap();
    tx.send(()).unwrap();
}

#[test]
fn scheduling_pause_holds_new_jobs() {
    let (queue, _rx) = queue_with(1, 1);
    queue.set_scheduling_paused(true);
    let id = queue.submit(JobSpec::new("held", JobKind::Cpu), |_: &JobContext| {
        Ok(None)
    });
    assert_eq!(queue.wait(id, Duration::from_millis(30)), None);
    queue.set_scheduling_paused(false);
    assert_eq!(queue.wait(id, WAIT), Some(JobStatus::Completed));
}

#[test]
fn shutdown_cancels_running_jobs_and_returns() {
    let (queue, _rx) = queue_with(1, 1);
    let (_tx, rx) = gate();
    let running = queue.submit(JobSpec::new("running", JobKind::Cpu), gated(rx.clone()));
    let queued = queue.submit(JobSpec::new("queued", JobKind::Cpu), gated(rx));
    wait_for_status(&queue, running, JobStatus::Running);
    queue.shutdown();
    assert_eq!(queue.get(running).unwrap().status, JobStatus::Cancelled);
    assert_eq!(queue.get(queued).unwrap().status, JobStatus::Cancelled);
}

#[test]
fn job_info_serialises_for_the_ui() {
    let (queue, _rx) = queue_with(1, 1);
    queue.set_scheduling_paused(true);
    let id = queue.submit(
        JobSpec::new("Export clip", JobKind::Gpu).priority(Priority::High),
        |_: &JobContext| Ok(None),
    );
    let json = serde_json::to_value(queue.get(id).unwrap()).unwrap();
    assert_eq!(json["kind"], "gpu");
    assert_eq!(json["priority"], "high");
    assert_eq!(json["status"], "queued");
    assert_eq!(json["etaSecs"], serde_json::Value::Null);
}
