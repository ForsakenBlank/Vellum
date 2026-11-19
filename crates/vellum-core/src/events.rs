//! the typed event bus.
//!
//! rust code emits plain structs that implement [`Event`]. rust subscribers get them over a
//! channel, fully typed. sinks (the tauri bridge) get them as `(name, json)` and forward them
//! to the ui, where `ui/src/lib/events.ts` maps each name back to its generated type.
//!
//! the ui can also send events back, but only ones registered with
//! [`EventBus::accept_from_ui`]. their payloads are checked by deserialising into the real
//! type, so a bad payload is an error at the boundary instead of a panic somewhere else.
//! user actions do not travel this way, they are commands.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// a message that can travel on the bus and out to the ui.
pub trait Event: Serialize + Clone + Send + Sync + 'static {
    /// name used on the ui side. tauri only allows letters, digits, `-`, `/`, `:` and `_`,
    /// so names look like `job:changed`.
    const NAME: &'static str;
}

/// receives every event as json, used to forward events to the ui.
pub trait EventSink: Send + Sync {
    fn deliver(&self, name: &'static str, payload: &serde_json::Value);
}

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("the ui is not allowed to send `{0}`")]
    NotAccepted(String),
    #[error("bad payload for `{name}`: {source}")]
    InvalidPayload {
        name: String,
        #[source]
        source: serde_json::Error,
    },
}

type InboundHandler =
    Box<dyn Fn(&EventBus, serde_json::Value) -> Result<(), EventError> + Send + Sync>;

#[derive(Default)]
struct Inner {
    subscribers: Mutex<HashMap<TypeId, Vec<Box<dyn Any + Send>>>>,
    sinks: RwLock<Vec<Arc<dyn EventSink>>>,
    inbound: RwLock<HashMap<&'static str, InboundHandler>>,
}

/// cheap to clone, every clone talks to the same bus.
#[derive(Clone, Default)]
pub struct EventBus {
    inner: Arc<Inner>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// sends an event to rust subscribers and every sink
    pub fn emit<E: Event>(&self, event: E) {
        let sinks = self.inner.sinks.read().clone();
        if !sinks.is_empty() {
            match serde_json::to_value(&event) {
                Ok(payload) => {
                    for sink in &sinks {
                        sink.deliver(E::NAME, &payload);
                    }
                }
                Err(err) => tracing::error!(event = E::NAME, %err, "could not serialise event"),
            }
        }
        self.deliver_local(event);
    }

    fn deliver_local<E: Event>(&self, event: E) {
        let mut subscribers = self.inner.subscribers.lock();
        let Some(list) = subscribers.get_mut(&TypeId::of::<E>()) else {
            return;
        };
        // drop subscribers whose receiver has gone away
        list.retain(|sender| match sender.downcast_ref::<Sender<E>>() {
            Some(sender) => sender.send(event.clone()).is_ok(),
            None => false,
        });
    }

    /// returns a typed receiver for one event type. events queue up until read, so keep
    /// draining it or drop it.
    pub fn subscribe<E: Event>(&self) -> Subscription<E> {
        let (tx, rx) = mpsc::channel();
        self.inner
            .subscribers
            .lock()
            .entry(TypeId::of::<E>())
            .or_default()
            .push(Box::new(tx));
        Subscription { rx }
    }

    pub fn add_sink(&self, sink: Arc<dyn EventSink>) {
        self.inner.sinks.write().push(sink);
    }

    /// allows the ui to send this event type to rust subscribers
    pub fn accept_from_ui<E: Event + DeserializeOwned>(&self) {
        let handler: InboundHandler = Box::new(|bus, payload| {
            let event: E =
                serde_json::from_value(payload).map_err(|source| EventError::InvalidPayload {
                    name: E::NAME.to_owned(),
                    source,
                })?;
            // ui events only go to rust, sending them back out would echo
            bus.deliver_local(event);
            Ok(())
        });
        self.inner.inbound.write().insert(E::NAME, handler);
    }

    /// entry point for events coming from the ui
    pub fn dispatch_from_ui(
        &self,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<(), EventError> {
        let inbound = self.inner.inbound.read();
        let handler = inbound
            .get(name)
            .ok_or_else(|| EventError::NotAccepted(name.to_owned()))?;
        handler(self, payload)
    }
}

impl fmt::Debug for EventBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventBus")
            .field("sinks", &self.inner.sinks.read().len())
            .finish_non_exhaustive()
    }
}

