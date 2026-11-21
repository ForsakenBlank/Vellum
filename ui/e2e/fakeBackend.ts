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
    /** quick tool sessions keep their files and options in each step */
    inputs?: string[];
    options?: Json;
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
  const runnable = new Set(["export_to_size", "video_to_gif", "gif_tools", "convert"]);
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

  // tool options keep null as a value, unlike settings patches
  const mergeOptions = (target: unknown, patch: unknown): unknown => {
    if (typeof patch !== "object" || patch === null || Array.isArray(patch)) return patch;
    if (typeof target !== "object" || target === null || Array.isArray(target)) return patch;
    const out: Json = { ...(target as Json) };
    for (const [key, value] of Object.entries(patch as Json)) {
      out[key] = key in out ? mergeOptions(out[key], value) : value;
    }
    return out;
  };

  // jobs the fake runs: they start, report progress, then finish with a tool result
  const jobs = new Map<number, Json>();
  let nextJob = 1;
  let revision = 1;
  const jobUpdate = (job: Json) => {
    job.revision = revision++;
    jobs.set(job.id as number, job);
    emit("job:changed", { type: "updated", job: structuredClone(job) });
  };
  const fakeResult = (doc: Doc, path: string) => {
    const options = current(doc).options ?? {};
    const name = fileName(path);
    const stem = name.includes(".") ? name.slice(0, name.lastIndexOf(".")) : name;
    const dir = path.slice(0, path.length - name.length);
    const ext = name.split(".").pop() ?? "";
    switch (doc.tool) {
      case "export_to_size": {
        const target = (options.target as { amount: number; unit: string } | null) ?? {
          amount: 10,
          unit: "MB",
        };
        return {
          path: `${dir}${stem} (${String(target.amount)} ${target.unit}).${ext}`,
          bytes: 9_412_000,
          sourceBytes: 48_200_000,
          targetBytes: 9_700_000,
          changes: ["Scaled down from 1920×1080 to 1280×720 so it stays sharp at this size."],
          notes: [],
        };
      }
      case "video_to_gif":
        return {
          path: `${dir}${stem}.gif`,
          bytes: 3_100_000,
          sourceBytes: 48_200_000,
          targetBytes: null,
          changes: [],
          notes: [
            "As an MP4 this would be about 0.6 MB instead of 3.1 MB, at full size and frame rate. Discord and most chats play MP4s just like GIFs.",
          ],
        };
      default:
        return {
          path: `${dir}${stem} (2).${ext}`,
          bytes: 1_000_000,
          sourceBytes: 2_000_000,
          targetBytes: null,
          changes: [],
          notes: [],
        };
    }
  };
  const runTool = (doc: Doc) => {
    const all = current(doc).inputs ?? [];
    if (all.length === 0)
      throw new Error("Add a file first. Drop one on the window or use Add files.");
    // joining makes one job for the whole list, like rust
    const joins = doc.tool === "convert" && current(doc).options?.action === "merge";
    const inputs = joins ? all.slice(0, 1) : all;
    return inputs.map((path) => {
      const id = nextJob++;
      const job: Json = {
        id,
        title: `${toolTitle(doc.tool ?? "")} ${fileName(path)}`,
        resource: "cpu",
        priority: "normal",
        state: "running",
        progress: 0.4,
        message: "Encoding (pass 2 of 2)",
        etaSecs: 3,
        elapsedSecs: 1,
        attempt: 0,
        maxRetries: 0,
        background: false,
        owner: doc.id,
        subject: path,
        error: null,
        result: null,
        createdAtMs: Date.now(),
        revision: 0,
      };
      jobUpdate(job);
      // finishes when the test says so, so it can look at the running state first
      fake.pendingJobs.push(() => {
        jobUpdate({
          ...job,
          state: "completed",
          progress: 1,
          message: null,
          etaSecs: null,
          result: { source: path, alreadyFits: false, folder: false, ...fakeResult(doc, path) },
        });
      });
      return id;
    });
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
    inputs: doc.tool ? (current(doc).inputs ?? doc.inputs) : doc.inputs,
    window: doc.window,
    dirty: doc.at !== doc.saved,
    docType: doc.tool && runnable.has(doc.tool) ? "quick_tool" : "placeholder",
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
      steps: [{ title, notes, name: "Start", kind: "", inputs: fields.inputs ?? [], options: {} }],
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
    } else if (record.kind === "tool.set_options") {
      const patch = params.patch as Json;
      const keys = JSON.stringify(Object.keys(patch).sort());
      const merge =
        !doc.sealed &&
        now.kind === `tool.set_options:${keys}` &&
        doc.at === doc.steps.length - 1 &&
        doc.at !== doc.saved;
      const options = mergeOptions(now.options ?? {}, patch) as Json;
      const name =
        Object.keys(patch).length === 1
          ? `Change ${Object.keys(patch)[0] ?? ""}`
          : "Change options";
      if (merge) {
        doc.steps[doc.at] = { ...now, options };
      } else {
        doc.steps = doc.steps.slice(0, doc.at + 1);
        doc.steps.push({ ...now, options, name, kind: `tool.set_options:${keys}` });
        doc.at++;
      }
      doc.sealed = false;
    } else if (record.kind === "tool.rename_files") {
      const renames = params.renames as { from: string; to: string }[];
      const inputs = (now.inputs ?? []).map((p) => renames.find((r) => r.from === p)?.to ?? p);
      doc.steps = doc.steps.slice(0, doc.at + 1);
      doc.steps.push({
        ...now,
        inputs,
        name: renames.length === 1 ? "Rename a file" : `Rename ${String(renames.length)} files`,
        kind: record.kind,
      });
      doc.at++;
      doc.sealed = true;
    } else if (record.kind === "tool.add_inputs" || record.kind === "tool.remove_inputs") {
      const paths = params.paths as string[];
      const before = now.inputs ?? [];
      const adding = record.kind === "tool.add_inputs";
      const inputs = adding
        ? [...before, ...paths.filter((p) => !before.includes(p))]
        : before.filter((p) => !paths.includes(p));
      if (inputs.length === before.length) throw new Error("nothing changed");
      const count = Math.abs(inputs.length - before.length);
      doc.steps = doc.steps.slice(0, doc.at + 1);
      doc.steps.push({
        ...now,
        inputs,
        name: `${adding ? "Add" : "Remove"} ${count === 1 ? "a file" : `${String(count)} files`}`,
        kind: record.kind,
      });
      doc.at++;
      doc.sealed = true;
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

  // what rust reports for a phone video, so the info panel can be tested without ffmpeg
  const sampleVideo = (path: string) => ({
    path,
    name: path.split("/").pop() ?? path,
    kind: "video",
    format: "MP4 / QuickTime",
    sizeBytes: 48_200_000,
    modifiedMs: 1_763_000_000_000,
    width: 1080,
    height: 1920,
    durationSecs: 21.4,
    bitRate: 18_000_000,
    video: {
      codec: "hevc",
      codecName: "H.265 / HEVC (High Efficiency Video Coding)",
      profile: "Main 10",
      pixelFormat: "yuv420p10le",
      bitDepth: 10,
      codedWidth: 1920,
      codedHeight: 1080,
      frameRate: 30000 / 1001,
      frameRateText: "30000/1001",
      variableFrameRate: true,
      frameCount: 641,
      bitRate: 17_700_000,
      rotation: 90,
      color: { space: "bt2020nc", primaries: "bt2020", transfer: "arib-std-b67", range: "tv" },
      hdr: { format: "hlg", masteringDisplay: null, maxCll: null, maxFall: null },
    },
    audio: [
      {
        codec: "aac",
        codecName: "AAC (Advanced Audio Coding)",
        channels: 2,
        channelLayout: "stereo",
        sampleRate: 48000,
        bitRate: 192_000,
        language: null,
        title: null,
      },
    ],
    subtitles: [],
    image: null,
    chapters: 0,
    tags: [
      { group: "File", key: "com.apple.quicktime.make", value: "Apple" },
      {
        group: "File",
        key: "com.apple.quicktime.location.ISO6709",
        value: "+51.5072-000.1276+011.000/",
      },
      { group: "Video", key: "handler_name", value: "Core Media Video" },
    ],
    hasLocation: true,
  });

  const noPreview = (args: Json, kind: string) => ({
    source: String(args.path),
    kind,
    state: { state: "ready", file: null, meta: null },
  });

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
      const step = current(doc);
      if (doc.tool) {
        return {
          info: info(doc),
          state: { title: step.title, inputs: step.inputs ?? [], options: step.options ?? {} },
        };
      }
      return { info: info(doc), state: { title: step.title, notes: step.notes } };
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
    list_jobs: () => ({ jobs: [...jobs.values()], waiting: [], revision }),
    retry_job: () => null,
    run_quick_tool: (args) => runTool(findDoc(args.document)),
    size_plan: (args) => ({
      kind: "video",
      goalBytes: 9_700_000,
      sourceBytes: 48_200_000,
      alreadyFits: false,
      summary: `1280×720, 1180 kbps picture, 128 kbps sound. (${
        ((args.options as Json).strategy as string | undefined) ?? ""
      })`,
      video: null,
      problem: null,
    }),
    anim_estimate: (args) => ({
      bytes: 2_400_000,
      width: ((args.options as Json).width as number | null) ?? 1080,
      height: 270,
      seconds: 3,
      frames: 45,
    }),
    gif_frames: () => ({
      info: { width: 200, height: 100, delaysMs: [100, 100, 100, 50], loops: 0 },
      sheet: "/cache/sheet.png",
      columns: 4,
      frameWidth: 144,
      frameHeight: 72,
    }),
    convert_targets: () => ["mp4", "webm", "mkv", "mov", "gif"],
    // a small copy of batch_rename::plan: {name} and {n} only, and "taken" is always taken
    rename_plan: (args) => {
      const pattern = args.pattern as { pattern: string; start: number; digits: number };
      const inputs = current(findDoc(args.document)).inputs ?? [];
      const unknown = /\{(?!name\}|n\}|date\}|folder\}|ext\})([^}]*)\}/.exec(pattern.pattern);
      if (unknown) {
        return {
          rows: inputs.map((p) => ({ from: p, to: p, problem: null })),
          error: `{${unknown[1] ?? ""}} is not a tag. Use {name}, {n}, {date}, {folder} or {ext}.`,
        };
      }
      return {
        rows: inputs.map((from, i) => {
          const slash = from.lastIndexOf("/");
          const dir = from.slice(0, slash + 1);
          const file = from.slice(slash + 1);
          const dot = file.lastIndexOf(".");
          const stem = dot > 0 ? file.slice(0, dot) : file;
          const ext = dot > 0 ? file.slice(dot) : "";
          const n = String(pattern.start + i).padStart(pattern.digits, "0");
          const name = pattern.pattern.replaceAll("{name}", stem).replaceAll("{n}", n);
          return {
            from,
            to: `${dir}${name}${ext}`,
            problem: name === "taken" ? `A file called ${name}${ext} is already there.` : null,
          };
        }),
        error: null,
      };
    },
    media_frame: (args) => `/cache/frame-${String(args.seconds)}.jpg`,
    preview_file: () => null,
    copy_files: (args) => {
      fake.copied = args.paths as string[];
      return null;
    },
    ui_event: () => null,
    // the browser has no ffmpeg or files to read, so media answers "nothing to show"
    media_capabilities: () => ({ ffmpeg: null, hardware: null }),
    probe_media: (args) => {
      const path = String(args.path);
      if (!path.endsWith(".mp4")) throw new Error("the e2e backend can only read mp4 files");
      return sampleVideo(path);
    },
    strip_metadata: () => 900,
    media_thumbnail: (args) => noPreview(args, "thumbnail"),
    media_filmstrip: (args) => noPreview(args, "filmstrip"),
    media_waveform: (args) => noPreview(args, "waveform"),
    waveform_overview: () => null,
    media_cache_stats: () => ({ bytes: 0, entries: 0, limitBytes: 2048 * 1024 * 1024, path: "" }),
    clean_media_cache: () => ({ bytes: 0, entries: 0, limitBytes: 2048 * 1024 * 1024, path: "" }),
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
    /** jobs waiting for the test to finish them */
    pendingJobs: [] as (() => void)[],
    finishJobs: () => {
      for (const done of fake.pendingJobs.splice(0)) done();
    },
    /** what the last "copy as file" put on the clipboard */
    copied: [] as string[],
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
