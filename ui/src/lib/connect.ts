import { listenForFileDrops } from "../shell/drop";
import { useApp } from "../stores/app";
import { useBackendCommands } from "../stores/commands";
import { newlyFinished, useJobs } from "../stores/jobs";
import { notificationForJob, outputPath, useNotifications } from "../stores/notifications";
import { useRecent } from "../stores/recent";
import { useSession } from "../stores/session";
import { useSettings } from "../stores/settings";
import { createApi, errorMessage, type Api } from "./api";
import { applyAppearance } from "./appearance";
import type { Backend } from "./backend";
import { EventHub, syncFromEvents } from "./events";

const MAIN_WINDOW = "main";

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
  const main = backend.windowLabel === MAIN_WINDOW;
  useApp.setState({
    windowLabel: backend.windowLabel,
    previewUrl: (path) => backend.previewUrl(path),
  });

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
    stops.push(await listenForFileDrops(backend));
    stops.push(
      useSettings.subscribe((state) => {
        if (state.sections) applyAppearance(state.sections.appearance);
      }),
    );
    stops.push(followJobs(api, main));

    const jobs = useJobs.getState();
    const settings = useSettings.getState();
    const session = useSession.getState();
    const recent = useRecent.getState();
    const [info, commands, recovery, ...syncs] = await Promise.all([
      api.appInfo(),
      api.commands(),
      // only one window asks, so the prompt shows once
      main ? api.recovery.offer() : Promise.resolve(null),
      syncFromEvents(hub, "jobs", api.jobs.overview, jobs.applyOverview, jobs.applyEvent),
      syncFromEvents(
        hub,
        "settings.changed",
        api.settings.all,
        settings.applyAll,
        settings.applyChange,
      ),
      syncFromEvents(hub, "session.changed", api.shell.view, session.applyView, (event) => {
        session.applyView(event.session);
      }),
      syncFromEvents(hub, "recent.changed", api.recent.list, recent.apply, (event) => {
        recent.apply(event.entries);
      }),
    ]);
    stops.push(...syncs);
    useBackendCommands.getState().apply(commands);
    session.setRecovery(recovery);

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

/**
 * reacts to jobs finishing: a notification in the main window, and opening
 * the result for jobs this window was asked to open when done
 */
function followJobs(api: Api, main: boolean): () => void {
  return useJobs.subscribe((state, previous) => {
    for (const job of newlyFinished(previous.jobs, state.jobs)) {
      const notifyJobs = useSettings.getState().sections?.general.notifyJobs ?? true;
      const notification = notificationForJob(job);
      if (main && notifyJobs && notification) {
        useNotifications.getState().push(notification);
      }
      const jobs = useJobs.getState();
      if (jobs.openWhenDone.includes(job.id)) {
        jobs.toggleOpenWhenDone(job.id);
        const path = outputPath(job);
        if (job.state === "succeeded" && path) {
          api.files.open(path).catch((error: unknown) => {
            useApp.getState().setNotice(errorMessage(error));
          });
        }
      }
    }
  });
}
