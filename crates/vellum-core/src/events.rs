//! typed event bus.
//!
//! the core publishes [`VellumEvent`]s, the desktop app forwards every one of them to the webview
//! on a single channel, and the ui gets the same types through the generated typescript bindings.

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::history::HistoryState;
use crate::jobs::JobEvent;
use crate::settings::Settings;

/// name of the tauri event every [`VellumEvent`] is sent on.
pub const UI_EVENT_CHANNEL: &str = "vellum://event";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "topic", content = "payload", rename_all = "camelCase")]
#[ts(export)]
pub enum VellumEvent {
    Job(JobEvent),
    SettingsChanged(Settings),
    HistoryChanged {
        document: String,
        state: HistoryState,
    },
}

type Callback<E> = Arc<dyn Fn(&E) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

struct Inner<E> {
    next_id: u64,
    channels: Vec<Sender<E>>,
    callbacks: Vec<(SubscriptionId, Callback<E>)>,
}

/// cheap to clone, every clone talks to the same subscribers.
pub struct EventBus<E> {
    inner: Arc<Mutex<Inner<E>>>,
}

impl<E> Clone for EventBus<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<E: Clone + Send + 'static> Default for EventBus<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Clone + Send + 'static> EventBus<E> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                next_id: 0,
                channels: Vec::new(),
                callbacks: Vec::new(),
            })),
        }
    }

    /// sends `event` to every subscriber. callbacks run on the calling thread, after the bus lock
    /// is released, so a callback may publish or subscribe without deadlocking.
    pub fn publish(&self, event: E) {
        let callbacks: Vec<Callback<E>> = {
            let mut inner = self.inner.lock();
            // dropped receivers fail to send, which is how channel subscribers unsubscribe
            inner.channels.retain(|tx| tx.send(event.clone()).is_ok());
            inner
                .callbacks
                .iter()
                .map(|(_, cb)| Arc::clone(cb))
                .collect()
        };
        for cb in callbacks {
            cb(&event);
        }
    }

    /// returns a receiver that gets every event from now on. drop it to unsubscribe.
    pub fn subscribe(&self) -> Receiver<E> {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.inner.lock().channels.push(tx);
        rx
    }

    pub fn on(&self, callback: impl Fn(&E) + Send + Sync + 'static) -> SubscriptionId {
        let mut inner = self.inner.lock();
        let id = SubscriptionId(inner.next_id);
        inner.next_id += 1;
        inner.callbacks.push((id, Arc::new(callback)));
        id
    }

    pub fn off(&self, id: SubscriptionId) -> bool {
        let mut inner = self.inner.lock();
        let before = inner.callbacks.len();
        inner.callbacks.retain(|(cb_id, _)| *cb_id != id);
        inner.callbacks.len() != before
    }

    pub fn subscriber_count(&self) -> usize {
        let inner = self.inner.lock();
        inner.channels.len() + inner.callbacks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn channel_subscribers_get_every_event_in_order() {
        let bus = EventBus::new();
        let rx = bus.subscribe();
        bus.publish(1);
        bus.publish(2);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn dropped_receivers_are_pruned() {
        let bus = EventBus::<u8>::new();
        let rx = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);
        drop(rx);
        bus.publish(0);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn callbacks_can_publish_without_deadlocking() {
        let bus = EventBus::<u32>::new();
        let seen = Arc::new(AtomicUsize::new(0));
        let inner_bus = bus.clone();
        let counter = Arc::clone(&seen);
        let id = bus.on(move |n| {
            counter.fetch_add(1, Ordering::SeqCst);
            if *n < 3 {
                inner_bus.publish(n + 1);
            }
        });
        bus.publish(0);
        assert_eq!(seen.load(Ordering::SeqCst), 4);

        assert!(bus.off(id));
        assert!(!bus.off(id));
        bus.publish(9);
        assert_eq!(seen.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn events_serialise_with_topic_and_payload() {
        let event = VellumEvent::HistoryChanged {
            document: "doc-1".into(),
            state: HistoryState::default(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["topic"], "historyChanged");
        assert_eq!(json["payload"]["document"], "doc-1");
        assert_eq!(json["payload"]["state"]["canUndo"], false);

        let back: VellumEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}
