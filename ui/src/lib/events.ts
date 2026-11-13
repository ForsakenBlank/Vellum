import type { Envelope } from "../bindings/Envelope";
import type { EventMap } from "../bindings/EventMap";

export type EventName = keyof EventMap;

type Listener = (payload: unknown) => void;

/**
 * the ui end of the rust event bus. names and payload types come from
 * EventMap, which is generated from the rust event types
 */
export class EventHub {
  private listeners = new Map<string, Set<Listener>>();

  /** returns a function that removes the listener again */
  on<K extends EventName>(name: K, listener: (payload: EventMap[K]) => void): () => void {
    const set = this.listeners.get(name) ?? new Set<Listener>();
    this.listeners.set(name, set);
    const erased = listener as Listener;
    set.add(erased);
    return () => {
      set.delete(erased);
    };
  }

  dispatch(envelope: Envelope): void {
    // copied so a listener can unsubscribe itself while we loop
    for (const listener of [...(this.listeners.get(envelope.name) ?? [])]) {
      try {
        listener(envelope.payload);
      } catch (error) {
        // one broken listener must not starve the others
        console.error(`listener for "${envelope.name}" failed`, error);
      }
    }
  }
}

/**
 * loads a snapshot and then keeps it current from events.
 *
 * events and command replies travel on separate channels, so an event can
 * overtake the snapshot it belongs after. events that arrive while the
 * snapshot is loading are held back and replayed on top of it, which leaves
 * the state correct whichever one wins the race
 */
export async function syncFromEvents<K extends EventName, S>(
  hub: EventHub,
  name: K,
  load: () => Promise<S>,
  onSnapshot: (snapshot: S) => void,
  onEvent: (event: EventMap[K]) => void,
): Promise<() => void> {
  let held: EventMap[K][] | null = [];
  const stop = hub.on(name, (event) => {
    if (held) held.push(event);
    else onEvent(event);
  });

  try {
    onSnapshot(await load());
  } catch (error) {
    stop();
    throw error;
  }

  const replay = held;
  held = null;
  replay.forEach(onEvent);
  return stop;
}
