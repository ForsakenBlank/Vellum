import { useApp } from "../stores/app";
import { useJobs } from "../stores/jobs";
import { useSettings } from "../stores/settings";
import { createApi, errorMessage } from "./api";
import { applyAppearance } from "./appearance";
import type { Backend } from "./backend";
import { EventHub, syncFromEvents } from "./events";

/**
 * wires the stores to the backend: loads their first state, keeps them
 * current from events, then tells the backend the ui is up.
 * returns a function that disconnects again
 */
export async function connect(backend: Backend | null): Promise<() => void> {
  if (!backend) {
    useApp.setState({ status: "browser" });
    return () => undefined;
  }

  const api = createApi(backend);
  const hub = new EventHub();
  const stops: (() => void)[] = [];
  const disconnect = () => {
    stops.forEach((stop) => {
      stop();
    });
    useApp.setState({ api: null });
  };

  try {
    stops.push(
      await backend.listen((envelope) => {
        hub.dispatch(envelope);
      }),
    );
    stops.push(
      useSettings.subscribe((state) => {
        if (state.sections) applyAppearance(state.sections.appearance);
      }),
    );

    const jobs = useJobs.getState();
    const settings = useSettings.getState();
    const [info, ...syncs] = await Promise.all([
      api.appInfo(),
      syncFromEvents(hub, "jobs", api.jobs.overview, jobs.applyOverview, jobs.applyEvent),
      syncFromEvents(
        hub,
        "settings.changed",
        api.settings.all,
        settings.applyAll,
        settings.applyChange,
      ),
    ]);
    stops.push(...syncs);

    useApp.setState({ status: "ready", info, api });
    // give react one frame to draw the loaded state before reporting in
    await new Promise((resolve) => requestAnimationFrame(resolve));
    await api.uiReady();
  } catch (error) {
    disconnect();
    useApp.setState({ status: "failed", notice: errorMessage(error) });
  }
  return disconnect;
}
