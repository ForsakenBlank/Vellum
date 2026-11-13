import type { AppInfo } from "../bindings/AppInfo";
import type { Envelope } from "../bindings/Envelope";
import type { EventMap } from "../bindings/EventMap";
import type { IpcError } from "../bindings/IpcError";
import type { JobSnapshot } from "../bindings/JobSnapshot";
import type { SettingsMap } from "../bindings/SettingsMap";
import type { Backend } from "../lib/backend";

type Reply = (args: Record<string, unknown>) => unknown;

/** a backend that answers from a table and lets a test send events */
export class FakeBackend implements Backend {
  calls: { command: string; args: Record<string, unknown> }[] = [];
  private replies = new Map<string, Reply>();
  private failures = new Map<string, IpcError>();
  private listeners = new Set<(envelope: Envelope) => void>();
  private seq = 0;

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
};

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
    .reply("jobs_overview", () => ({ jobs: [], pending: [] }));
}
