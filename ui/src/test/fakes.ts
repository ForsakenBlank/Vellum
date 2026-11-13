import type { AppInfo } from "../bindings/AppInfo";
import type { CommandSpec } from "../bindings/CommandSpec";
import type { Envelope } from "../bindings/Envelope";
import type { EventMap } from "../bindings/EventMap";
import type { IpcError } from "../bindings/IpcError";
import type { JobSnapshot } from "../bindings/JobSnapshot";
import type { SettingsMap } from "../bindings/SettingsMap";
import type { OpenDocument } from "../bindings/OpenDocument";
import type { SessionView } from "../bindings/SessionView";
import type { Backend, FileDropEvent } from "../lib/backend";
import { connect } from "../lib/connect";

type Reply = (args: Record<string, unknown>) => unknown;

/** a backend that answers from a table and lets a test send events */
export class FakeBackend implements Backend {
  calls: { command: string; args: Record<string, unknown> }[] = [];
  private replies = new Map<string, Reply>();
  private failures = new Map<string, IpcError>();
  private listeners = new Set<(envelope: Envelope) => void>();
  private dropListeners = new Set<(event: FileDropEvent) => void>();
  private seq = 0;

  constructor(readonly windowLabel = "main") {}

  previewUrl(path: string): string {
    return `preview://${path}`;
  }

  onFileDrop(onEvent: (event: FileDropEvent) => void): Promise<() => void> {
    this.dropListeners.add(onEvent);
    return Promise.resolve(() => {
      this.dropListeners.delete(onEvent);
    });
  }

  /** pretends files are being dragged over the window */
  drag(event: FileDropEvent): void {
    this.dropListeners.forEach((listener) => {
      listener(event);
    });
  }

  /** sets what a command returns */
  reply(command: string, reply: Reply): this {
    this.replies.set(command, reply);
    this.failures.delete(command);
    return this;
  }

  /** makes a command fail the way the real backend does, with an error object */
  fail(command: string, error: IpcError): this {
    this.failures.set(command, error);
    return this;
  }

  invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    this.calls.push({ command, args });
    const failure = this.failures.get(command);
    if (failure) {
      // tauri rejects with the plain serialised error, not an Error instance
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
      return Promise.reject(failure);
    }
    const reply = this.replies.get(command);
    return Promise.resolve((reply ? reply(args) : null) as T);
  }

  listen(onEvent: (envelope: Envelope) => void): Promise<() => void> {
    this.listeners.add(onEvent);
    return Promise.resolve(() => {
      this.listeners.delete(onEvent);
    });
  }

  emit<K extends keyof EventMap>(name: K, payload: EventMap[K]): void {
    const envelope = { name, seq: this.seq++, payload } as Envelope;
    this.listeners.forEach((listener) => {
      listener(envelope);
    });
  }

  callsTo(command: string): Record<string, unknown>[] {
    return this.calls.filter((call) => call.command === command).map((call) => call.args);
  }

  get listenerCount(): number {
    return this.listeners.size;
  }
}

export function job(overrides: Partial<JobSnapshot> = {}): JobSnapshot {
  return {
    id: 1,
    name: "Export clip.mp4",
    kind: "cpu",
    category: null,
    background: false,
    priority: "normal",
    state: "running",
    progress: null,
    message: null,
    etaMs: null,
    elapsedMs: 0,
    attempt: 1,
    maxAttempts: 1,
    canRetry: false,
    error: null,
    output: null,
    createdMs: 0,
    startedMs: 0,
    finishedMs: null,
    ...overrides,
  };
}

export const settings: SettingsMap = {
  appearance: {
    theme: "dark",
    accent: "#7c6cff",
    uiScale: 1,
    compact: false,
    reducedMotion: false,
  },
  performance: { gpuAcceleration: true, maxCpuJobs: 0, maxGpuJobs: 1, lowMemoryMode: false },
  general: { autosaveSeconds: 30, recentLimit: 30, notifyJobs: true },
  shortcuts: { preset: "vellum", bindings: {} },
  layouts: { current: {}, saved: [] },
};

export const emptySession: SessionView = {
  documents: [],
  windows: [{ label: "main", view: { type: "home" }, position: null }],
  closedCount: 0,
  revision: 0,
};

/** the shell commands the rust registry offers, as `commands_list` returns them */
export const shellSpecs: CommandSpec[] = [
  spec("shell.open", "Open files", "File", null, false),
  spec("shell.close", "Close tab", "File", "Mod+W"),
  spec("shell.reopen_closed", "Reopen closed tab", "File", "Mod+Shift+T"),
  spec("shell.activate", "Switch to tab", "File", null, false),
  spec("shell.home", "Go to home", "Go", "Mod+Shift+H"),
  spec("shell.workspace", "Go to workspace", "Go", null, false),
  spec("shell.next_tab", "Next tab", "Go", "Mod+Tab"),
  spec("shell.previous_tab", "Previous tab", "Go", "Mod+Shift+Tab"),
  spec("shell.move_tab", "Move tab", "File", null, false),
  spec("shell.move_to_window", "Move tab to new window", "Window", null),
];

function spec(
  id: string,
  title: string,
  category: string,
  shortcut: string | null,
  palette = true,
): CommandSpec {
  return { id, title, category, keywords: [], shortcut, palette };
}

export const appInfo: AppInfo = {
  name: "Vellum",
  version: "0.1.0",
  os: "linux",
  arch: "x86_64",
  debugBuild: true,
};

/** a backend with sensible answers for everything `connect` asks at startup */
export function readyBackend(): FakeBackend {
  return new FakeBackend()
    .reply("app_info", () => appInfo)
    .reply("settings_all", () => settings)
    .reply("jobs_overview", () => ({ jobs: [], pending: [] }))
    .reply("commands_list", () => shellSpecs)
    .reply("shell_view", () => emptySession)
    .reply("recent_list", () => [])
    .reply("recovery_offer", () => null);
}

export function doc(overrides: Partial<OpenDocument> = {}): OpenDocument {
  return {
    id: 1,
    title: "clip.mp4",
    path: "/media/clip.mp4",
    kind: "video",
    workspace: "video",
    tool: null,
    window: "main",
    openedMs: 0,
    focused: 0,
    ...overrides,
  };
}

/** a session whose main window shows `view`, with these documents open */
export function sessionWith(
  documents: OpenDocument[],
  view: SessionView["windows"][number]["view"],
  revision = 1,
): SessionView {
  return {
    documents,
    windows: [{ label: "main", view, position: null }],
    closedCount: 0,
    revision,
  };
}

const connections: (() => void)[] = [];

/** connects the stores to a fake backend, and disconnects after the test */
export async function connectTo(backend: Backend | null): Promise<() => void> {
  const disconnect = await connect(backend);
  connections.push(disconnect);
  return disconnect;
}

export function disconnectAll(): void {
  connections.splice(0).forEach((disconnect) => {
    disconnect();
  });
}
