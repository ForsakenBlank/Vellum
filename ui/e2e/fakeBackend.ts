// a stand in for the rust side, so the real ui can run in a plain browser under playwright.
//
// it installs the globals @tauri-apps/api talks to (window.__TAURI_INTERNALS__) and answers
// the same ipc commands as apps/desktop/src/ipc.rs, using real data rust generated
// (e2e/fixtures/backend.json and src/bindings/fileKinds.json). its state lives in
// localStorage, so reloading the page is a restart: without a clean exit first, the next
// load finds the autosaved session and offers to restore it, like after a crash.
//
// the function is turned into a string and run inside the page, so it must not use
// anything from outside its own body.

export interface FakeOptions {
  fixture: unknown;
  fileKinds: unknown;
  windowLabel: string;
}

export function installFakeBackend({ fixture, fileKinds, windowLabel }: FakeOptions) {
  type Json = Record<string, unknown>;
  interface Step {
    title: string;
    notes: string;
    name: string;
    kind: string;
  }
  interface Doc {
    id: string;
    path: string | null;
    kind: string | null;
    workspace: string;
    tool: string | null;
    inputs: string[];
    window: string;
    steps: Step[];
    at: number;
    saved: number;
    sealed: boolean;
  }
  interface Session {
    cleanExit: boolean;
    savedAtMs: number;
    documents: Doc[];
  }
  interface State {
    settings: Json;
    recent: Json[];
    session: Session;
    nextDoc: number;
  }

  const data = fixture as {
    commands: Json[];
    keymaps: Record<string, { bindings: Record<string, string[]>; conflicts: unknown[] }>;
    settings: Json;
  };
  const kinds = fileKinds as { kind: string; workspace: string; extensions: string[] }[];
  const KEY = "vellum.fake";
  const rustCommands = new Set([
    "edit.undo",
    "edit.redo",
    "file.save",
    "view.theme.dark",
    "view.theme.light",
    "view.theme.high_contrast",
    "view.compact.toggle",
    "view.reduced_motion.toggle",
    "view.zoom.in",
    "view.zoom.out",
    "view.zoom.reset",
    "jobs.clear_finished",
    "recent.clear",
    "notifications.mark_read",
    "notifications.clear",
  ]);

  const load = (): State | null => {
    try {
      const raw = localStorage.getItem(KEY);
      return raw ? (JSON.parse(raw) as State) : null;
    } catch {
      return null;
    }
  };
  const previous = load();
  // whatever the last page load left behind is what a crash or quit would leave
  const leftover = previous && previous.session.documents.length > 0 ? previous.session : null;
  const state: State = {
    settings: previous?.settings ?? structuredClone(data.settings),
    recent: previous?.recent ?? [],
    session: { cleanExit: false, savedAtMs: Date.now(), documents: [] },
    nextDoc: previous?.nextDoc ?? 1,
  };
  let pending: Session | null = leftover;
  const persist = () => {
    const stored: State = { ...state, session: pending ?? state.session };
    // the old session stays stored until the user decides, like the recovery folders
    if (pending && state.session.documents.length > 0) stored.session = state.session;
    localStorage.setItem(KEY, JSON.stringify(stored));
  };
  persist();

  // callbacks and event listeners, the way tauri's ipc keeps them
  const callbacks = new Map<number, (value: unknown) => void>();
  const listeners = new Map<string, { id: number; handler: number }[]>();
  let nextCallback = 1;
  let nextListener = 1;

  const emit = (event: string, payload: unknown) => {
    for (const { id, handler } of listeners.get(event) ?? []) {
      callbacks.get(handler)?.({ event, id, payload });
    }
  };

  const mergePatch = (target: unknown, patch: unknown): unknown => {
    if (typeof patch !== "object" || patch === null || Array.isArray(patch)) return patch;
    const out: Json =
      typeof target === "object" && target !== null && !Array.isArray(target)
        ? { ...(target as Json) }
        : {};
    for (const [key, value] of Object.entries(patch as Json)) {
      if (value === null) Reflect.deleteProperty(out, key);
      else out[key] = mergePatch(out[key], value);
    }
    return out;
  };

  const look = () => state.settings.appearance as Json;
  const shortcuts = () =>
    state.settings.shortcuts as { preset: string; overrides: Record<string, string[]> };
  const keymap = () => {
    const base = data.keymaps[shortcuts().preset] ?? data.keymaps.vellum;
    const bindings: Record<string, string[]> = { ...(base?.bindings ?? {}) };
    for (const [kind, chords] of Object.entries(shortcuts().overrides)) {
      if (chords.length > 0) bindings[kind] = chords;
      else Reflect.deleteProperty(bindings, kind);
    }
    return { preset: shortcuts().preset, bindings, conflicts: [] };
  };

  const setSettings = (next: unknown) => {
    const before = JSON.stringify(state.settings.shortcuts);
    state.settings = next as Json;
    persist();
    emit("settings:changed", { settings: state.settings });
    if (JSON.stringify(state.settings.shortcuts) !== before) emit("keymap:changed", keymap());
  };

  const docs = () => state.session.documents;
  const findDoc = (id: unknown) => {
    const doc = docs().find((d) => d.id === id);
    if (!doc) throw new Error("that document is no longer open");
    return doc;
  };
  const current = (doc: Doc) => doc.steps[doc.at] as Step;
  const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;
  const kindOf = (path: string) => {
    const name = fileName(path);
    const ext = name.includes(".") ? (name.split(".").pop() ?? "").toLowerCase() : "";
    return kinds.find((k) => k.extensions.includes(ext));
  };
  const info = (doc: Doc) => ({
    id: doc.id,
    title: current(doc).title || (doc.path ? fileName(doc.path) : "Untitled"),
    path: doc.path,
    kind: doc.kind,
    workspace: doc.workspace,
    tool: doc.tool,
    inputs: doc.inputs,
    window: doc.window,
    dirty: doc.at !== doc.saved,
    docType: "placeholder",
  });
  const summary = (doc: Doc) => ({
    canUndo: doc.at > 0,
    canRedo: doc.at < doc.steps.length - 1,
    undoName: doc.at > 0 ? current(doc).name : null,
    redoName: doc.steps[doc.at + 1]?.name ?? null,
    dirty: doc.at !== doc.saved,
  });
  const changed = (doc?: Doc) => {
    state.session.savedAtMs = Date.now();
    persist();
    emit("autosave:changed", { savedAtMs: Date.now(), error: null });
    if (doc) emit("history:changed", { document: doc.id, summary: summary(doc) });
    emit("documents:changed", { documents: docs().map(info) });
  };

  const newDoc = (fields: Partial<Doc>, title: string, notes = "", dirty = false): Doc => {
    const doc: Doc = {
      id: `doc-${state.nextDoc++}`,
      path: null,
      kind: null,
      workspace: "quick-tools",
      tool: null,
      inputs: [],
      window: "main",
      steps: [{ title, notes, name: "Start", kind: "" }],
      at: 0,
      saved: dirty ? -1 : 0,
      sealed: true,
      ...fields,
    };
    docs().push(doc);
    return doc;
  };

  const toolTitle = (tool: string) =>
    (data.commands.find((c) => c.kind === `tool.${tool}`)?.title as string | undefined) ?? tool;

  const remember = (paths: string[]) => {
    for (const path of paths.filter((p) => kindOf(p))) {
      const pinned = state.recent.some((f) => f.path === path && f.pinned === true);
      state.recent = [
        {
          path,
          name: fileName(path),
          kind: kindOf(path)?.kind ?? null,
          openedAtMs: Date.now(),
          sizeBytes: 1234,
          pinned,
        },
        ...state.recent.filter((f) => f.path !== path),
      ];
    }
    persist();
    emit("recent:changed", { settings: { files: state.recent } });
  };

  const openFiles = (paths: string[], tool: string | null, window: string) => {
    const opened: unknown[] = [];
    const skipped: unknown[] = [];
    if (tool) {
      const title =
        paths.length === 1 ? `${toolTitle(tool)}: ${fileName(paths[0] ?? "")}` : toolTitle(tool);
      const kind = paths.map(kindOf).find(Boolean)?.kind ?? null;
      opened.push(info(newDoc({ tool, inputs: paths, window, kind }, title)));
    } else {
      for (const path of paths) {
        const row = kindOf(path);
        if (!row) {
          const name = fileName(path);
          const ext = name.includes(".")
            ? `.${name.split(".").pop() ?? ""} files`
            : "files without an extension";
          skipped.push({ path, reason: `Vellum cannot open ${ext} yet.` });
          continue;
        }
        const existing = docs().find((d) => d.path === path && !d.tool);
        if (existing) {
          opened.push(info(existing));
          continue;
        }
        opened.push(
          info(newDoc({ path, kind: row.kind, workspace: row.workspace, window }, fileName(path))),
        );
      }
    }
    changed();
    remember(paths);
    return { opened, skipped };
  };

  const execute = (doc: Doc, record: Json) => {
    const params = record.params as Json;
    const now = current(doc);
    if (record.kind === "document.rename") {
      const title = String(params.title).trim();
      if (!title) throw new Error("a document needs a name");
      doc.steps = doc.steps.slice(0, doc.at + 1);
      doc.steps.push({ ...now, title, name: `Rename to "${title}"`, kind: "document.rename" });
      doc.at++;
      doc.sealed = false;
    } else if (record.kind === "document.edit_notes") {
      const notes = String(params.notes);
      const merge =
        !doc.sealed &&
        now.kind === "document.edit_notes" &&
        doc.at === doc.steps.length - 1 &&
        doc.at !== doc.saved;
      if (merge) {
        doc.steps[doc.at] = { ...now, notes };
      } else {
        doc.steps = doc.steps.slice(0, doc.at + 1);
        doc.steps.push({ ...now, notes, name: "Edit notes", kind: "document.edit_notes" });
        doc.at++;
      }
      doc.sealed = false;
    } else {
      throw new Error(`no command is registered as \`${String(record.kind)}\``);
    }
    changed(doc);
  };

  const zoomSteps = [0.75, 0.85, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];

  const runCommand = (record: Json, document: string | null) => {
    const kind = String(record.kind);
    if (!data.commands.some((c) => c.kind === kind))
      throw new Error(`no command is registered as \`${kind}\``);
    if (!rustCommands.has(kind)) return { uiActions: [record], message: null };
    const doc = docs().find((d) => d.id === document);
    const patchLook = (fields: Json) => {
      setSettings(mergePatch(state.settings, { appearance: fields }));
    };
    switch (kind) {
      case "edit.undo":
        if (!doc || doc.at === 0) return { uiActions: [], message: "Nothing to undo" };
        doc.at--;
        doc.sealed = true;
        changed(doc);
        break;
      case "edit.redo":
        if (!doc || doc.at >= doc.steps.length - 1)
          return { uiActions: [], message: "Nothing to redo" };
        doc.at++;
        doc.sealed = true;
        changed(doc);
        break;
      case "file.save":
        if (!doc) return { uiActions: [], message: "Open a document to save it" };
        doc.saved = doc.at;
        doc.sealed = true;
        changed(doc);
        break;
      case "view.theme.dark":
        patchLook({ theme: "dark" });
        break;
      case "view.theme.light":
        patchLook({ theme: "light" });
        break;
      case "view.theme.high_contrast":
        patchLook({ theme: "high-contrast" });
        break;
      case "view.compact.toggle":
        patchLook({ compact: look().compact !== true });
        break;
      case "view.reduced_motion.toggle":
        patchLook({ reducedMotion: look().reducedMotion !== true });
        break;
      case "view.zoom.in":
      case "view.zoom.out":
      case "view.zoom.reset": {
        const scale = Number(look().uiScale);
        const next =
          kind === "view.zoom.reset"
            ? 1
            : kind === "view.zoom.in"
              ? (zoomSteps.find((s) => s > scale + 0.001) ?? scale)
              : ([...zoomSteps].reverse().find((s) => s < scale - 0.001) ?? scale);
        patchLook({ uiScale: next });
        break;
      }
      case "recent.clear":
        state.recent = state.recent.filter((f) => f.pinned === true);
        persist();
        emit("recent:changed", { settings: { files: state.recent } });
        break;
      default:
        break;
    }
    return { uiActions: [], message: null };
  };

  const recoveryOffer = () =>
    pending
      ? {
          reason: pending.cleanExit ? "unsavedAtExit" : "crash",
          savedAtMs: pending.savedAtMs,
          documents: pending.documents.map((d) => ({
            title: info(d).title,
            path: d.path,
            workspace: d.workspace,
            dirty: d.at !== d.saved,
          })),
        }
      : null;

  // a plausible description of a file, enough for the info panel to fill in
  function fakeProbe(path: string): Json {
    const ext = path.split(".").pop()?.toLowerCase() ?? "";
    const kind = kinds.find((k) => k.extensions.includes(ext))?.kind;
    const base: Json = {
      path,
      format: ext,
      formatLabel: `${ext.toUpperCase()} file`,
      sizeBytes: 1_234_567,
      durationSecs: null,
      bitRate: null,
      video: null,
      audio: null,
      image: null,
      streams: [],
      tags: [],
      warnings: [],
    };
    if (kind === "image" || kind === "raw") {
      return {
        ...base,
        kind: "image",
        image: {
          width: 4032,
          height: 3024,
          displayWidth: 4032,
          displayHeight: 3024,
          frames: 1,
          loopCount: null,
          hasAlpha: false,
          bitDepth: 8,
          colorModel: "RGB",
          hasIccProfile: true,
          orientation: 1,
          dpi: 72,
        },
        tags: [{ group: "GPS", key: "GPSLatitude", value: "51 deg 30 min", sensitive: true }],
      };
    }
    if (kind === "audio") {
      return { ...base, kind: "audio", durationSecs: 180 };
    }
    return { ...base, kind: kind === "video" ? "video" : "other", durationSecs: 12.5 };
  }

  const handlers: Record<string, (args: Json) => unknown> = {
    app_info: () => ({ version: "0.1.0", settingsPath: "/fake/settings.json", os: "linux" }),
    get_settings: () => state.settings,
    update_settings: (args) => {
      setSettings(mergePatch(state.settings, args.patch));
      return state.settings;
    },
    reset_settings: () => {
      setSettings(structuredClone(data.settings));
      return state.settings;
    },
    list_commands: () => data.commands,
    run_command: (args) =>
      runCommand(args.record as Json, (args.document as string | null) ?? null),
    get_keymap: () => keymap(),
    list_documents: () => docs().map(info),
    open_files: (args) =>
      openFiles(args.paths as string[], (args.tool as string | null) ?? null, String(args.window)),
    close_document: (args) => {
      findDoc(args.id);
      state.session.documents = docs().filter((d) => d.id !== args.id);
      changed();
      return null;
    },
    move_document: (args) => {
      const doc = findDoc(args.id);
      state.session.documents = docs().filter((d) => d !== doc);
      doc.window = String(args.window);
      const index = typeof args.index === "number" ? args.index : docs().length;
      docs().splice(index, 0, doc);
      changed();
      return null;
    },
    tear_off_document: () => {
      throw new Error("tear off needs the desktop app");
    },
    document_command: (args) => {
      execute(findDoc(args.id), args.record as Json);
      return null;
    },
    seal_document: (args) => {
      findDoc(args.id).sealed = true;
      return null;
    },
    get_document: (args) => {
      const doc = findDoc(args.id);
      return { info: info(doc), state: { title: current(doc).title, notes: current(doc).notes } };
    },
    document_history: (args) => {
      const doc = findDoc(args.id);
      return {
        root: 0,
        current: doc.at,
        entries: doc.steps.map((step, i) => ({
          id: i,
          parent: i === 0 ? null : i - 1,
          name: step.name,
          kind: step.kind,
          onCurrentPath: i <= doc.at,
        })),
        snapshots: [],
      };
    },
    checkout_history: (args) => {
      const doc = findDoc(args.id);
      doc.at = Number(args.node);
      changed(doc);
      return null;
    },
    create_snapshot: () => 1,
    list_recent: () => ({ files: state.recent }),
    pin_recent: (args) => {
      state.recent = state.recent.map((f) =>
        f.path === args.path ? { ...f, pinned: args.pinned } : f,
      );
      persist();
      emit("recent:changed", { settings: { files: state.recent } });
      return null;
    },
    remove_recent: (args) => {
      state.recent = state.recent.filter((f) => f.path !== args.path);
      persist();
      emit("recent:changed", { settings: { files: state.recent } });
      return null;
    },
    recovery_offer: () => recoveryOffer(),
    restore_session: () => {
      const restored = (pending?.documents ?? []).map((d) => {
        const step = current(d);
        return info(
          newDoc(
            { path: d.path, kind: d.kind, workspace: d.workspace, tool: d.tool, inputs: d.inputs },
            step.title,
            step.notes,
            d.at !== d.saved,
          ),
        );
      });
      pending = null;
      changed();
      return restored;
    },
    discard_recovery: () => {
      pending = null;
      persist();
      return null;
    },
    autosave_status: () => ({ savedAtMs: null, error: null }),
    list_notifications: () => ({ items: [], unread: 0 }),
    dismiss_notification: () => null,
    open_result: () => null,
    list_jobs: () => ({ jobs: [], waiting: [], revision: 0 }),
    ui_event: () => null,
    media_status: () => ({
      ffmpeg: { ffmpegPath: "/fake/ffmpeg", ffprobePath: "/fake/ffprobe", version: "7.1" },
      ffmpegError: null,
      hardware: { encoders: [], decoders: [], softwareEncoders: ["libx264"], ffmpegVersion: "7.1" },
      checkingHardware: false,
      cache: { entries: 0, bytes: 0, limitBytes: 2 ** 31, path: "/fake/cache" },
      cacheError: null,
    }),
    probe_media: (args) => fakeProbe(String(args.path)),
    // thumbnails never finish here, so tiles keep their icons
    media_thumbnail: () => null,
    media_filmstrip: () => null,
    media_waveform: () => null,
    strip_metadata: () => 1,
    clean_media_cache: () => ({ entries: 0, bytes: 0, limitBytes: 2 ** 31, path: "/fake/cache" }),
    recheck_hardware: () => null,
    "plugin:event|listen": (args) => {
      const id = nextListener++;
      const event = String(args.event);
      listeners.set(event, [
        ...(listeners.get(event) ?? []),
        { id, handler: Number(args.handler) },
      ]);
      return id;
    },
    "plugin:event|unlisten": (args) => {
      const event = String(args.event);
      listeners.set(
        event,
        (listeners.get(event) ?? []).filter((l) => l.id !== args.eventId),
      );
      return null;
    },
    "plugin:dialog|open": () => fake.nextDialog.shift() ?? null,
  };

  const fake = {
    /** paths the next file dialogs return */
    nextDialog: [] as (string[] | null)[],
    emit,
    /** what a clean quit does: the session stays only if something is unsaved */
    cleanExit: () => {
      const dirty = docs().some((d) => d.at !== d.saved);
      state.session.cleanExit = true;
      if (!dirty) state.session.documents = [];
      pending = null;
      persist();
    },
    /** sends the drag and drop events tauri sends, positions in css pixels */
    drag: (phase: "enter" | "over" | "drop" | "leave", paths: string[], x: number, y: number) => {
      const scale = window.devicePixelRatio || 1;
      const position = { x: x * scale, y: y * scale };
      const payload =
        phase === "leave" ? null : phase === "over" ? { position } : { paths, position };
      emit(`tauri://drag-${phase}`, payload);
    },
  };

  const target = window as unknown as Json;
  target.isTauri = true;
  target.__fake = fake;
  target.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (event: string, id: number) => {
      listeners.set(
        event,
        (listeners.get(event) ?? []).filter((l) => l.id !== id),
      );
    },
  };
  target.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: windowLabel },
      currentWebview: { windowLabel, label: windowLabel },
    },
    transformCallback: (callback: (value: unknown) => void, once = false) => {
      const id = nextCallback++;
      callbacks.set(id, (value) => {
        if (once) callbacks.delete(id);
        callback(value);
      });
      return id;
    },
    unregisterCallback: (id: number) => callbacks.delete(id),
    convertFileSrc: (path: string) => `data:,${encodeURIComponent(path)}`,
    invoke: (cmd: string, args: Json = {}) => {
      const handler = handlers[cmd];
      if (!handler) return Promise.reject(new Error(`the fake backend has no ${cmd}`));
      try {
        return Promise.resolve(structuredClone(handler(args)));
      } catch (err) {
        // tauri rejects with the plain string rust sent, so the fake does too
        // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
        return Promise.reject(err instanceof Error ? err.message : String(err));
      }
    },
  };
}
