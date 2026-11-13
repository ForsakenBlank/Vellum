// a stand in for the rust backend that lives in the page. it answers the same
// commands with the same shapes, keeps its state in memory and in
// localStorage, and lets tests pretend files are dragged in from the desktop.
//
// it mirrors the rust behaviour closely enough to test the ui against, but
// the rust crates are the real thing and have their own tests

import type { CommandSpec } from "../../src/bindings/CommandSpec";
import type { DocumentId } from "../../src/bindings/DocumentId";
import type { Envelope } from "../../src/bindings/Envelope";
import type { EventMap } from "../../src/bindings/EventMap";
import type { JobSnapshot } from "../../src/bindings/JobSnapshot";
import type { MediaKind } from "../../src/bindings/MediaKind";
import type { OpenDocument } from "../../src/bindings/OpenDocument";
import type { OpenPlan } from "../../src/bindings/OpenPlan";
import type { OpenTarget } from "../../src/bindings/OpenTarget";
import type { RecentEntry } from "../../src/bindings/RecentEntry";
import type { RecoveryOffer } from "../../src/bindings/RecoveryOffer";
import type { SessionView } from "../../src/bindings/SessionView";
import type { SettingsMap } from "../../src/bindings/SettingsMap";
import type { WindowState } from "../../src/bindings/WindowState";
import type { Backend, FileDropEvent } from "../../src/lib/backend";

declare global {
  interface Window {
    vellumHarness?: MemoryBackend;
  }
}

const RUNNING = "vellum-harness.running";
const AUTOSAVE = "vellum-harness.autosave";

const KINDS: [MediaKind, string[]][] = [
  ["image", ["png", "jpg", "jpeg", "webp", "gif", "tif", "tiff", "psd", "heic", "avif", "bmp"]],
  ["video", ["mp4", "mov", "mkv", "webm", "avi", "m4v"]],
  ["audio", ["mp3", "wav", "flac", "ogg", "m4a", "aac", "opus"]],
  ["vector", ["svg", "ai", "eps"]],
  ["document", ["pdf"]],
  ["project", ["vellum"]],
];

const WORKSPACE: Partial<Record<MediaKind, string>> = {
  image: "image",
  video: "video",
  audio: "audio",
  vector: "vector",
  document: "vector",
};

const TOOLS: Record<string, MediaKind[]> = {
  "export-to-size": ["video", "image", "audio"],
  "video-to-gif": ["video"],
  "remove-background": ["image"],
  convert: ["image", "video", "audio", "vector"],
};

function classify(path: string): MediaKind {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  return KINDS.find(([, list]) => list.includes(extension))?.[0] ?? "unknown";
}

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function plan(paths: string[], target: OpenTarget): OpenPlan {
  const result: OpenPlan = { opens: [], skipped: [] };
  for (const path of paths) {
    const kind = classify(path);
    const name = fileName(path);
    const skip = (reason: string) => result.skipped.push({ path, name, reason });
    if (target.type === "tool") {
      const accepts = TOOLS[target.id];
      if (!accepts) skip("That tool does not take files.");
      else if (!accepts.includes(kind)) skip("This tool only takes other kinds of file.");
      else result.opens.push({ path, name, kind, workspace: "quick-tools", tool: target.id });
      continue;
    }
    const workspace = target.type === "workspace" ? target.id : WORKSPACE[kind];
    if (!workspace || kind === "unknown" || kind === "project") {
      skip("Vellum does not know this file type yet.");
      continue;
    }
    result.opens.push({ path, name, kind, workspace, tool: null });
  }
  return result;
}

const SPECS: CommandSpec[] = [
  ["shell.open", "Open files", "File", null, false],
  ["shell.close", "Close tab", "File", "Mod+W", true],
  ["shell.reopen_closed", "Reopen closed tab", "File", "Mod+Shift+T", true],
  ["shell.activate", "Switch to tab", "File", null, false],
  ["shell.home", "Go to home", "Go", "Mod+Shift+H", true],
  ["shell.workspace", "Go to workspace", "Go", null, false],
  ["shell.next_tab", "Next tab", "Go", "Mod+Tab", true],
  ["shell.previous_tab", "Previous tab", "Go", "Mod+Shift+Tab", true],
  ["shell.move_tab", "Move tab", "File", null, false],
  ["shell.move_to_window", "Move tab to new window", "Window", null, true],
].map(([id, title, category, shortcut, palette]) => ({
  id: id as string,
  title: title as string,
  category: category as string,
  keywords: [],
  shortcut: shortcut as string | null,
  palette: palette as boolean,
}));

