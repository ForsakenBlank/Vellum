//! the notification centre: finished jobs, restored sessions and anything else the user
//! should hear about after the fact.
//!
//! notifications live in memory for the session. the centre follows job events itself, so
//! every finished or failed job gets a notification however it was started.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::{Event, EventBus};
use crate::jobs::{JobEvent, JobId, JobSnapshot, JobState};
use crate::recent::now_ms;

/// older notifications are dropped beyond this
pub const MAX_NOTIFICATIONS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// what clicking the notification does
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum NotificationAction {
    /// opens a file with the system's default app
    OpenPath {
        path: String,
    },
    /// shows the job in the jobs panel
    ShowJob {
        job: JobId,
    },
    RetryJob {
        job: JobId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Notification {
    #[ts(type = "number")]
    pub id: u64,
    pub level: NotificationLevel,
    pub title: String,
    pub body: Option<String>,
    pub action: Option<NotificationAction>,
    /// label for the action button, eg "Open"
    pub action_label: Option<String>,
    #[ts(type = "number")]
    pub created_at_ms: u64,
    pub read: bool,
}

/// the whole list, newest first, sent on every change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NotificationsChanged {
    pub items: Vec<Notification>,
    pub unread: usize,
}

impl Event for NotificationsChanged {
    const NAME: &'static str = "notifications:changed";
}

/// a notification before it gets an id and a time
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNotification {
    pub level: NotificationLevel,
    pub title: String,
    pub body: Option<String>,
    pub action: Option<(String, NotificationAction)>,
}

impl NewNotification {
    pub fn new(level: NotificationLevel, title: impl Into<String>) -> Self {
        Self {
            level,
            title: title.into(),
            body: None,
            action: None,
        }
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn action(mut self, label: impl Into<String>, action: NotificationAction) -> Self {
        self.action = Some((label.into(), action));
        self
    }
}

#[derive(Default)]
struct Inner {
    items: Vec<Notification>,
    next_id: u64,
    /// the last state seen per job, to spot the moment it finishes
    job_states: HashMap<JobId, JobState>,
}

/// cheap to clone, every clone shares the same list
#[derive(Clone)]
pub struct NotificationCenter {
    inner: Arc<Mutex<Inner>>,
    bus: EventBus,
}

impl NotificationCenter {
    pub fn new(bus: EventBus) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                next_id: 1,
                ..Inner::default()
            })),
            bus,
        }
    }

    pub fn push(&self, new: NewNotification) -> u64 {
        let mut inner = self.inner.lock();
        let id = inner.next_id;
        inner.next_id += 1;
        let (action_label, action) = match new.action {
            Some((label, action)) => (Some(label), Some(action)),
            None => (None, None),
        };
        inner.items.insert(
            0,
            Notification {
                id,
                level: new.level,
                title: new.title,
                body: new.body,
                action,
                action_label,
                created_at_ms: now_ms(),
                read: false,
            },
        );
        inner.items.truncate(MAX_NOTIFICATIONS);
        self.changed(&inner);
        id
    }

    pub fn list(&self) -> NotificationsChanged {
        Self::view(&self.inner.lock())
    }

    pub fn mark_all_read(&self) {
        let mut inner = self.inner.lock();
        if inner.items.iter().all(|n| n.read) {
            return;
        }
        for item in &mut inner.items {
            item.read = true;
        }
        self.changed(&inner);
    }

    pub fn dismiss(&self, id: u64) {
        let mut inner = self.inner.lock();
        let before = inner.items.len();
        inner.items.retain(|n| n.id != id);
        if inner.items.len() != before {
            self.changed(&inner);
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        if !inner.items.is_empty() {
            inner.items.clear();
            self.changed(&inner);
        }
    }

    /// turns a job update into a notification when the job just finished or failed.
    /// cancelled jobs stay quiet, the user asked for that.
    pub fn observe_job(&self, event: &JobEvent) {
        let job = match event {
            JobEvent::Updated { job } => job,
            JobEvent::Removed { id, .. } => {
                self.inner.lock().job_states.remove(id);
                return;
            }
            JobEvent::Order { .. } => return,
        };
        let previous = self.inner.lock().job_states.insert(job.id, job.state);
        if previous == Some(job.state) {
            return;
        }
        if let Some(new) = job_notification(job) {
            self.push(new);
        }
    }

    /// follows the bus on a background thread for as long as the bus lives
    pub fn follow_jobs(&self) {
        let events = self.bus.subscribe::<JobEvent>();
        let center = self.clone();
        let spawned = thread::Builder::new()
            .name("vellum-notify-jobs".into())
            .spawn(move || {
                while let Some(event) = events.recv() {
                    center.observe_job(&event);
                }
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "finished jobs will not show notifications");
        }
    }

    fn view(inner: &Inner) -> NotificationsChanged {
        NotificationsChanged {
            items: inner.items.clone(),
            unread: inner.items.iter().filter(|n| !n.read).count(),
        }
    }

    fn changed(&self, inner: &Inner) {
        self.bus.emit(Self::view(inner));
    }
}

