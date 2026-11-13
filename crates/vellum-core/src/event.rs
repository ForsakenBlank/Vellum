//! the event bus.
//!
//! rust code publishes typed events and subscribes to them by type. a tap sees
//! every event in serialised form, which is how the desktop app forwards them
//! to the ui over one channel. the ui side has a matching typed map generated
//! from the same rust types, so both ends agree on names and payloads.
//!
//! handlers run on the publishing thread, in the order they subscribed. keep
//! them short: hand real work to a job or a channel.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// a message on the bus. the name is what the ui subscribes to.
pub trait Event: Serialize + Send + Sync + 'static {
    /// unique, dotted and lowercase, such as `jobs` or `settings.changed`
    const NAME: &'static str;
}

/// an event in the form that crosses to the ui.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Envelope {
    pub name: String,
    /// counts up by one per published event, so the ui can spot gaps
    pub seq: u64,
    pub payload: Value,
}

type Handler = Arc<dyn Fn(&dyn Any) + Send + Sync>;
type Tap = Arc<dyn Fn(&Envelope) + Send + Sync>;

#[derive(Default)]
struct Inner {
    handlers: RwLock<HashMap<TypeId, Vec<(u64, Handler)>>>,
    taps: RwLock<Vec<(u64, Tap)>>,
    next_subscription: AtomicU64,
    seq: AtomicU64,
}

#[derive(Clone, Default)]
pub struct EventBus {
    inner: Arc<Inner>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish<E: Event>(&self, event: E) {
        // copy the handler list out so a handler can publish or subscribe without deadlocking
        let handlers: Vec<Handler> = self
            .inner
            .handlers
            .read()
            .get(&TypeId::of::<E>())
            .map(|list| list.iter().map(|(_, handler)| handler.clone()).collect())
            .unwrap_or_default();
        let taps: Vec<Tap> = self
            .inner
            .taps
            .read()
            .iter()
            .map(|(_, tap)| tap.clone())
            .collect();

        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed);
        for handler in &handlers {
            handler(&event);
        }

        if taps.is_empty() {
            return;
        }
        match serde_json::to_value(&event) {
            Ok(payload) => {
                let envelope = Envelope {
                    name: E::NAME.to_owned(),
                    seq,
                    payload,
                };
                for tap in &taps {
                    tap(&envelope);
                }
            }
            Err(error) => {
                tracing::error!(event = E::NAME, %error, "could not serialise an event for the ui");
            }
        }
    }

    /// calls `handler` for every `E` published from now on, until the returned
    /// subscription is dropped
    pub fn subscribe<E: Event>(
        &self,
        handler: impl Fn(&E) + Send + Sync + 'static,
    ) -> Subscription {
        let id = self.next_id();
        let erased: Handler = Arc::new(move |event: &dyn Any| {
            if let Some(event) = event.downcast_ref::<E>() {
                handler(event);
            }
        });
        let key = TypeId::of::<E>();
        self.inner
            .handlers
            .write()
            .entry(key)
            .or_default()
            .push((id, erased));
        Subscription {
            bus: Arc::downgrade(&self.inner),
            slot: Slot::Typed(key),
            id,
        }
    }

    /// sees every event of every type, already serialised
    pub fn tap(&self, tap: impl Fn(&Envelope) + Send + Sync + 'static) -> Subscription {
        let id = self.next_id();
        self.inner.taps.write().push((id, Arc::new(tap)));
        Subscription {
            bus: Arc::downgrade(&self.inner),
            slot: Slot::Tap,
            id,
        }
    }

    fn next_id(&self) -> u64 {
        self.inner.next_subscription.fetch_add(1, Ordering::Relaxed)
    }
}

enum Slot {
    Typed(TypeId),
    Tap,
}

/// keeps a handler registered. dropping it unsubscribes.
///
/// a publish already in flight on another thread can still reach the handler
/// once after the drop.
#[must_use = "dropping a subscription unsubscribes straight away"]
pub struct Subscription {
    bus: Weak<Inner>,
    slot: Slot,
    id: u64,
}

