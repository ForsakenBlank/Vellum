# Vellum architecture

This file explains how the foundations work and why they were built this way. Read it before changing anything in `vellum-core`, the desktop bridge or the UI stores, because every later phase leans on these pieces.

It covers phase 0 (the foundations) and phase 1 (the app shell). Later phases add their own sections (the GPU engine, the timeline) as they land.

## Contents

1. [The shape of the project](#1-the-shape-of-the-project)
2. [Commands](#2-commands)
3. [Undo history](#3-undo-history)
4. [Events](#4-events)
5. [Settings](#5-settings)
6. [Jobs](#6-jobs)
7. [The desktop bridge](#7-the-desktop-bridge)
8. [The UI](#8-the-ui)
9. [The app shell](#9-the-app-shell)
10. [Tooling, tests and CI](#10-tooling-tests-and-ci)
11. [Recipes](#11-recipes)
12. [Where this differs from PLAN.md](#12-where-this-differs-from-planmd)
13. [Known gaps](#13-known-gaps)

---

## 1. The shape of the project

```
apps/desktop      tauri shell: window, services, commands, ui bridge
ui                react front end
crates/
  vellum-core     commands, history, events, settings, jobs,  (phase 0)
                  open documents, recent files, autosave      (phase 1)
  vellum-media    vellum-gpu    vellum-image   vellum-timeline
  vellum-audio    vellum-ai     vellum-export  vellum-capture
  vellum-vector   vellum-automation                           (stubs)
```

Three rules hold the layers together:

- **`vellum-core` depends on nothing of ours and has no Tauri in it.** It is plain Rust with `serde`, `parking_lot`, `thiserror`, `tracing` and `ts-rs`. That keeps it fast to compile and test, and lets the future CLI and plugin host use the same commands and jobs as the app.
- **Feature crates depend on `vellum-core` and on each other only downwards.** The intended order, lowest first: `media`, `gpu`, `audio`, then `image`, `ai`, `vector`, then `timeline`, `export`, `capture`, then `automation`. The stubs only declare the dependency on `vellum-core` today. Add the others when the code that needs them arrives, and never upwards.
- **`apps/desktop` is the only crate that knows about Tauri.** It wires services together and translates between IPC and crate APIs. Logic that deserves a test belongs in a crate, not here.

The whole repo is one cargo workspace and one pnpm workspace. `pnpm tauri dev` works from the root: the Tauri CLI finds `apps/desktop/tauri.conf.json`, which starts the Vite dev server in `ui`.

---

## 2. Commands

Code: `crates/vellum-core/src/command.rs`

Every user action is a `Command`. Buttons, menus, shortcuts, macro steps, CLI calls and plugins all end up constructing one and handing it to a history. That single rule is what makes undo, the command palette, macro recording and scripting come for free instead of each feature inventing its own.

```rust
#[derive(Debug, Serialize, Deserialize)]
struct SetOpacity {
    layer: LayerId,
    value: f32,
    #[serde(skip)]
    before: Option<f32>, // captured by apply, used by undo
}

impl Command<Document> for SetOpacity {
    const ID: &'static str = "image.layer.set_opacity";
    const TITLE: &'static str = "Set layer opacity";

    fn apply(&mut self, doc: &mut Document) -> CommandResult { /* .. */ }
    fn undo(&mut self, doc: &mut Document) -> CommandResult { /* .. */ }
    fn merge(&mut self, next: &Self) -> Merge { /* same layer: take next.value */ }
}
```

### Decisions

**A command is generic over its target (`Command<T>`).** `T` is the thing being edited: a raster document, a timeline, the app shell. The alternative was one target type for everything, reached through `Any`. Generic targets keep commands type safe and let each document type have its own registry and its own history, at the price that there is no single list of "all commands" in Rust. The palette does not need one: it collects `CommandSpec`s from every registry.

**A command is a serde struct, and its serialised form is only what the user asked for.** State captured for undo lives in `#[serde(skip)]` fields. This makes a `CommandRecord { id, params }` small, stable and replayable on another file, which is exactly what a macro or a CLI call is. The cost is that the undo stack itself is not serialisable. Crash recovery is therefore autosave of the document (phase 1), not a replay of history.

**Two traits: `Command<T>` for authors, `DynCommand<T>` for storage.** Authors implement the typed trait, where `merge` receives `&Self`. The history and the registry hold `Box<dyn DynCommand<T>>`. The bridge between them is a private wrapper, so the downcast needed for merging happens in exactly one place and a command can never be merged with a different type by accident. I tried a blanket `impl DynCommand for C: Command` first and dropped it: both traits have the same method names, so any file importing both got ambiguity errors.

**`apply` takes `&mut self`.** Capturing "what was there before" at apply time is the simplest correct way to undo. `redo` defaults to calling `apply` again and can be overridden when replaying saved state is cheaper than recomputing.

**Merging is the previous command's decision.** After a new command has been applied, the history offers it to the command before it. `Merge::Yes` folds a slider drag into one step. `Merge::Annul` says the pair cancelled out (the slider went back to where it started), and the history drops the step entirely, so the document is not marked as changed.

**Commands that cannot be undone still go through the system.** Exporting or copying returns `false` from `is_undoable`. The history runs them and stores nothing. They exist as commands so macros can record them and the palette can list them.

**`Batch` groups commands into one undo step** and is all or nothing: if a step fails, the steps before it are rolled back. Its record nests the records of its children, so a batch round trips through the registry like any other command.

**`cost()` reports the heap bytes a command holds for undo.** It is zero for everything in phase 0. Image tiles in phase 5 will make undo memory the main budget of the app, so the hook is in the trait now rather than retrofitted across every command later.

### What the contract demands of a command

- If `apply` returns an error, the target is exactly as it was.
- `undo` after `apply` restores the previous state. `redo` after `undo` restores the applied state.
- A non undoable command must not change state the undo stack depends on.

---

## 3. Undo history

Code: `crates/vellum-core/src/history.rs`

`History<T>` is a tree of nodes, not a stack. Each node holds the command that leads to it from its parent. The head is where the document currently is.

```
root - A - B - C          undo twice from C, then do D:
            \
             D   <- head  B and C are still there, go_to(C) walks back
```

### Decisions

**Branching instead of discarding.** Doing something new after an undo starts a sibling branch. Nothing is lost, which is what PLAN.md section 7 asks for ("undo, change something, still able to get back"). `redo` follows the branch the user was on most recently. `go_to(node)` reaches any node by undoing up to the shared ancestor and redoing down.

**Snapshots are bookmarks, not copies.** A named snapshot is a label on a node. Restoring one is a `go_to`. This is free in memory and always consistent with the commands, where copying the document per snapshot would be neither. It depends on the command contract above, which is why that contract is strict.

**The history does not own the document.** The owner keeps the target and its history next to each other and passes the target into every call. This keeps `History` free of locking decisions (a render thread reading a document while the UI thread edits it is a problem for phase 4 and 5 to solve in one place) and lets tools mutate the document directly during a gesture and record the result afterwards with `push_applied`. A brush stroke works that way: it paints as it goes and only knows its undo data at the end.

**Merging has two brakes.** A time window (two seconds by default) stops unrelated edits from fusing, and `seal()` ends a run explicitly. The UI calls `seal()` on pointer up, so two separate drags of the same slider are two undo steps. Undo, redo, snapshots and saving all seal too, which also guarantees that a merged or annulled node is always a leaf nothing else points at.

**Limits prune the least useful history first.** There is a cap on entries and a cap on bytes (the sum of `cost()`). When over, the branch tip the user has not visited for the longest goes first. Only when a single line is left does the oldest step fall off the bottom, as it would in a plain undo stack. Nodes with a snapshot are never pruned: a snapshot is a deliberate act and silently losing one would break "never surprise the user". The limit is therefore soft by exactly the amount the user has pinned.

**Dirty tracking is a node comparison.** `mark_saved()` remembers the head, `is_dirty()` compares. Undoing back to the saved state makes the document clean again. If the saved node is pruned the document stays dirty, which is the honest answer.

**`view()` gives the UI the whole tree** as plain data (`HistoryView`), with a revision counter so the UI knows when to refresh.

---

## 4. Events

Code: `crates/vellum-core/src/event.rs`, `apps/desktop/src/bindings.rs`, `ui/src/lib/events.ts`

Rust code publishes typed events on an `EventBus`. Rust subscribers receive them by type. A "tap" receives every event already serialised, which is how they reach the UI.

```
job queue --publish(JobEvent)--> EventBus --typed--> rust subscribers
                                     |
                                     +--tap: Envelope{name, seq, payload}
                                              |  one tauri event, "vellum:event"
                                              v
                                  EventHub.on("jobs", handler)   // payload typed by EventMap
```

### Decisions

**Events are an open set of types, not one big enum.** Any crate can define an event by implementing `Event` on a serde struct and giving it a name. A central enum would have forced `vellum-core` to know the payload types of every other crate, which inverts the dependency order.

**The UI learns the types from the same Rust definitions.** `ts-rs` generates a TypeScript type per payload during `cargo test`. `apps/desktop/src/bindings.rs` lists the events the UI may see, and a test turns that list into `EventMap.ts`. `EventHub.on("jobs", ...)` is typed against that map, so a renamed field or event is a TypeScript compile error. CI fails if the generated files were not committed.

**One Tauri event carries everything.** Events travel inside an `Envelope` on a single channel. This keeps order across event types, gives one place to log or trace, and means adding an event never touches the bridge. `seq` counts published events for ordering and debugging.

**Handlers run on the publishing thread.** No queue, no extra thread, no surprise reordering. The rule that comes with it: handlers must be quick and must not block. The bus copies its handler list before calling out, so a handler may publish or subscribe without deadlocking.

**Subscriptions are RAII guards.** Dropping one unsubscribes. `detach()` keeps a handler for the life of the bus.

**This bus is for state changes, not for bulk data.** Video frames, waveforms and anything else large or high rate will use Tauri channels or shared textures (phase 4 and 8). JSON events are the wrong tool for those.

### Loading state without races

The UI needs "the current state, then every change after it". Command replies and events travel on separate channels, so an event can arrive before the snapshot it logically follows. `syncFromEvents` in `ui/src/lib/events.ts` subscribes first, holds back events that arrive while the snapshot loads, applies the snapshot, then replays the held events on top. Because events carry whole values (a full job snapshot, a full settings section), replaying one that the snapshot already included is harmless. Every store uses this helper.

---

## 5. Settings

Code: `crates/vellum-core/src/settings.rs`

One JSON file made of named sections:

```json
{
  "format": 1,
  "sections": {
    "appearance": { "version": 1, "value": { "theme": "light" } }
  }
}
```

### Decisions

**Sections are typed and owned by whoever needs them.** A section is a serde struct implementing `SettingsSection` with a key. `store.get::<AppearanceSettings>()` and `store.update::<AppearanceSettings>(|a| ...)` are the whole API for Rust code. Later crates add their own sections without touching `vellum-core`.

**Each section has its own version and its own migrations.** `migrate(from, json)` upgrades stored JSON by one version at a time, and the store chains the steps. One global version would have made every crate coordinate on a single number. Migrations run on raw JSON before typed parsing, so they can rename and reshape freely. A migrated section is written back straight away.

**Only values that differ from the defaults are stored.** If a user never touched a setting, it follows whatever the default is in the build they run, including after an update changes that default. It also keeps the file short enough to read. The consequence for migration authors: any field may be missing.

**Unknown sections are preserved.** A section this build has no type for (a plugin that is not loaded, a newer version) is kept as raw JSON and written back untouched.

**Bad data never destroys the file.** A file that is not valid JSON, or uses a newer file format, is moved aside to `settings.unreadable-<time>.json` and the app starts from defaults. A single section that fails to parse or migrate falls back to its defaults in memory while the stored data stays in the file. Writes are atomic (temp file, fsync, rename), so a crash mid write leaves the old file or the new one, never a torn one.

**The UI reads and writes by key, through the type.** `settings_all` returns every registered section in full. `settings_set` parses the incoming JSON as the section's Rust type, runs `sanitize()` (clamping ranges, rejecting malformed colours), saves, and returns what was actually stored. The UI cannot write a shape the Rust side does not understand.

**Every change publishes `SettingsChanged`** with the full new value, which is how both the UI and Rust services (the job limits, for instance) react.

Trade off accepted: writes are synchronous and happen under the store lock. Settings are small and changes are rare. Callers that drive a setting from a slider should debounce on their side.

---

## 6. Jobs

Code: `crates/vellum-core/src/jobs/`

Anything slow is a `Job` on the `JobQueue`: exports, AI, proxies, thumbnails, conversions, downloads.

```rust
let handle = queue.submit(
    JobSpec::cpu("Export holiday.mp4").category("export").priority(Priority::High),
    move |ctx: &JobContext| {
        for (index, frame) in frames.iter().enumerate() {
            ctx.checkpoint()?;                       // cancel and pause take effect here
            encode(frame)?;
            ctx.progress(index as f32 / frames.len() as f32);
        }
        Ok(json!({ "path": output }))
    },
);
```

### Decisions

**Threads, not async.** Media work is blocking and CPU or GPU bound: FFmpeg calls, encoders, ONNX sessions. An async runtime adds nothing for that and makes every call site worry about blocking the executor. Each running job gets its own OS thread. Thread creation is noise next to the cost of any real job, and a thread per job is what makes pausing simple (see below). `vellum-core` stays free of tokio.

**Cancel and pause are cooperative.** A job calls `ctx.checkpoint()` often. It is one atomic load when nothing is happening. When the job has been cancelled it returns an error the job passes up with `?`, and when paused it blocks until resumed. Killing threads is not an option in Rust, and cooperative points are also where a job can close files cleanly. `ctx.sleep()` waits without delaying a cancel, and `ctx.cancel_token()` lets worker threads and decoder callbacks see the cancel too.

**The states are honest.** `Pausing` and `Cancelling` exist because a job does not stop the instant the user clicks. Showing "Cancelled" while the thread still holds a file open leads to bugs like a failed delete of the partial output.

**A paused job gives its slot back.** The thread parks inside `checkpoint`, the slot goes to the next waiting job, and on resume the job waits at the front of its priority level for a slot. The usual reason to pause a long export is to let something else through, so holding the slot would defeat the point. Memory the job holds stays held while paused.

**Limits are per kind of work.** `JobKind` is `Cpu`, `Gpu` or `Io`, each with its own limit: by default half the cores up to four for CPU (encoders already spread over every core), one for GPU (two models fighting over VRAM is worse than queueing), four for IO. A download never waits behind an export. The limits follow the performance settings at runtime.

**The waiting line is an explicit list.** Priority decides where a new job is inserted. After that the list is the order, so a manual move in the queue panel simply reorders it and wins over priority.

**One dispatcher thread starts jobs, fires delayed retries and publishes every event.** Funnelling all events through one thread keeps them in order and means no event handler ever runs on a job's thread or under the queue lock, so a handler can call straight back into the queue.

**Progress events are rate limited.** A job may report progress a thousand times a second. State changes are sent at once, progress is sent at most every 100 ms per job, and the final state is always sent.

**Retries come in two kinds.** A job returns `JobError::retryable(..)` for failures that may pass (a dropped connection) and the queue retries with doubling backoff up to the spec's limit. Separately, the user can retry any failed or cancelled job by hand. The job object is kept for that. A job that panicked is the exception: the panic is caught and reported as a failure, but the object is dropped because its state cannot be trusted.

**The ETA follows recent speed.** `EtaEstimator` looks at progress over the last thirty seconds of active running time, so it adapts when a job speeds up or slows down, ignores time spent paused, and starts over when progress goes backwards (a second pass).

**Events are whole snapshots.** `JobEvent::Updated` carries the complete `JobSnapshot`, so the UI store is an upsert by id with no merging logic. `Removed` and `Reordered` cover the rest. `Reordered` is only sent when a job lands somewhere other than the end of the line, so queueing five thousand thumbnails does not send five thousand lists.

**Finished jobs are kept, up to a cap** (200), so the queue panel and the notification centre have something to show. The oldest fall off with a `Removed` event.

**Shutdown is explicit and safe to skip.** The app calls `shut_down()` on exit, which cancels everything. Dropping the last `JobQueue` clone does the same.

### Rules for job authors

- Call `ctx.checkpoint()` at least a few times a second.
- `run` is called again on the same object for a retry, so keep what a second attempt needs.
- Return the result as JSON that the UI can act on, typically the output path.
- Mark housekeeping with `.background()` so it stays out of the user's way.

---

## 7. The desktop bridge

Code: `apps/desktop/src/`

`AppState` owns the event bus, the settings store and the job queue for the life of the app. `commands.rs` exposes them to the UI.

### Decisions

**Every command runs off the main thread.** All of them are declared `#[tauri::command(async)]`. A synchronous Tauri command runs on the main thread, where a slow disk or a contended lock would freeze the window.

**Commands are thin.** Check the input, call a service, map the error. Anything longer belongs in a crate where it can be tested without a window.

**Errors are written for the user.** A failed command rejects with `IpcError { code, message }`. `code` is stable for the UI to branch on, `message` is a sentence that says what happened and, where possible, what to do. The UI shows it in the notice bar.

**Payload types are generated, command signatures are not.** `ts-rs` generates every type that crosses the bridge. The command names and argument lists in `ui/src/lib/api.ts` are kept in step with `commands.rs` by hand. `tauri-specta` would generate those too, but it has been a release candidate for a long time and it would put a second code generator at the centre of the build. The hand written part is one small file. Worth revisiting once the command list grows past a few dozen.

**Settings problems never stop the app from opening.** If the settings file cannot be opened at all, the app runs with an in memory store for the session and logs why.

**Startup is measured.** The UI calls `ui_ready` after its first frame with loaded state, and the backend logs the time since launch. The same signal powers the CI smoke test: with `VELLUM_SMOKE_TEST` set, the app exits with 0 at that point, or with 1 if it never gets there.

**The CSP is strict from day one.** Scripts only from the app itself, IPC and the asset protocol allowed, inline styles allowed because Radix positions popups with them. Starting strict is much cheaper than tightening later.

**`panic = "unwind"` is required.** The job queue catches panics inside jobs. Switching the release profile to `abort` to save binary size would turn a bug in one job into a crash of the whole app.

---

## 8. The UI

Code: `ui/src/`

```
lib/backend.ts    the only file that imports @tauri-apps/api
lib/api.ts        typed wrappers for every backend command
lib/events.ts     EventHub and syncFromEvents
lib/connect.ts    wires stores to the backend at startup
lib/keys.ts       shortcut strings, parsing and display
lib/fuzzy.ts      the palette's matcher
stores/           zustand stores: app, jobs, settings, session, recent, notifications, ui
commands/         the command registry, shortcut presets and the global key handler
dock/             dock layouts (pure functions), their state and the Dock component
shell/            app frame: top bar, tabs, rail, home, palette, settings, drop overlay
workspaces/       one folder per workspace with its panels and default layout
components/       shared building blocks
bindings/         generated from rust, never edited by hand
e2e/              playwright tests and the in memory backend they run against
```

### Decisions

**The backend is a small interface**: `invoke`, `listen`, `onFileDrop`, `previewUrl` and the window's label. Everything else builds on it. Tests use a fake that answers from a table and can send events, so stores and components are tested against the same protocol the real app speaks, with no Tauri mocking.

**The stores hold what the backend said, nothing more.** The backend is the source of truth for jobs and settings. Stores apply snapshots and events. A settings change is sent to the backend and the store takes the value that comes back, which may differ from what was sent because the backend sanitises it. There is no optimistic state to reconcile.

**User actions go through `perform()`**, which catches a failed backend call and puts its message in the notice bar. No failure ends up only in the console.

**The UI works without the backend.** Opened in a plain browser it says so and disables what needs the desktop app. This keeps `pnpm dev` useful for layout work.

**Design tokens are CSS variables.** Components use semantic Tailwind names (`bg-surface`, `text-ink-soft`, `border-line`). A theme only changes the values behind them, set by a `data-theme` attribute. Accent colour and UI scale come from settings: the scale is the root font size, and every size in the UI is in rem. Compact mode changes Tailwind's `--spacing`, which every padding and gap is a multiple of. Motion durations are three variables (`--v-fast`, `--v-base`, `--v-slow`) that reduced motion sets to zero.

**A workspace is an entry in a registry.** Each workspace folder exports a description (id, title, icon, summary), the panels it offers, its default panel layout, and later a component. Until a workspace has a component the shell shows a placeholder that names the file and the phase that builds the editor.

**Buttons always have a tooltip.** `IconButton` requires a label, used for both the tooltip and the accessible name. Section 5 of PLAN.md asks for this, and making it a required prop is cheaper than auditing later.

The sample job buttons now live at the bottom of the jobs panel and only show in debug builds. The `dev_sample_job` command can go once real jobs exist.

---

## 9. The app shell

Code: `crates/vellum-core/src/shell/`, `apps/desktop/src/host.rs`, `ui/src/shell/`, `ui/src/dock/`, `ui/src/commands/`

Phase 1 built everything around the editors: the home screen, document tabs and tear-off windows, docking panels, the command palette, settings and shortcuts, autosave and crash recovery, notifications and the job panel.

### Open documents live in Rust

```
shell.open { paths, target, window }        ui (or a drop, or a second launch)
        |                                    |
        v                                    v
   Shell::run(record)  ---------------->  ShellUpdate { view, opened, windows_added, windows_removed }
        |                                    |
        | publish SessionChanged             | host.rs: create or close windows,
        v                                    | add to recent files, allow previews
  every window's ui picks its own tabs       v
```

**The session is owned by the backend, not by a window's UI.** Every Tauri window is its own webview with its own JavaScript. A document torn off into a second window has to belong to something both windows can see, so `Session` (in `vellum-core`) holds the documents, the windows and what each window shows. Each window's UI is a view of it, filtered by its own label. The alternative, documents in the UI and messages between windows, would have meant two sources of truth and a protocol for handing documents over.

**Every change to it is a `Command`.** Opening, closing, switching, reordering and tearing off are commands in a `CommandRegistry<Session>`, run through one `shell_run(record)` IPC command. They are not undoable: Ctrl+Z belongs to the document, and closing a tab has its own way back (Ctrl+Shift+T reopens the last 20). Going through the registry still gives them names and shortcuts for the palette and makes them recordable by macros, as section 5 of PLAN.md asks.

**A failed command leaves the session as it was.** `Shell::run` clones the session before applying and puts it back on error. The session is small (a list of documents and windows), so the copy is cheap, and the commands do not each need their own rollback.

**Windows follow the session.** A command that creates a window (a tear-off) or empties one comes back with `windows_added` and `windows_removed`. `host.rs` creates or destroys the Tauri windows to match. Closing a torn off window with its close button sends its documents back to the main window instead of closing them. Closing the main window quits.

**Documents get new ids when restored.** A restored document could otherwise clash with one opened since the app started.

### Routing files

`shell/files.rs` decides what a file is (by extension in phase 1; phase 2 can swap in a real probe) and where it opens: by type, in a workspace that was dropped on, or in a quick tool. `plan_open` returns the plan without opening anything. The drop overlay asks for the plan while files hover and shows it in words ("Open 3 files in 2 workspaces", "This tool only takes videos"), and the open command works the plan out again from the same function, so what the overlay promised is what happens. Quick tools list which kinds of file they take here, by the same id the UI's cards use.

**Drops come from Tauri, not HTML drag and drop.** Only Tauri's drag events carry real file paths. Tauri reports the pointer in physical pixels. The UI converts and uses `elementFromPoint` to find the closest `data-drop` attribute (`tool:video-to-gif`, `workspace:video`), so any element can become a drop target with one attribute. The rail icons are targets too.

**Internal drags use pointer events, not HTML drag and drop.** Tauri's file drop handling takes over the webview's own drag events on Windows. Tabs and panels are dragged with pointer capture instead, which also works past the edge of the window, which is how a tab is torn off.

### Recent files

`shell/recent.rs` keeps recent and pinned files in `recent.json` next to the settings, newest first. Pinned entries never count against the limit (a general setting). It is history rather than preference, so it is not a settings section, and a broken file starts the list over rather than being moved aside. "Pinned projects" are pinned entries: a `.vellum` folder shows as a project once the project model arrives.

Thumbnails for images use Tauri's asset protocol. Its scope starts empty and each image the user opened or has in the list is allowed one by one, so the UI cannot read arbitrary files through it. Real thumbnails for video come from the cache in phase 2.

### Autosave and crash recovery

`shell/autosave.rs` keeps three files in the app data folder:

```
session/running                  exists while the app runs
session/session.json             the latest autosave of this run
session/session.recovered.json   what a crashed run left, until the user answers
```

A clean exit deletes `running` and the autosave. If `running` is still there at the next start, the last run crashed, and its autosave (if anything was open) moves to `session.recovered.json`. The main window asks "Restore your last session?" with the list of files. Restore reopens them with their windows, start fresh forgets them. A second crash before the user answers keeps the older offer, so nothing is lost by crashing twice.

The autosave timer is a plain thread (`apps/desktop/src/autosave.rs`), not a job: a session save is a few kilobytes, and a job every thirty seconds would push real work out of the list of finished jobs. It saves only when the session changed since the last save, at the interval in the general settings, or never if autosave is off. When documents with real content arrive, their own saves will be jobs started from there.

**What is saved today is the session, not document content.** Phase 1 has no editable documents, so the autosave is which files were open, where and in which tool. Phase 5 adds document content next to it.

A second launch does not start a second app: the single instance plugin focuses the running one and hands over any files from the command line. Without it, a second instance would see the first one's `running` marker and offer a bogus recovery.

### Commands, shortcuts and the palette

`ui/src/commands/registry.ts` builds the list of everything the palette can find and a key can run. Backend commands come from `commands_list` with their titles and default keys from Rust; the UI only knows how to fill in their parameters from what is on screen ("the document in front of this window"). Commands that only touch one window (open the palette, change the theme, show a panel) are declared in the UI with the same shape.

**Shortcuts are strings like `Mod+Shift+K`.** `Mod` is Ctrl on Windows and Linux and Cmd on a Mac, so one string works everywhere, and modifiers always come in the same order so equal shortcuts are equal strings. Keys are read from `event.code`, so Shift+1 is "Shift+1" on every keyboard layout. A shortcut with no modifier never fires while typing in a field.

**Three layers make the keymap**: the default each command declares, the preset (Vellum, Photoshop, Premiere Pro, DaVinci Resolve, Audacity), and the user's own changes. A layer that names a command replaces that command's keys entirely, which keeps "what will this key do" easy to reason about. Only the preset and the user's changes are stored (the `shortcuts` settings section), so new commands pick up their defaults after an update.

**A conflict needs overlapping scopes.** Two commands share a key only if both can run in the same place: a global command clashes with everything, a command scoped to the image workspace only clashes with global and image ones. The shortcut editor shows conflicts, asks before taking a key that is in use, and lets the user keep both on purpose.

**The palette ranks with a small fuzzy matcher** (`lib/fuzzy.ts`): letters in order, bonuses for word starts and runs, a small cost for gaps. Keyword matches rank behind matches on the visible label. Before anything is typed it shows what was used recently (kept in `localStorage`, it is per machine convenience, not a setting), then recent files, then commands.

### Docking

A layout is three zones (left, right, bottom), each a list of tabbed panel stacks with sizes in rem. It is plain data (`DockLayout`, typed in Rust so the settings store can check it) and every change is a pure function in `dock/layout.ts`, tested on its own. While a gesture is on (a resize, a panel drag), the dock shows a preview layout from local state, and only on release is the result saved to the `layouts` settings section. Dragging never writes settings sixty times a second.

Each workspace declares its panels and a default layout. The `layouts` section stores the current layout per workspace (missing means the default) and named layouts the user saved. Panels a layout names but the workspace no longer offers are dropped when it is read. Sanitising in Rust clamps sizes and makes sure a panel is only in one place.

The home screen docks panels too (the jobs panel), with the same code.

### Notifications

Notifications are UI state in the main window: jobs finishing (done, or failed with the reason) and other app notices, with an unread count on the bell and a short toast. Clicking one with a result opens the file. They are not stored: a list of what happened while the app was open is what the plan asks for, and a persistent history belongs with the export history in phase 13. "Open the result when done" is kept per window for the same reason.

Opening a result goes through `path_open`, which refuses anything that is not a media file, so the UI can never start a program through it.

### Testing the shell

The Rust parts (`files`, `session`, `recent`, `autosave`, the shell settings) have unit tests like the rest of `vellum-core`. The UI has Vitest tests for the logic (keys, fuzzy matching, keymaps and conflicts, dock layouts, drop wording, notifications) and for the components against the fake backend.

The Playwright tests (`ui/e2e/`) run the real UI in Chromium against `e2e/harness/memoryBackend.ts`, an in memory backend that answers the same commands with the same shapes. They cover drag and drop routing, the palette, crash recovery (a reload without a clean exit is a crash to the harness), tab reordering and tear-off, and docking. Driving the real desktop app with WebDriver works on Linux and Windows only and needs a full build, so the desktop side is covered by the Rust tests and the CI smoke test instead. The harness repeats some routing rules from Rust. The Rust tests are the authority; if the two drift, the e2e tests are wrong, not the app.

---

## 10. Tooling, tests and CI

- **Rust is pinned** in `rust-toolchain.toml` so `clippy -D warnings` means the same thing on every machine and in CI. Bump it on purpose, not by accident.
- **`clippy::unwrap_used` is on** for the workspace, with tests exempt. This enforces the "no `unwrap()` outside tests" rule mechanically. `expect("why this cannot fail")` is allowed for real invariants.
- **`.gitattributes` forces LF line endings** so Prettier and the generated bindings behave the same on Windows.
- **TypeScript is strict**, with `noUncheckedIndexedAccess`. ESLint runs the type checked `strict` preset.

Tests:

| What | Where | How |
|---|---|---|
| Commands, history, events, settings, jobs, session, routing, recent files, autosave | `crates/vellum-core` | `cargo test`, 178 tests |
| Bridge lists, error text, limits | `apps/desktop` | `cargo test` |
| Stores, event sync, keys, fuzzy search, keymaps, dock layouts, components | `ui` | Vitest with jsdom and Testing Library |
| Drop routing, palette, crash recovery, tabs, docking in a real browser | `ui/e2e` | Playwright against the in memory backend |
| The window opens and the UI reaches the backend | CI | builds the app and runs it with `VELLUM_SMOKE_TEST=1` |

The job queue tests coordinate with gates and wait on conditions instead of sleeping for fixed times, so they are stable on slow machines. They were run forty times in a row and pinned to a single core without a failure.

CI (`.github/workflows/ci.yml`) runs on Linux, Windows and macOS for every push: Prettier, ESLint, TypeScript, Vitest, rustfmt, Clippy, Rust tests, a check that `ui/src/bindings` matches what the Rust types generate, a build of the app and the smoke test. Playwright runs on Linux only, since it tests the UI in Chromium and not the platform.

Useful commands:

```bash
pnpm tauri dev          # run the app
pnpm test               # ui tests
pnpm e2e                # playwright tests, starts its own vite server
cargo test --workspace  # rust tests, also regenerates ui/src/bindings
pnpm bindings           # only regenerate the bindings
```

---

## 11. Recipes

**Add a command.** Write a serde struct, implement `Command<YourDocument>`, put undo state in `#[serde(skip)]` fields, register it with `registry.register::<YourCommand>()`. Override `spec()` for category, keywords and a default shortcut. Test apply, undo, redo and, if it merges, the merge.

**Add an event.** Derive `Serialize` and `TS` (with `#[ts(export)]`), implement `Event` with a unique name, add the type to `ui_events!` in `apps/desktop/src/bindings.rs`, run `cargo test`, commit the regenerated bindings. In the UI: `hub.on("your.name", handler)`.

**Add a settings section.** Derive `Serialize`, `Deserialize`, `TS`, implement `Default` and `SettingsSection` with a key. Put `#[serde(default)]` on the struct so new fields get defaults. Add it to `settings_sections!` in `apps/desktop/src/bindings.rs`. When the stored shape changes in a way defaults cannot absorb, bump `VERSION` and add a `migrate` step with a test.

**Add a job.** Build a `JobSpec`, submit a closure or a `Job` type, call `ctx.checkpoint()` and `ctx.progress()` in its loop. The UI needs nothing new: the job appears in the queue panel.

**Add a backend command.** Add a `#[tauri::command(async)]` function in `commands.rs`, list it in `generate_handler!`, add the typed wrapper in `ui/src/lib/api.ts`. If the UI tests use it, teach `ui/e2e/harness/memoryBackend.ts` to answer it.

**Add a palette command or shortcut.** A command that changes shared state is a Rust command in a registry; give the UI a binder for its parameters in `ui/src/commands/registry.ts`. A command that only touches one window goes straight into `uiCommands()` there. Either way it shows in the palette and the shortcut editor, and its default key can be changed per preset in `ui/src/commands/shortcuts.ts`.

**Add a panel.** Write the component, add a `PanelDef` to the workspace's `panels`, and place it in the workspace's default `layout` if it should show from the start. Saved layouts that do not mention it are unaffected; it can be opened from the layout menu or the palette.

**Make something a drop target.** Give the element `data-drop="workspace:<id>"` or `data-drop="tool:<id>"`. For a new quick tool, add what it accepts to `quick_tool_accepts` in `crates/vellum-core/src/shell/files.rs`.

---

## 12. Where this differs from PLAN.md

PLAN.md asks that deviations are called out.

From phase 0:

- **The job queue lives in `vellum-core`**, not in a crate of its own. Section 4 of the plan has no crate for it and nearly every crate needs it.
- **`apps/desktop` holds the Tauri crate directly**, with no nested `src-tauri` folder. The plan describes `apps/desktop` as the Rust side of the app, and `ui/` is already separate.
- **Tauri 2, not 3.** Tauri 3 is in alpha at the time of writing. The plan says Tauri 2 and that is the stable line.
- **TypeScript 6.0, not 7.** `typescript-eslint` does not support TypeScript 7 yet. Move up when it does.
- **No project (`.vellum` bundle) model yet.** The plan lists the project model under `vellum-core`, but designing it without a real document type would be guesswork. Phase 1 autosaves the session (which files are open where) instead, and the project model arrives with the first real document in phase 5.
- **No per document session yet.** The piece that owns a document's content, its `History` and its `CommandRegistry`, and exposes "run this `CommandRecord` on document N" over IPC, arrives with the first real document type. Phase 1 added the shell session that knows which documents are open, with the same shape at a smaller scale (a `Mutex`, a registry and one `shell_run` command). The per document one will hang off `OpenDocument`.

From phase 1:

- **Notifications are not stored.** The plan does not say either way. They live for the session; a history of exports belongs with phase 13.
- **Placeholder panels.** The plan says placeholder workspaces are fine; each one also docks placeholder panels (layers, timeline, scopes) so docking and saved layouts can be used and tested before the real panels exist.
- **Shortcut presets cover the commands that exist.** The Photoshop, Premiere Pro, DaVinci Resolve and Audacity presets map the shell's keys (settings, panels, workspaces as pages). They grow with every editor phase. An Audacity preset was added since the phase 1 prompt asks for one.
- **"Pinned projects" are pinned recent entries** until the project model exists.

---

## 13. Known gaps

- **`pnpm tauri dev` was run by hand on Linux only.** On Windows and macOS, CI builds the app and proves the window opens and the UI connects, but nobody has yet looked at it with their own eyes there.
- **A job is one kind.** A hardware encode uses both CPU and GPU. If that causes contention, a job could claim more than one slot. Wait for evidence from phase 2 and 3.
- **No job groups.** A batch of 200 files is 200 jobs sharing a `category`. If the queue panel needs to collapse them into one row, add an optional group id to `JobSpec`, which does not break anything.
- **Macro recording only has its building blocks.** `History::records_since` returns the undoable commands along the current line. Capturing non undoable commands too is the job of the session layer in phase 16.
- **Settings are not watched on disk.** Editing the file by hand while the app runs has no effect until restart, and the next save overwrites the edit.
- **Logs go to stderr only.** A log file and the opt in crash reports are phase 18.
- **Tear-off has been tried by hand on Linux only.** Dragging a tab out of the bar opened a new window with it. Closing a torn off window with its close button (which should send its files back to the main window) is covered by the Rust tests but has not been clicked through in the real app, and nothing about tear-off has been looked at on Windows or macOS. Dragging a tab back into another window's tab bar is not supported yet; use "Move to main window" on the tab's menu.
- **A batch drop onto a quick tool opens one document per file.** Proper batches arrive with phase 16.
- **Window positions and sizes are not remembered** across restarts, beyond where a torn off window first appears.
- **The harness repeats routing rules.** `ui/e2e/harness/memoryBackend.ts` mirrors `shell/files.rs` in a simpler form. It is test code, but it has to be kept roughly in step.