/// the path a job produced, if its result has one
pub fn result_path(job: &JobSnapshot) -> Option<String> {
    job.result
        .as_ref()?
        .get("path")?
        .as_str()
        .map(str::to_owned)
}

fn job_notification(job: &JobSnapshot) -> Option<NewNotification> {
    match job.state {
        JobState::Completed => {
            let new =
                NewNotification::new(NotificationLevel::Success, &job.title).body("Finished.");
            Some(match result_path(job) {
                Some(path) => new.action("Open", NotificationAction::OpenPath { path }),
                None => new.action("Show", NotificationAction::ShowJob { job: job.id }),
            })
        }
        JobState::Failed => {
            let reason = job.error.clone().unwrap_or_else(|| "It failed.".into());
            Some(
                NewNotification::new(NotificationLevel::Error, &job.title)
                    .body(format!("{reason}. Retry it, or check the input file."))
                    .action("Retry", NotificationAction::RetryJob { job: job.id }),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobPriority, JobResource};
    use serde_json::json;

    fn snapshot(id: u64, state: JobState) -> JobSnapshot {
        JobSnapshot {
            id: JobId(id),
            title: format!("Export {id}"),
            resource: JobResource::Cpu,
            priority: JobPriority::Normal,
            state,
            progress: None,
            message: None,
            eta_secs: None,
            elapsed_secs: 0.0,
            attempt: 0,
            max_retries: 0,
            error: None,
            result: None,
            created_at_ms: 0,
            revision: 0,
        }
    }

    fn update(job: JobSnapshot) -> JobEvent {
        JobEvent::Updated { job }
    }

    #[test]
    fn finished_jobs_notify_once() {
        let bus = EventBus::new();
        let changes = bus.subscribe::<NotificationsChanged>();
        let center = NotificationCenter::new(bus);
        center.observe_job(&update(snapshot(1, JobState::Running)));
        assert!(center.list().items.is_empty());

        let mut done = snapshot(1, JobState::Completed);
        done.result = Some(json!({ "path": "/out/clip.gif" }));
        center.observe_job(&update(done.clone()));
        center.observe_job(&update(done));

        let list = center.list();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.unread, 1);
        assert_eq!(list.items[0].level, NotificationLevel::Success);
        assert_eq!(
            list.items[0].action,
            Some(NotificationAction::OpenPath {
                path: "/out/clip.gif".into()
            })
        );
        assert_eq!(list.items[0].action_label.as_deref(), Some("Open"));
        assert_eq!(changes.drain().len(), 1);
    }

    #[test]
    fn failures_offer_a_retry_and_cancels_stay_quiet() {
        let center = NotificationCenter::new(EventBus::new());
        let mut failed = snapshot(2, JobState::Failed);
        failed.error = Some("disk is full".into());
        center.observe_job(&update(failed));
        center.observe_job(&update(snapshot(3, JobState::Cancelled)));
        let list = center.list();
        assert_eq!(list.items.len(), 1);
        assert_eq!(
            list.items[0].body.as_deref(),
            Some("disk is full. Retry it, or check the input file.")
        );
        assert_eq!(
            list.items[0].action,
            Some(NotificationAction::RetryJob { job: JobId(2) })
        );
    }

    #[test]
    fn a_retried_job_can_notify_again() {
        let center = NotificationCenter::new(EventBus::new());
        center.observe_job(&update(snapshot(4, JobState::Failed)));
        center.observe_job(&update(snapshot(4, JobState::Running)));
        center.observe_job(&update(snapshot(4, JobState::Completed)));
        assert_eq!(center.list().items.len(), 2);
    }

    #[test]
    fn read_dismiss_and_clear() {
        let center = NotificationCenter::new(EventBus::new());
        let first = center.push(NewNotification::new(NotificationLevel::Info, "One"));
        center.push(NewNotification::new(NotificationLevel::Warning, "Two").body("details"));
        assert_eq!(center.list().items[0].title, "Two");

        center.mark_all_read();
        assert_eq!(center.list().unread, 0);
        center.dismiss(first);
        assert_eq!(center.list().items.len(), 1);
        center.clear();
        assert!(center.list().items.is_empty());
    }

    #[test]
    fn the_list_is_capped() {
        let center = NotificationCenter::new(EventBus::new());
        for n in 0..(MAX_NOTIFICATIONS + 10) {
            center.push(NewNotification::new(NotificationLevel::Info, n.to_string()));
        }
        let list = center.list();
        assert_eq!(list.items.len(), MAX_NOTIFICATIONS);
        assert_eq!(list.items[0].title, (MAX_NOTIFICATIONS + 9).to_string());
    }

    #[test]
    fn follows_jobs_on_the_bus() {
        let bus = EventBus::new();
        let center = NotificationCenter::new(bus.clone());
        let changes = bus.subscribe::<NotificationsChanged>();
        center.follow_jobs();
        bus.emit(update(snapshot(5, JobState::Completed)));
        let change = changes
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(change.items[0].title, "Export 5");
    }
}