impl Subscription {
    /// keeps the handler registered for as long as the bus lives
    pub fn detach(mut self) {
        self.bus = Weak::new();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let Some(inner) = self.bus.upgrade() else {
            return;
        };
        match self.slot {
            Slot::Typed(key) => {
                let mut handlers = inner.handlers.write();
                if let Some(list) = handlers.get_mut(&key) {
                    list.retain(|(id, _)| *id != self.id);
                    if list.is_empty() {
                        handlers.remove(&key);
                    }
                }
            }
            Slot::Tap => inner.taps.write().retain(|(id, _)| *id != self.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;
    use serde_json::json;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize)]
    struct Ping {
        n: u32,
    }

    impl Event for Ping {
        const NAME: &'static str = "test.ping";
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    struct Pong;

    impl Event for Pong {
        const NAME: &'static str = "test.pong";
    }

    fn collector<T: Clone + Send + 'static>() -> (Arc<Mutex<Vec<T>>>, impl Fn(&T) + Send + Sync) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        (seen, move |item: &T| sink.lock().push(item.clone()))
    }

    #[test]
    fn subscribers_only_get_their_own_event_type() {
        let bus = EventBus::new();
        let (pings, on_ping) = collector::<Ping>();
        let (pongs, on_pong) = collector::<Pong>();
        let _a = bus.subscribe(on_ping);
        let _b = bus.subscribe(on_pong);

        bus.publish(Ping { n: 1 });
        bus.publish(Pong);
        bus.publish(Ping { n: 2 });

        assert_eq!(*pings.lock(), [Ping { n: 1 }, Ping { n: 2 }]);
        assert_eq!(*pongs.lock(), [Pong]);
    }

    #[test]
    fn dropping_a_subscription_unsubscribes() {
        let bus = EventBus::new();
        let (pings, on_ping) = collector::<Ping>();
        let subscription = bus.subscribe(on_ping);
        bus.publish(Ping { n: 1 });
        drop(subscription);
        bus.publish(Ping { n: 2 });
        assert_eq!(*pings.lock(), [Ping { n: 1 }]);
    }

    #[test]
    fn a_detached_subscription_stays() {
        let bus = EventBus::new();
        let (pings, on_ping) = collector::<Ping>();
        bus.subscribe(on_ping).detach();
        bus.publish(Ping { n: 7 });
        assert_eq!(pings.lock().len(), 1);
    }

    #[test]
    fn taps_see_everything_as_json_in_order() {
        let bus = EventBus::new();
        let (envelopes, on_any) = collector::<Envelope>();
        let _tap = bus.tap(on_any);

        bus.publish(Ping { n: 5 });
        bus.publish(Pong);

        let envelopes = envelopes.lock();
        assert_eq!(envelopes[0].name, "test.ping");
        assert_eq!(envelopes[0].payload, json!({ "n": 5 }));
        assert_eq!(envelopes[1].name, "test.pong");
        assert_eq!(envelopes[1].seq, envelopes[0].seq + 1);
    }

    #[test]
    fn a_handler_can_publish_and_subscribe_without_deadlocking() {
        let bus = EventBus::new();
        let (pongs, on_pong) = collector::<Pong>();
        let _pong = bus.subscribe(on_pong);

        let inner = bus.clone();
        let _ping = bus.subscribe(move |ping: &Ping| {
            if ping.n == 1 {
                inner.publish(Pong);
                inner.subscribe(|_: &Ping| {}).detach();
            }
        });

        bus.publish(Ping { n: 1 });
        assert_eq!(pongs.lock().len(), 1);
    }

    #[test]
    fn events_cross_threads() {
        let bus = EventBus::new();
        let (pings, on_ping) = collector::<Ping>();
        let _subscription = bus.subscribe(on_ping);

        let workers: Vec<_> = (0..4)
            .map(|n| {
                let bus = bus.clone();
                std::thread::spawn(move || bus.publish(Ping { n }))
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }

        let mut seen: Vec<u32> = pings.lock().iter().map(|ping| ping.n).collect();
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2, 3]);
    }

    #[test]
    fn a_subscription_outliving_the_bus_is_harmless() {
        let bus = EventBus::new();
        let subscription = bus.subscribe(|_: &Ping| {});
        drop(bus);
        drop(subscription);
    }
}