function defaults(): SettingsMap {
  return {
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
}

/** fails a command the way the rust side does, with a code and a sentence */
class Failure extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

type Params = Record<string, unknown>;

export class MemoryBackend implements Backend {
  readonly windowLabel = "main";
  /** every command the ui sent, for tests to check */
  readonly calls: { command: string; args: Params }[] = [];
  /** what the next "open file" dialog returns */
  nextPick: string[] = [];

  private listeners = new Set<(envelope: Envelope) => void>();
  private dropListeners = new Set<(event: FileDropEvent) => void>();
  private seq = 0;
  private settings = defaults();
  private documents: OpenDocument[] = [];
  private windows: WindowState[] = [{ label: "main", view: { type: "home" }, position: null }];
  private closed: OpenDocument[] = [];
  private nextDocument = 1;
  private nextWindow = 1;
  private ticks = 0;
  private revision = 0;
  private recent: RecentEntry[] = [];
  private recovery: RecoveryOffer | null = null;
  private jobs = new Map<number, JobSnapshot>();
  private nextJob = 1;

  constructor() {
    const params = new URLSearchParams(location.search);
    if (params.has("fresh")) {
      localStorage.removeItem(RUNNING);
      localStorage.removeItem(AUTOSAVE);
    }
    // the marker still being there means the last page never quit cleanly
    const saved = localStorage.getItem(AUTOSAVE);
    if (localStorage.getItem(RUNNING) && saved) {
      const offer = JSON.parse(saved) as RecoveryOffer;
      if (offer.documents.length > 0) this.recovery = offer;
    }
    localStorage.setItem(RUNNING, "1");
    localStorage.removeItem(AUTOSAVE);
    for (const path of params.getAll("recent")) this.touch(path);
  }

  previewUrl(path: string): string {
    return `data:image/svg+xml,${encodeURIComponent(
      `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="120"><rect width="160" height="120" fill="#3b3f4a"/><text x="80" y="64" font-size="12" text-anchor="middle" fill="#ccc">${fileName(path)}</text></svg>`,
    )}`;
  }

  listen(onEvent: (envelope: Envelope) => void): Promise<() => void> {
    this.listeners.add(onEvent);
    return Promise.resolve(() => {
      this.listeners.delete(onEvent);
    });
  }

  onFileDrop(onEvent: (event: FileDropEvent) => void): Promise<() => void> {
    this.dropListeners.add(onEvent);
    return Promise.resolve(() => {
      this.dropListeners.delete(onEvent);
    });
  }

  /** pretends files are being dragged over the page from the desktop */
  drag(event: FileDropEvent): void {
    this.dropListeners.forEach((listener) => {
      listener(event);
    });
  }

  /** a clean exit, so the next load offers no recovery */
  quit(): void {
    localStorage.removeItem(RUNNING);
    localStorage.removeItem(AUTOSAVE);
  }

  private emit<K extends keyof EventMap>(name: K, payload: EventMap[K]): void {
    const envelope = { name, seq: this.seq++, payload } as Envelope;
    this.listeners.forEach((listener) => {
      listener(envelope);
    });
  }

  invoke<T>(command: string, args: Params = {}): Promise<T> {
    this.calls.push({ command, args });
    try {
      return Promise.resolve(this.answer(command, args) as T);
    } catch (error) {
      // tauri rejects with the plain serialised error
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
      return Promise.reject(
        error instanceof Failure ? { code: error.code, message: error.message } : error,
      );
    }
  }

  private answer(command: string, args: Params): unknown {
    switch (command) {
      case "app_info":
        return { name: "Vellum", version: "0.1.0", os: "linux", arch: "x86_64", debugBuild: true };
      case "ui_ready":
      case "jobs_clear_finished":
        return null;
      case "commands_list":
        return SPECS;
      case "settings_all":
        return this.settings;
      case "settings_set":
        return this.setSettings(args.section as keyof SettingsMap, args.value);
      case "jobs_overview":
        return { jobs: [...this.jobs.values()], pending: [] };
      case "dev_sample_job":
        return this.sampleJob(Number(args.seconds), Boolean(args.fail));
      case "job_cancel":
      case "job_remove":
        return this.endJob(Number(args.id), command === "job_remove");
      case "shell_view":
        return this.view();
      case "shell_run":
        return this.run(args.record as { id: string; params: Params });
      case "files_plan":
        return plan(args.paths as string[], args.target as OpenTarget);
      case "files_pick":
        return this.nextPick;
      case "path_open":
      case "path_reveal":
        return null;
      case "recent_list":
        return this.recent;
      case "recent_pin":
        return this.pin(args.path as string, Boolean(args.pinned));
      case "recent_remove":
        this.recent = this.recent.filter((entry) => entry.path !== args.path);
        this.emit("recent.changed", { entries: this.recent });
        return null;
      case "recent_clear":
        this.recent = this.recent.filter((entry) => entry.pinned);
        this.emit("recent.changed", { entries: this.recent });
        return null;
      case "recovery_offer":
        return this.recovery;
      case "recovery_restore":
        return this.restore();
      case "recovery_discard":
        this.recovery = null;
        return null;
      default:
        throw new Failure("command.unknown", `The harness does not know ${command}.`);
    }
  }

  private setSettings(section: keyof SettingsMap, value: unknown): unknown {
    const next: SettingsMap = { ...this.settings, [section]: value };
    next.appearance.uiScale = Math.min(2, Math.max(0.75, next.appearance.uiScale));
    this.settings = next;
    this.emit("settings.changed", { section, value: next[section] });
    return next[section];
  }

  private touch(path: string): void {
    const pinned = this.recent.find((entry) => entry.path === path)?.pinned ?? false;
    this.recent = [
      {
        path,
        name: fileName(path),
        kind: classify(path),
        openedMs: Date.now(),
        pinned,
        exists: true,
      },
      ...this.recent.filter((entry) => entry.path !== path),
    ];
  }

  private pin(path: string, pinned: boolean): null {
    const entry = this.recent.find((item) => item.path === path);
    if (entry) entry.pinned = pinned;
    else if (pinned) this.touch(path);
    const added = this.recent.find((item) => item.path === path);
    if (added) added.pinned = pinned;
    this.recent = [...this.recent];
    this.emit("recent.changed", { entries: this.recent });
    return null;
  }

  private view(): SessionView {
    return {
      documents: this.documents.map((doc) => ({ ...doc })),
      windows: this.windows.map((window) => ({ ...window })),
      closedCount: this.closed.length,
      revision: this.revision,
    };
  }

  private changed(): SessionView {
    this.revision++;
    const view = this.view();
    // the harness autosaves on every change, the real app on a timer
    localStorage.setItem(
      AUTOSAVE,
      JSON.stringify({ savedMs: Date.now(), documents: view.documents }),
    );
    this.emit("session.changed", { session: view });
    return view;
  }

  private window(label: string): WindowState {
    const window = this.windows.find((entry) => entry.label === label);
    if (!window) throw new Failure("command.failed", "That window has already been closed.");
    return window;
  }

  private doc(id: DocumentId): OpenDocument {
    const doc = this.documents.find((entry) => entry.id === id);
    if (!doc) throw new Failure("command.failed", "That file is no longer open.");
    return doc;
  }

  private focus(id: DocumentId): void {
    const doc = this.doc(id);
    doc.focused = ++this.ticks;
    this.window(doc.window).view = { type: "document", id };
  }

  private leave(label: string, left: DocumentId, next: DocumentId | undefined): void {
    const tabs = this.documents.filter((doc) => doc.window === label);
    if (label !== "main" && tabs.length === 0) {
      this.windows = this.windows.filter((window) => window.label !== label);
      return;
    }
    const window = this.window(label);
    if (window.view.type === "document" && window.view.id === left) {
      if (next !== undefined) this.focus(next);
      else window.view = { type: "home" };
    }
  }

  private neighbour(doc: OpenDocument): DocumentId | undefined {
    const tabs = this.documents.filter((entry) => entry.window === doc.window);
    const at = tabs.indexOf(doc);
    return (tabs[at + 1] ?? tabs[at - 1])?.id;
  }

  private run(record: { id: string; params: Params }): SessionView {
    const p = record.params;
    switch (record.id) {
      case "shell.open": {
        const result = plan(p.paths as string[], p.target as OpenTarget);
        if (result.opens.length === 0) {
          const first = result.skipped[0];
          throw new Failure(
            "command.failed",
            first ? `${first.name} could not be opened. ${first.reason}` : "Nothing to open.",
          );
        }
        let last: DocumentId | null = null;
        for (const open of result.opens) {
          const existing = this.documents.find(
            (doc) =>
              doc.path === open.path && doc.workspace === open.workspace && doc.tool === open.tool,
          );
          if (existing) {
            last = existing.id;
          } else {
            const id = this.nextDocument++;
            this.documents.push({
              id,
              title: open.name,
              path: open.path,
              kind: open.kind,
              workspace: open.workspace,
              tool: open.tool,
              window: p.window as string,
              openedMs: Date.now(),
              focused: 0,
            });
            last = id;
          }
          this.touch(open.path);
        }
        if (last !== null) this.focus(last);
        this.emit("recent.changed", { entries: this.recent });
        break;
      }
      case "shell.close": {
        const doc = this.doc(p.id as number);
        const next = this.neighbour(doc);
        this.documents = this.documents.filter((entry) => entry !== doc);
        this.closed.push(doc);
        this.leave(doc.window, doc.id, next);
        break;
      }
      case "shell.reopen_closed": {
        const doc = this.closed.pop();
        if (!doc) throw new Failure("command.failed", "There is no closed file to reopen.");
        doc.window = this.windows.some((w) => w.label === p.window) ? (p.window as string) : "main";
        this.documents.push(doc);
        this.focus(doc.id);
        break;
      }
      case "shell.activate":
        this.focus(p.id as number);
        break;
      case "shell.home":
        this.window(p.window as string).view = { type: "home" };
        break;
      case "shell.workspace": {
        const recent = this.documents
          .filter((doc) => doc.window === p.window && doc.workspace === p.workspace)
          .sort((a, b) => b.focused - a.focused)[0];
        if (recent) this.focus(recent.id);
        else
          this.window(p.window as string).view = { type: "workspace", id: p.workspace as string };
        break;
      }
      case "shell.next_tab":
      case "shell.previous_tab": {
        const tabs = this.documents.filter((doc) => doc.window === p.window);
        if (tabs.length === 0) break;
        const view = this.window(p.window as string).view;
        const at = view.type === "document" ? tabs.findIndex((doc) => doc.id === view.id) : -1;
        const step = record.id === "shell.next_tab" ? 1 : -1;
        const next =
          at < 0 ? (step > 0 ? 0 : tabs.length - 1) : (at + step + tabs.length) % tabs.length;
        const target = tabs[next];
        if (target) this.focus(target.id);
        break;
      }
      case "shell.move_tab": {
        const doc = this.doc(p.id as number);
        this.documents = this.documents.filter((entry) => entry !== doc);
        const siblings = this.documents.filter((entry) => entry.window === doc.window);
        const before = siblings[p.index as number];
        const at = before ? this.documents.indexOf(before) : this.documents.length;
        this.documents.splice(at, 0, doc);
        break;
      }
      case "shell.move_to_window": {
        const doc = this.doc(p.id as number);
        const source = doc.window;
        const next = this.neighbour(doc);
        let label = p.window as string | null;
        if (!label) {
          label = `window-${this.nextWindow++}`;
          this.windows.push({
            label,
            view: { type: "home" },
            position: (p.position as WindowState["position"]) ?? null,
          });
        }
        this.documents = this.documents.filter((entry) => entry !== doc);
        doc.window = label;
        this.documents.push(doc);
        this.focus(doc.id);
        this.leave(source, doc.id, next);
        break;
      }
      default:
        throw new Failure("command.unknown", `unknown command \`${record.id}\``);
    }
    return this.changed();
  }

  private restore(): SessionView {
    const offer = this.recovery;
    if (!offer)
      throw new Failure("recovery.none", "There is no earlier session to restore any more.");
    let last: DocumentId | null = null;
    for (const doc of offer.documents) {
      const id = this.nextDocument++;
      this.documents.push({ ...doc, id, window: "main" });
      last = id;
    }
    if (last !== null) this.focus(last);
    this.recovery = null;
    return this.changed();
  }

  private sampleJob(seconds: number, fail: boolean): number {
    const id = this.nextJob++;
    const started = Date.now();
    const job: JobSnapshot = {
      id,
      name: fail ? "Sample job that fails" : "Sample job",
      kind: "cpu",
      category: "sample",
      background: false,
      priority: "normal",
      state: "running",
      progress: 0,
      message: null,
      etaMs: null,
      elapsedMs: 0,
      attempt: 1,
      maxAttempts: 1,
      canRetry: false,
      error: null,
      output: { seconds, path: "/home/me/Videos/sample-output.mp4" },
      createdMs: started,
      startedMs: started,
      finishedMs: null,
    };
    this.jobs.set(id, job);
    this.emit("jobs", { type: "updated", job: { ...job } });
    const timer = setInterval(() => {
      const current = this.jobs.get(id);
      if (current?.state !== "running") {
        clearInterval(timer);
        return;
      }
      const elapsed = Date.now() - started;
      current.progress = Math.min(1, elapsed / (seconds * 1000));
      current.elapsedMs = elapsed;
      current.etaMs = Math.max(0, seconds * 1000 - elapsed);
      if (fail && current.progress >= 0.5) {
        Object.assign(current, {
          state: "failed",
          error: "This sample job is meant to fail half way. Press retry to run it again.",
          canRetry: true,
          finishedMs: Date.now(),
        });
      } else if (current.progress >= 1) {
        Object.assign(current, { state: "succeeded", finishedMs: Date.now(), etaMs: null });
      }
      this.emit("jobs", { type: "updated", job: { ...current } });
    }, 100);
    return id;
  }

  private endJob(id: number, remove: boolean): null {
    const job = this.jobs.get(id);
    if (!job) throw new Failure("job.not_found", "That job is no longer in the queue.");
    if (remove) {
      this.jobs.delete(id);
      this.emit("jobs", { type: "removed", id });
    } else {
      Object.assign(job, { state: "cancelled", finishedMs: Date.now() });
      this.emit("jobs", { type: "updated", job: { ...job } });
    }
    return null;
  }
}