/// a typed stream of one event type from the bus.
#[derive(Debug)]
pub struct Subscription<E> {
    rx: Receiver<E>,
}

impl<E> Subscription<E> {
    pub fn try_recv(&self) -> Option<E> {
        self.rx.try_recv().ok()
    }

    /// blocks until the next event. only returns `None` once the bus is gone.
    pub fn recv(&self) -> Option<E> {
        self.rx.recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<E> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// everything queued right now, without waiting
    pub fn drain(&self) -> Vec<E> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Ping {
        n: u32,
    }

    impl Event for Ping {
        const NAME: &'static str = "test:ping";
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Pong;

    impl Event for Pong {
        const NAME: &'static str = "test:pong";
    }

    #[derive(Default)]
    struct RecordingSink {
        seen: Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl EventSink for RecordingSink {
        fn deliver(&self, name: &'static str, payload: &serde_json::Value) {
            self.seen.lock().push((name.to_owned(), payload.clone()));
        }
    }

    #[test]
    fn subscribers_get_only_their_type() {
        let bus = EventBus::new();
        let pings = bus.subscribe::<Ping>();
        let pongs = bus.subscribe::<Pong>();
        bus.emit(Ping { n: 1 });
        bus.emit(Ping { n: 2 });
        assert_eq!(pings.drain(), [Ping { n: 1 }, Ping { n: 2 }]);
        assert!(pongs.try_recv().is_none());
    }

    #[test]
    fn every_subscriber_gets_a_copy() {
        let bus = EventBus::new();
        let a = bus.subscribe::<Ping>();
        let b = bus.clone().subscribe::<Ping>();
        bus.emit(Ping { n: 7 });
        assert_eq!(a.try_recv(), Some(Ping { n: 7 }));
        assert_eq!(b.recv_timeout(Duration::from_secs(1)), Some(Ping { n: 7 }));
    }

    #[test]
    fn dropped_subscribers_are_cleaned_up() {
        let bus = EventBus::new();
        let sub = bus.subscribe::<Ping>();
        drop(sub);
        bus.emit(Ping { n: 1 });
        let count = bus.inner.subscribers.lock()[&TypeId::of::<Ping>()].len();
        assert_eq!(count, 0);
    }

    #[test]
    fn sinks_receive_json_with_the_event_name() {
        let bus = EventBus::new();
        let sink = Arc::new(RecordingSink::default());
        bus.add_sink(sink.clone());
        bus.emit(Ping { n: 3 });
        let seen = sink.seen.lock();
        assert_eq!(
            seen.as_slice(),
            [("test:ping".to_owned(), json!({ "n": 3 }))]
        );
    }

    #[test]
    fn ui_events_must_be_accepted_and_valid() {
        let bus = EventBus::new();
        let sink = Arc::new(RecordingSink::default());
        bus.add_sink(sink.clone());
        let pings = bus.subscribe::<Ping>();

        let err = bus
            .dispatch_from_ui("test:ping", json!({ "n": 1 }))
            .unwrap_err();
        assert!(matches!(err, EventError::NotAccepted(_)));

        bus.accept_from_ui::<Ping>();
        bus.dispatch_from_ui("test:ping", json!({ "n": 5 }))
            .unwrap();
        assert_eq!(pings.try_recv(), Some(Ping { n: 5 }));

        let err = bus
            .dispatch_from_ui("test:ping", json!({ "n": "five" }))
            .unwrap_err();
        assert!(matches!(err, EventError::InvalidPayload { .. }));

        // events from the ui are not echoed back out
        assert!(sink.seen.lock().is_empty());
    }

    #[test]
    fn works_across_threads() {
        let bus = EventBus::new();
        let sub = bus.subscribe::<Ping>();
        let handles: Vec<_> = (0..4)
            .map(|n| {
                let bus = bus.clone();
                std::thread::spawn(move || bus.emit(Ping { n }))
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let mut got: Vec<_> = sub.drain().into_iter().map(|p| p.n).collect();
        got.sort_unstable();
        assert_eq!(got, [0, 1, 2, 3]);
    }
}
