import { describe, expect, it, vi } from "vitest";

import type { JobEvent } from "../bindings/JobEvent";
import { job } from "../test/fakes";
import { EventHub, syncFromEvents } from "./events";

const updated = (id: number): JobEvent => ({ type: "updated", job: job({ id }) });
const envelope = (payload: JobEvent, seq = 0) => ({ name: "jobs", seq, payload });

describe("EventHub", () => {
  it("delivers payloads to listeners of that name only", () => {
    const hub = new EventHub();
    const onJobs = vi.fn();
    const onSettings = vi.fn();
    hub.on("jobs", onJobs);
    hub.on("settings.changed", onSettings);

    hub.dispatch(envelope(updated(1)));

    expect(onJobs).toHaveBeenCalledExactlyOnceWith(updated(1));
    expect(onSettings).not.toHaveBeenCalled();
  });

  it("stops delivering after unsubscribe", () => {
    const hub = new EventHub();
    const listener = vi.fn();
    const stop = hub.on("jobs", listener);
    stop();
    hub.dispatch(envelope(updated(1)));
    expect(listener).not.toHaveBeenCalled();
  });

  it("keeps going when one listener throws", () => {
    const hub = new EventHub();
    const logged = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const second = vi.fn();
    hub.on("jobs", () => {
      throw new Error("broken listener");
    });
    hub.on("jobs", second);

    hub.dispatch(envelope(updated(1)));

    expect(second).toHaveBeenCalledOnce();
    expect(logged).toHaveBeenCalledOnce();
  });

  it("ignores events nobody listens to", () => {
    expect(() => {
      new EventHub().dispatch({ name: "unknown.event", seq: 0, payload: null });
    }).not.toThrow();
  });
});

describe("syncFromEvents", () => {
  it("replays events that arrived during loading on top of the snapshot", async () => {
    const hub = new EventHub();
    const order: string[] = [];
    let finishLoading: (snapshot: string) => void = () => undefined;
    const load = () =>
      new Promise<string>((resolve) => {
        finishLoading = resolve;
      });

    const syncing = syncFromEvents(
      hub,
      "jobs",
      load,
      (snapshot) => order.push(snapshot),
      (event) => order.push(`event ${event.type}`),
    );
    hub.dispatch(envelope(updated(1)));
    hub.dispatch(envelope({ type: "removed", id: 1 }));
    expect(order).toEqual([]);

    finishLoading("snapshot");
    await syncing;
    hub.dispatch(envelope({ type: "reordered", pending: [] }));

    expect(order).toEqual(["snapshot", "event updated", "event removed", "event reordered"]);
  });

  it("stops listening when loading fails", async () => {
    const hub = new EventHub();
    const onEvent = vi.fn();
    const failing = syncFromEvents(
      hub,
      "jobs",
      () => Promise.reject(new Error("backend is gone")),
      () => undefined,
      onEvent,
    );
    await expect(failing).rejects.toThrow("backend is gone");

    hub.dispatch(envelope(updated(1)));
    expect(onEvent).not.toHaveBeenCalled();
  });
});
