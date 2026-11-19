# Vellum architecture

This file explains how Vellum is put together and why. It is written for whoever works on the next phase, human or AI. Read `PLAN.md` first for the what; this file is the how.

Every section ends with the alternatives that were considered and why they lost, so a later phase can tell a deliberate choice from an accident before changing it.

## 1. The big picture

```
 ui/ (React, TypeScript)
   zustand stores  <──── tauri events ("job:changed", "documents:changed", ...)
   ipc.ts wrappers ────> tauri commands (invoke)
         │
 apps/desktop/ (thin tauri shell)
   AppState { AppServices, window counter }
   TauriSink: forwards every bus event to every window
         │
 crates/vellum-core/          AppServices: commands, documents, history, events,
                              settings, recent files, notifications, autosave, jobs
         ▲
 crates/vellum-media, -gpu, -image, -timeline, -audio, -ai,
        -export, -capture, -vector, -automation   (stubs until their phase)
```

Rules that keep this healthy:

- **Dependencies point one way.** `vellum-core` depends on no other Vellum crate. Feature crates depend on core (and on each other only where the plan says so, for example timeline on media and gpu). Only `apps/desktop` knows about Tauri.
- **Logic lives in crates, not in the shell.** `apps/desktop` parses IPC arguments, calls into the core and maps errors to strings. Anything worth testing belongs in a crate so the CLI (phase 16) and tests can use it without a window.
- **The UI is a view.** Rust owns the state of settings, jobs, documents, recent files and notifications. The UI renders what Rust reports and asks Rust to change things. It never keeps its own copy of the truth that could drift. The only state the UI keeps is per window and throwaway: which tab is active, whether the palette is open, toasts, and the palette's recent commands.

## 2. Repo and tooling

- **Cargo workspace** at the root, edition 2024, resolver 3, `rust-version = 1.90` (the minimum Tauri 2.12 needs). Shared dependency versions live in `[workspace.dependencies]` so crates cannot drift apart.
- **Shared lints** in `[workspace.lints]`. `clippy::unwrap_used` is a warning, which CI turns into an error, so `unwrap()` outside tests fails the build. `clippy.toml` allows it in tests. An intentional unwrap needs an `#[allow]` with a comment saying why it cannot fail.
- **pnpm workspace** with one package for now (`ui/`, published as `@vellum/ui`). The Tauri CLI lives at the root, so `pnpm tauri dev` works from the repo root.
- **TypeScript is pinned to 6.0.x.** TypeScript 7 is out, but `typescript-eslint` only supports up to 6.0, and strict type-aware linting is worth more than the newer compiler. Revisit when `typescript-eslint` supports 7.
- **`.gitattributes` forces LF line endings** everywhere, so Prettier and rustfmt give the same answer on Windows CI as on Linux.
- **Prettier ignores Markdown.** `PLAN.md` and these docs are formatted by hand, and letting Prettier rewrap tables in the plan would make every phase's diff noisy.

## 3. Commands

File: `crates/vellum-core/src/command.rs`.

Every user action that changes a document is a `Command<T>`, where `T` is the thing it changes (an image document, a timeline, the app itself).

```rust
pub trait Command<T>: Any + Send {
    fn kind(&self) -> &'static str;          // stable id, eg "image.layer.set_opacity"
    fn name(&self) -> String;                // "Set layer opacity", for the history panel
    fn apply(&mut self, target: &mut T) -> Result<(), CommandError>;
    fn revert(&mut self, target: &mut T) -> Result<(), CommandError>;
    fn merge(&mut self, next: &dyn Command<T>) -> bool { false }
    fn params(&self) -> Result<serde_json::Value, CommandError>;
    fn undoable(&self) -> bool { true }
    fn memory_cost(&self) -> usize { size_of_val(self) }
}
```

Decisions:

- **`apply` and `revert`, not `do` and `undo`.** `do` is a Rust keyword, and `revert` reads better next to `History::undo`.
- **Commands carry their own undo state** (the old value, the replaced pixels) in fields marked `#[serde(skip)]`. `apply` captures it, `revert` uses it. This is the classic command pattern rather than snapshotting the whole document per step, because documents in this app are huge (a 50 megapixel image, a two hour timeline) and most edits touch a tiny part of them.
- **Serialisation is the user's intent, not the undo state.** `params()` returns only what is needed to rebuild the command from scratch, so a recorded macro replayed on another file captures fresh undo state there. A `CommandRecord { kind, params }` is the one wire format for UI dispatch, macros, the CLI and plugins.
- **An explicit `CommandRegistry<T>`** maps `kind` strings to factories. `register::<C>(info)` stores a monomorphised `fn(Value) -> Box<dyn Command<T>>`. The registry also holds `CommandInfo` (title, category, description), which is what the phase 1 command palette lists. Instantiating checks that the built command reports the kind it was registered under, which catches copy and paste mistakes.
- **Merging uses a downcast.** `merge(&mut self, next)` gets the next command (already applied) and folds it in if it is the same kind of edit, which is how a slider drag becomes one undo step. `command::downcast::<T, C>(next)` does the type check using trait upcasting to `&dyn Any` (stable since Rust 1.86).
- **Non-undoable commands** (`undoable() == false`) run and are recorded in macros but never land on the undo stack. "Export" and "start background removal" are commands too, so macros can replay them, but undo should not try to un-export a file.
- **Error contract:** `apply` must leave the target unchanged if it returns an error. History relies on this to stay consistent.

Alternatives considered:

- *Snapshot (memento) undo.* Simple and always correct, but costs a full document copy per step. It stays available as an implementation detail for individual commands (a filter can store the tile it replaced), without forcing it on every slider tweak.
- *`typetag` or `inventory` for automatic registration.* Less boilerplate, but they rely on linker tricks that are fragile across platforms and cannot register plugin commands at runtime. An explicit registry is a few lines per command and easy to inspect.
- *Type-erased target (`&mut dyn Any`).* Would allow one global history across document types, but loses compile-time checking for every command. One history per document is what users expect anyway.

## 4. Undo history

File: `crates/vellum-core/src/history.rs`.

`History<T>` is a **tree**. The root is the oldest state we can return to. Every other node holds the command that leads to it from its parent. Undoing and then doing something new starts a new branch instead of deleting the old one, which is the "branching history" from `PLAN.md` section 7.

- **Redo follows the most recently visited branch.** Each node remembers which child redo should take (`redo_child`), updated on undo, redo and checkout. This matches what people expect from Ctrl+Y.
- **`checkout(node)`** moves to any node on any branch by undoing up to the common ancestor and redoing down. The history panel will call this when the user clicks an entry on another branch.
- **Named snapshots are pointers to nodes**, so they are free to create. `restore_snapshot` is a `checkout`. Snapshots **pin** their node: pruning never removes it, so a snapshot can hold history beyond the limits until the user deletes it. Later phases can attach cached document states to snapshots if replaying a long path gets slow.
- **Limits:** `max_steps` (depth from root to the current state) and `max_bytes` (sum of `memory_cost`). When over, the root moves forward along the path to the current state and branches hanging off the old root are dropped. The defaults (500 steps, 1 GB) come from settings.
- **Merging rules.** A command merges into the current step only if: the current step was added less than `merge_window` (1.5 s) ago, nothing has been undone or redone since, the history was not `seal()`ed (the UI seals on pointer up), the current step has no children, and it is not the saved step or a snapshot. These keep merging from silently changing a step the user can see as a distinct point.
- **Dirty tracking:** `mark_saved()` remembers the current node, `is_dirty()` compares. Undoing back to the saved point makes the document clean again. If the saved node is pruned the document counts as dirty, which is the safe answer.
- **Macro recording** lives in the history because only the history knows about merges and undo. While recording, every executed command adds a `CommandRecord`; a merge updates the last record instead of adding one (so a drag records as one step); undo removes the undone step's record; non-undoable commands are recorded too. The result is a macro that matches what the user ended up doing, not every keystroke.
- `HistorySummary` (can undo, can redo, names, dirty) is sent to the UI in a `history:changed` event. `HistoryView` is the whole tree for the history panel.

Alternatives considered:

- *Linear undo stack.* Simpler, but loses work whenever someone undoes and changes something, which the plan explicitly rules out.
- *Snapshots as full copies.* Robust against pruning, but expensive for big documents. Pinning gives the same guarantee for free until memory is actually tight.
- *Dropping snapshots when pruning.* Keeps memory strictly bounded, but silently deleting something the user named felt worse than using more memory. The settings page can show history memory use later.

## 5. Event bus

File: `crates/vellum-core/src/events.rs`.

```rust
pub trait Event: Serialize + Clone + Send + Sync + 'static {
    const NAME: &'static str; // "job:changed"
}
```

- **One trait, many event types**, each defined next to the code that emits it (`JobEvent` in jobs, `HistoryChanged` in history). A single big enum in core would force core to know about every crate's events.
- **Rust subscribers get typed channels.** `bus.subscribe::<JobEvent>()` returns a `Subscription<JobEvent>`. Channels rather than callbacks mean an emitter can never deadlock on a subscriber that takes a lock, and a slow subscriber cannot block the emitter. Dropped subscriptions are cleaned up on the next emit.
- **Sinks get JSON.** The desktop app adds a `TauriSink` that forwards every event to the webview with `app.emit(name, payload)`. Events are serialised once per emit, and only if a sink exists.
- **The UI can send events too, but only whitelisted ones.** `bus.accept_from_ui::<UiReady>()` registers a type; `dispatch_from_ui(name, json)` deserialises into it, so a bad payload is rejected at the boundary. These go to Rust subscribers only, never back out to sinks, so nothing echoes. User actions do not use this path; they are commands.
- **Names use colons** (`job:changed`) because Tauri only allows letters, digits, `-`, `/`, `:` and `_` in event names.
- **Types are generated, names are checked.** Payload types are exported to `ui/src/bindings/` by `ts-rs` whenever `cargo test` runs. The name to type map lives in `ui/src/lib/events.ts`; a Rust test (`vellum-core/src/ui.rs`) fails if any event name is missing from it. CI deletes the bindings before running tests and fails if the regenerated files differ from what is committed.

Alternatives considered:

- *`tauri-specta`.* Generates typed command and event bindings straight from Tauri, but it (and `specta` under it) is still a 2.0 release candidate, and it would tie the core crates to Tauri. `ts-rs` works on plain types in any crate.
- *Callback subscribers.* Lower latency, but reentrancy and lock ordering get hard fast, and the latency of a channel hop is nothing next to a frame.
- *Bounded channels.* Would cap memory if a subscriber stops reading, but would mean either blocking emitters or dropping events. Subscribers are expected to drain or drop their subscription; revisit if one ever misbehaves.

## 6. Settings

File: `crates/vellum-core/src/settings.rs`.

The settings file is one JSON object with a `version` field next to the settings:

```json
{ "version": 1, "appearance": { "theme": "dark", "uiScale": 1.0 }, "jobs": { "maxCpuJobs": 2 } }
```

- **Generic store, typed schema.** `SettingsStore<S: SettingsSchema>` handles loading, migrating, saving and events. `AppSettings` is the app's schema. Other files that need the same treatment (shortcut presets, workspace layouts) can reuse the store with their own schema.
- **Most changes need no migration.** Every struct uses `#[serde(default)]`, so new fields get their default and removed fields are ignored. A migration is only needed when a field is renamed, moved or changes meaning. Migrations run on the raw JSON map before deserialising (`Migration { from, run }`), one step per version, so old files can be upgraded any number of versions in one load. A test checks every version step has a migration.
- **`sanitize()`** clamps or resets out-of-range values after every load and every update (UI scale, accent colour format, job limits, history limits).
- **Never lose the user's file:**
  - every write goes to `settings.json.tmp`, is flushed with `sync_all`, then renamed over the real file, so a crash mid-write cannot leave half a file;
  - a migrated file is backed up as `settings.json.v{old}.bak` before it is rewritten;
  - a file from a newer Vellum is read as well as possible and backed up before being rewritten, so going back to the newer version loses nothing;
  - a file that cannot be parsed or migrated is moved to `settings.json.corrupt-{timestamp}` and defaults are used. Startup never fails because of settings.
- **The UI sends JSON merge patches** (RFC 7396), for example `{ "appearance": { "theme": "light" } }`. The store applies the patch, validates by deserialising into the real type, sanitises, saves, then emits `settings:changed` with the full result. The UI store simply adopts what Rust sends back.
- **Updates are all or nothing.** If saving fails, the in-memory settings do not change.
- No-op updates do not write or emit.
- **Phase 1 additions need no migration.** `appearance.compact`, `shortcuts` (preset plus overrides) and `layout` (the current dock layout, the name it came from, saved layouts) are new fields with defaults, so `VERSION` stays 1 and a phase 0 file loads as is (tested). The default accent changed to the darkroom amber; files that already store an accent keep theirs.
- **What lives where.** Shortcuts and layouts are small and change rarely, so they stay in `settings.json`. Recent files change on every open and can grow, so they use the same store type in their own `recent.json` (`recent.rs`), with their own `recent:changed` event.

Alternatives considered:

- *SQLite for settings.* Already planned for caches and the library, but a JSON file is easy to read, diff, back up and fix by hand, which matters more for settings.
- *Lenient per-field parsing* (keep good fields when one has the wrong type). Nicer recovery, but a lot of code for a rare case. The whole file is backed up instead, so nothing is lost.

## 7. Job queue

Files: `crates/vellum-core/src/jobs/` (`mod.rs` types, `queue.rs` scheduler, `eta.rs` time estimates).

A job is a function plus a spec:

```rust
let id = queue.submit(
    JobSpec::cpu("Export holiday.mp4 to 10 MB").with_priority(JobPriority::High),
    move |ctx: &JobContext| {
        for frame in 0..frames {
            ctx.checkpoint()?;                    // pause and cancel take effect here
            ctx.set_progress_steps(frame, frames);
            encode(frame)?;
        }
        Ok(json!({ "path": out_path }))           // shown as "open when done"
    },
);
```

### Resources and limits

Each job needs one slot of a `JobResource`: `Cpu` (encoding, decoding, image maths), `Gpu` (renders, AI inference) or `Io` (downloads, big copies). Every resource has its own limit, so a long GPU upscale never stops a quick CPU conversion from starting. Defaults are 2 CPU, 1 GPU and 4 IO jobs: encoders and AI runtimes already use many threads each, and two GPU jobs would mostly fight over VRAM. Limits come from settings and follow them live.

`Io` is not in the plan's wording (which says GPU vs CPU), but phase 7 model downloads and phase 2 cache work would otherwise either block real CPU work or bypass the queue.

### Scheduling

- One `Mutex<State>` plus one `Condvar`. There is no scheduler thread: every change calls `pump()`, which starts as many waiting jobs as the limits allow.
- **One OS thread per running job.** Media jobs run for seconds to hours, so thread start-up cost is irrelevant, and a blocking FFmpeg or ONNX call cannot starve other jobs the way it would in a fixed pool or an async runtime.
- **Order:** waiting jobs are a list in start order. New jobs go after everything of the same or higher priority (`Low`, `Normal`, `High`, `Urgent`). The user can drag a job anywhere (`move_before`), and that order wins. Changing a priority moves the job to the end of its new band. A blocked GPU job at the head does not hold back CPU jobs behind it.

### Pause, cancel and retry

- **Cooperative.** `ctx.checkpoint()` returns `Err(Cancelled)` once cancel is requested (pass it on with `?`) and blocks while paused. A job that never checks just runs to the end, so long-running code must call it in its loops. Checking is two atomic loads, so calling it per frame is fine.
- **A paused job gives its slot back** and other jobs can run. When resumed, it queues for a slot ahead of other jobs of its priority and continues from the exact point it stopped, with all its local state intact.
- States reflect reality: `Pausing` and `Cancelling` mean the request is in but the job has not reached a checkpoint yet.
- **Automatic retries:** `JobError::failed` is retryable, `JobError::fatal` is not (a missing input file will not appear by trying again). Retryable failures retry up to `max_retries` times with doubling delays (capped at 5 minutes). The job keeps its place in line while it waits. Pausing a job that then fails parks it as `Paused` instead of retrying behind the user's back.
- **Manual retry** runs a failed or cancelled job again from the start. Tasks are `FnMut` rather than `FnOnce` for exactly this reason.
- **Panics are caught** with `catch_unwind` and turn into a failed job with the panic message. A crashing job never takes the app down, and its slot is always released.
- On app exit the shell cancels every job so nothing is left half written.

### Progress, ETA and events

- `set_progress` takes 0 to 1 and is cheap enough to call per frame. Progress-only updates reach the UI at most every 100 ms per job; state changes are always sent at once.
- ETA uses an exponential moving average of progress per second (weight 0.3, samples at least 200 ms apart, no estimate in the first second), measured on running time only, so pauses do not distort it. If progress goes backwards (a second encoding pass) the measurement restarts.
- Events are `JobEvent::Updated { job }` with a full `JobSnapshot`, `Removed { id }` and `Order { waiting }`. Full snapshots rather than diffs mean the UI cannot get into a wrong state by missing one event.
- **Events are sent after the lock is released,** so a slow listener cannot stall the queue. The price is that events from different threads can arrive out of order, so every event carries a `revision` from one global counter and the UI ignores anything older than what it has. The UI also remembers the revision a job was removed at, so a late update cannot bring it back.
- **Startup:** the UI subscribes first, then fetches `JobsOverview { jobs, waiting, revision }` (taken under one lock) and merges it, keeping any event that arrived meanwhile with a higher revision. Nothing is lost in the gap.

Alternatives considered:

- *Tokio or another async runtime.* Most heavy work here is blocking native code (FFmpeg, ONNX, image codecs), which would need `spawn_blocking` anyway. Threads keep the model simple and debuggable.
- *A fixed thread pool (rayon).* Rayon is right for data parallelism inside a job, not for scheduling long jobs, and one blocking job would occupy a pool thread indefinitely.
- *Preemptive pause* (suspending threads or processes). Not portable for threads, and unsafe while holding locks or GPU resources. Phase 2 may still pause an FFmpeg child process with OS signals inside a job, behind the same `checkpoint` API.
- *Diff events instead of snapshots.* Smaller, but a missed or reordered diff corrupts the UI state. Snapshots are a few hundred bytes.

## 8. Desktop shell

Directory: `apps/desktop/` (crate `vellum-desktop`, binary `vellum`).

- `AppState` holds the `AppServices` from `vellum-core` (section 12) and a counter for window labels. It is created in Tauri's `setup` hook and managed by Tauri. Everything that used to live here in phase 0 (job limit following, logging the ready event) moved into `AppServices` so it is tested.
- `TauriSink` forwards bus events to every window.
- IPC commands (`src/ipc.rs`) stay thin: parse, call into the core, map errors to strings with `IpcError`. The list covers settings, app commands (`list_commands`, `run_command`, `get_keymap`), documents and their history, recent files, recovery, notifications, job results and the job queue, plus `start_sample_job`, which only exists so the queue can be tried by hand until real jobs arrive in phase 2. The sample job now writes a small text file as its result so "Open" can be tried too.
- **Plugins:** `tauri-plugin-dialog` for the open file dialog and `tauri-plugin-opener` for opening job results. The opener is only called from Rust, from `open_result`, which refuses any path that is not the result of a job in the queue. The webview has no opener permission of its own, so it cannot be used to open arbitrary files.
- **Thumbnails** use the asset protocol with an empty scope. Rust allows each file one at a time as it is opened or listed in recent files (`allow_asset`), so the webview can show those images and nothing else.
- **Windows:** the first window is `main`. `tear_off_document` builds a new window labelled `doc-1`, `doc-2` and so on, loading the same UI, and moves the document there. The capability file covers `main` and `doc-*`. When a document window is destroyed its documents move back to `main`, and closing `main` quits the app.
- On exit the shell calls `AppServices::shutdown`: stop autosave, cancel jobs, write the last autosave.
- Errors reach the UI as plain strings via `IpcError`.
- The content security policy only allows the app's own scripts and the IPC and asset protocols.
- **The crate has no tests, on purpose.** `tauri-build` embeds the Windows app manifest only into the main binary, and a test binary that links Tauri can then fail to start on Windows. Everything worth testing lives in `vellum-core` or the other crates. The lib and bin targets set `test = false` so `cargo test --workspace` does not try to run them.
- Logging uses `tracing`. Set `VELLUM_LOG=debug` (or any `tracing` filter) to change it.

## 9. UI

Directory: `ui/` (Vite, React 19, TypeScript strict with `noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`).

- `src/lib/ipc.ts` wraps every Rust command with its generated types. `src/lib/events.ts` maps event names to payload types and gives `onRustEvent` and `sendUiEvent`.
- `src/connect.ts` runs once per window at start-up: subscribe to every event and to file drops, then load settings, jobs, commands, the keymap, documents, recent files, notifications and the autosave status, then ask for a recovery offer (main window only), then send `ui:ready`.
- State lives in small zustand stores (`src/state/`). Anything with tricky logic is a pure function next to its store (`applyJobEvent`, `nextActive` for tabs) so it is unit tested without React.
- Pure logic lives in `src/lib/` and is tested there: keyboard chords (`keys.ts`), fuzzy search (`fuzzy.ts`), drop routing (`routing.ts`), the dock layout reducer (`dock.ts`) and accent colour contrast (`color.ts`).
- `src/shell/` is the app frame (section 18). `src/workspaces/` holds the workspace and quick tool catalogues and the placeholder views that later phases replace.
- Radix UI primitives (tooltip, dialog, menubar, context menu, dropdown, popover, switch) with Tailwind 4 and lucide icons. Inter is bundled with `@fontsource-variable/inter`, so the type looks the same on every OS and the CSP needs no outside fonts.
- `components/Button.tsx` requires a `tooltip` prop, and `IconButton` requires a `label` that doubles as its tooltip and accessible name. Both take a `command` prop that shows that command's current shortcut, read live from the keymap, so tooltips follow the user's shortcut changes.
- Outside the desktop app (a plain browser) the UI says so instead of failing.
- ESLint uses `typescript-eslint`'s `strictTypeChecked` preset plus the React hooks rules. Generated bindings and the e2e fixture are excluded from ESLint and Prettier.

## 10. Testing and CI

- `cargo test --workspace` covers commands, history, the event bus, settings (including migrations, corruption and failed writes), the job queue, and from phase 1 the document manager, file routing, shortcuts and presets, recent files, notifications, autosave and recovery (crashes, clean exits, live sessions in another window, broken folders, unknown document types), the app command registry and `AppServices` start-up and shutdown.
- Vitest with Testing Library covers the stores and pure logic listed in section 9, the shell, the jobs panel and the palette's item list.
- **Playwright** (`ui/e2e/`, `pnpm e2e`) covers drop routing, the command palette and crash recovery end to end (section 21).
- Rust tests also write generated files the UI reads: the TypeScript bindings, `ui/src/bindings/fileKinds.json` and `ui/e2e/fixtures/backend.json`. A file is only rewritten when its contents change.
- CI (`.github/workflows/ci.yml`) runs on Windows, macOS and Linux: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, ESLint, Prettier, `tsc`, Vitest and a production build of the UI. The Linux job also installs the Tauri system packages and checks the generated files are committed and current. A separate Linux job installs Chromium and runs Playwright, keeping the report when something fails.

## 11. Known limits and notes for later phases

- **Placeholder documents.** Every workspace opens a `PlaceholderDocument` with a title and notes, so undo, redo, autosave and recovery can be tried for real. Save only marks it saved because there is no file format to write. Phase 2 and the workspace phases add real document types with `DocumentManager::register_type`.
- **Document state is JSON.** `DocumentModel::to_state` returns JSON, which is fine for placeholders. Big documents (a 50 megapixel image) should write binary sidecar files next to the manifest in their own phase, and autosave should then only write documents whose revision changed.
- **One lock for all documents.** `DocumentManager` holds one mutex. Edits are instant today; once a command can take long (a filter on a huge image), documents need their own locks or the work belongs in a job.
- **Autosave runs on its own thread, not as a job.** It writes a few small files and must not fill the jobs panel every minute. When document states get big it should become a hidden IO job so it shares the IO limit.
- **Thumbnails are placeholders.** Recent images under 20 MB show themselves through the asset protocol; everything else shows a tile with its workspace icon. Phase 2 should generate cached thumbnails as jobs.
- **Torn off windows keep no layout.** They show tabs and the document, no rail or docks. Window titles are set when the window opens and do not follow a rename yet.
- **Drag and drop between windows** is done by dragging a tab out of a window (new window) or from the tab menu (move to the main window). Dragging a tab straight into another existing window needs native drag support and is left for later.
- **Commands in a workspace context** (like `image.brush`) are supported by the keymap and the shortcut handler, but no workspace registers any yet.
- **macOS uses Command for Ctrl.** Chords store `Ctrl`, and on macOS the Command key counts as Ctrl. The real Control key on a Mac also counts as Ctrl for now.
- **Pause is cooperative.** Every long loop in a job must call `ctx.checkpoint()`. Code that blocks for minutes in one native call (an FFmpeg pass) needs its own way to stop, for example killing or signalling the child process when `ctx.is_cancelled()` turns true.
- **Snapshots pin history.** A user who keeps an early snapshot keeps every step after it. If that becomes a memory problem, store a full document state in the snapshot and unpin it.
- **Event subscriptions are unbounded channels.** Fine for the current subscribers; anything that subscribes and stops reading leaks memory. The settings and notification followers live as long as the process by design.
- **The window has only been run on Linux,** under a virtual X display (Xvfb, with openbox as the window manager for the close tests) in a headless container. Dropping files from a real file manager was not possible there, so the drop path was tested through the same Tauri events in Playwright. It has not been opened on Windows or macOS yet.

## 12. App services

File: `crates/vellum-core/src/app.rs`.

`AppServices::start(paths)` opens everything a running Vellum needs and returns one cheap to clone handle: the event bus, settings, recent files, the job queue, the document manager, the notification centre, recovery and the app command registry. `shutdown()` stops autosave, cancels jobs and writes the last autosave.

- **Start-up never fails on a bad file.** Only an unusable settings folder stops start-up. Unreadable settings are reset and reported as a notification; a recovery folder that cannot be created turns autosave off and says so.
- **Background threads hold only what they need.** The settings follower (job limits, history limits, and a new `keymap:changed` when shortcuts change) holds the job queue, the documents and the bus. The autosave thread holds an `Autosaver` with the recovery handle, and is stopped and joined on shutdown, so dropping the services really releases the recovery lock. An earlier draft let the threads hold a full clone of the services, which kept the lock alive forever.
- **Opening files** lives here (`open_files`), not in the shell: one document per supported file in the workspace that fits it, or one quick tool session for all of them; files already open are not opened twice; unsupported files come back with a reason in plain words; every opened file goes to the top of the recent list.

Alternatives considered:

- *Keep wiring in the desktop crate.* It worked in phase 0 with three services. With eight, the wiring is real logic (what follows what, what happens on a bad file) and it could not be tested there.

## 13. Documents

File: `crates/vellum-core/src/documents.rs`.

`DocumentManager` owns every open document. Each one is a `DocumentModel` (title, state for autosave, its own `CommandRegistry`) wrapped together with its own `History`, behind an object safe `OpenDocument` trait so documents of different types live in one list.

- **One history per document**, as section 3 planned. Undo in one tab never touches another. History limits from settings reach every open document.
- **Rust owns the tab list.** `documents:changed` sends every open document in tab order with its window, title and dirty flag. Moving a tab (reorder, tear off, back to main) is a Rust call, so every window agrees. Which tab is active is per window and stays in the UI.
- **Events are emitted while the lock is held**, unlike the job queue. Document changes come from IPC calls that are already serial, so holding the lock keeps events in the same order as the changes without revision numbers. It is safe because sinks and subscribers never call back into the manager.
- **Every change emits `history:changed`** for the document it touched and `documents:changed` with the full list. The edit menu, the undo and redo buttons, the tab dirty dot and the history panel all follow those.
- **Undo and redo are app commands** (`edit.undo`, `edit.redo`) that take the active document from the UI, so the menu, shortcuts and palette all reach the same code.
- A revision counter goes up on every change so autosave can skip writing when nothing changed.
- `History::mark_unsaved` was added so a document restored after a crash counts as unsaved until the user saves it.

Alternatives considered:

- *Tabs owned by the UI.* Simpler for one window, but with torn off windows two webviews would each own part of the list and moving a tab would need them to agree. One list in Rust avoids that.
- *Separate locks per document.* Better for long edits, unnecessary for now (section 11).

## 14. Autosave and crash recovery

File: `crates/vellum-core/src/recovery.rs`.

Every running Vellum owns a session folder under `recovery/` in the app data folder, holding a `lock` file, `manifest.json` (the open documents, when they were saved, whether Vellum exited cleanly) and one state file per document.

- **An OS file lock tells a live session from a dead one.** Each session holds an exclusive `File::lock` (stable since Rust 1.89) on its `lock` file. On start-up any folder whose lock can be taken belongs to a session that is gone. Two Vellum windows running at once never offer each other's work, which a pid file cannot promise across platforms.
- **Autosave writes atomically** (temp file, flush, rename, the same helper as settings) every N seconds from settings, and only if the document revision changed. State files of closed documents are removed.
- **Nothing is deleted until the user decides.** Old sessions found at start-up are held, locked, until the prompt is answered. Restore opens every document (unsaved ones marked unsaved) and only then deletes the folder; a document that cannot be loaded (an unknown type from a newer Vellum) keeps its folder. Discard deletes them.
- **Crash or quit.** A clean exit writes a last autosave with `cleanExit: true` and deletes the folder only if nothing is unsaved. So the next launch can say either "Vellum did not close properly" or "Vellum was closed with unsaved changes". Like VS Code's hot exit, quitting with unsaved work never loses it, but here the user is asked rather than surprised.
- Torn off windows do not come back; their documents return in the main window.
- `autosave:changed` drives the "Autosaved 2 min ago" note in the status bar, and failures show there with the reason.

Alternatives considered:

- *A single recovery file.* The next session would overwrite the crashed session's data before the user answered the prompt. Per session folders avoid that by construction.
- *Restore silently on a clean exit.* Less clicking, but a user who quit on purpose may not want old tabs back. One prompt with honest wording is calmer than a surprise.

## 15. App commands and UI actions

File: `crates/vellum-core/src/actions.rs`.

Every menu item, shortcut, palette entry and toolbar button runs an app command from one `CommandRegistry<AppContext>`, as section 11 asked for phase 1. `list_commands` sends their `CommandInfo` to the UI for the palette, menus, tooltips and the shortcut editor.

- **Two flavours, one path.** *Rust actions* change something Rust owns (theme, compact mode, zoom, undo, redo, save, clear finished jobs, clear recent files, notifications). *UI actions* only make sense in a window (open the palette, switch workspace, tear off a tab, toggle a panel); running one returns its record in `CommandOutcome.uiActions` and the window performs it. Both go through `run_command`, so a macro recorder (phase 16) sees every step the user took.
- A small `app_commands!` macro declares each command's struct, palette entry and body in one place. Every command is `undoable() == false`; undo belongs to documents.
- `CommandOutcome.message` carries quiet feedback such as "Nothing to undo", shown as a toast.
- Tests check every kind used in a shortcut preset is registered, and that every description is a full sentence without an em dash.

Alternatives considered:

- *UI actions only in TypeScript.* Faster by one IPC round trip (well under a millisecond), but then macros, the CLI and plugins could not see them, and the palette would list commands from two places.
- *One generic `UiAction { kind }` command.* The registry checks a built command reports the kind it was registered under, which a single struct cannot do for many kinds without leaking strings. The macro keeps the explicit, inspectable list.

## 16. Shortcuts

File: `crates/vellum-core/src/shortcuts.rs`.

- **Canonical chords.** Every chord is stored as `Ctrl+Alt+Shift+Meta+Key`, so comparing chords is string equality. `normalize_chord` accepts aliases (`cmd`, `option`, `esc`, `ArrowUp`) from files and users.
- **Physical keys.** The UI builds chords from `KeyboardEvent.code`, named after the US layout, so `Ctrl+Z` stays in the same place on an AZERTY keyboard, as in Photoshop and Resolve.
- **Presets are changes on top of Vellum's own map.** Photoshop (Ctrl+K opens settings, Tab hides panels, export on Ctrl+Alt+Shift+W), Premiere (export on Ctrl+M, workspaces on Alt+Shift+number), Resolve (pages on Shift+2 to Shift+8, export on Shift+8) and Audacity (redo on Ctrl+Y, preferences on Ctrl+P). A chord a preset hands to one command is taken from whatever had it, and a test checks no preset has conflicts.
- **The user's overrides sit on top of the preset** in `AppSettings.shortcuts`, so switching preset keeps personal changes. An empty list unbinds a command.
- **Contexts.** A command whose kind starts with a workspace name (`image.brush`) only fires in that workspace; everything else is global. Two commands clash if they share a chord and either is global or both are in the same workspace. Rust computes the conflict list for the editor's warning; the editor checks a new chord instantly with the same rule in TypeScript before saving.
- **Typing wins.** In a text field, plain keys and editing chords (Ctrl+Z, Ctrl+A, Ctrl+C) stay with the field; app shortcuts like Ctrl+K and Ctrl+S still fire.

## 17. File routing

File: `crates/vellum-core/src/files.rs`.

`FILE_KINDS` maps extensions to a `FileKind` (image, camera raw, GIF, video, audio, vector, PDF, project) and each kind to the workspace that opens it. A Rust test writes the same table to `ui/src/bindings/fileKinds.json`, so the drop overlay in the window and `open_files` (and later the CLI) always agree. GIFs open in the video workspace because the GIF tools work on frames over time; PDFs open with vector tools; projects open in video until phase 2 stores their own workspace.

The quick tools (`ui/src/workspaces/quickTools.ts`) list the kinds they accept, and `planDrop` in `ui/src/lib/routing.ts` turns files plus a target (the window, or one tool card) into what will happen: open in a workspace, open a tool, or a refusal with a reason. The overlay shows that plan while dragging; dropping performs the same plan.

## 18. The shell and design system

Directories: `ui/src/styles/`, `ui/src/shell/`.

**Visual direction: "darkroom".** Graphite surfaces stepping up from the window background, warm off white text, and a safelight amber accent the user can change. The light theme is warm paper with ink text; high contrast is pure black with white text and solid lines. A soft film grain sits behind empty canvases and the home drop zone.

**Tokens** live in `styles/tokens.css` as CSS variables and reach Tailwind through `@theme inline`, so `bg-panel` or `text-fg-muted` follow the theme with no JavaScript: five surface levels, three text levels, lines, accent with a computed readable ink (`--accent-fg`, picked by WCAG contrast in `lib/color.ts`), semantic colours, a type scale in rem (so the UI scale setting scales everything), a 4 px spacing grid, radii, elevations, motion durations under 150 ms for panels, and z layers. Token names avoid Tailwind's own namespaces (`--fs-*` rather than `--text-*`) so the two never fight. `data-theme`, `data-density="compact"` and `data-reduced-motion` on `<html>` switch them; reduced motion also follows the OS setting.

**Layout.** A title bar (logo, menu bar, document tabs, palette button, notification bell), a rail of workspaces, the dock frame around the centre, and a status bar (where you are, autosave, jobs, layout, zoom). The home screen and empty workspaces are views in the centre.

- **Docks** (`lib/dock.ts`, `shell/Dock.tsx`): left, right and bottom docks hold groups of panels shown as tabs. Panels drag by their tab onto another group's tab strip or a dock edge; docks and groups resize by their edges. The layout is a plain value saved in `AppSettings.layout` on pointer up, with built in presets (Default, Editing, Focus) and user saved layouts. Dragging uses pointer events, not HTML drag and drop, because Tauri's file drop handling turns HTML drag and drop off on Windows.
- **Tabs** reorder by dragging and tear off by dragging out of the window or from their context menu; middle click closes. Closing a tab with unsaved changes asks first.
- **File drops** come from Tauri's drag and drop events, which carry real paths and the pointer position. The position is hit tested against `data-drop-tool` cards so a drop on a quick tool opens that tool.
- **Command palette** (`shell/palette/`): fuzzy search over commands, settings, recent files and presets (shortcut presets and layouts), with recent commands first. Titles match fuzzily; keywords (descriptions, categories) need the actual word, because fuzzy matching across long descriptions found nonsense.
- **Settings window** with sections, a search over every setting and every shortcut, and the shortcut editor. Changes are JSON merge patches; removing a shortcut override sends `null`, since merge patches cannot remove a key any other way.
- **Notification centre and toasts.** New notifications pop up as toasts in the main window only, so torn off windows do not repeat them. Errors stay until dismissed.
- **Empty states, skeletons and tooltips.** Every panel and list says what it is for when empty, lists that load show skeletons, and every control has a tooltip that shows its live shortcut.

Alternatives considered:

- *A native menu bar.* Platform menus would look different on each OS and need their own enabled state wiring. The in-app menu bar is driven by the same command list and keymap as everything else; macOS keeps Tauri's default app menu for standard items.
- *A settings window as its own OS window.* It would need its own copy of the stores and focus handling. A large modal reads the same stores and closes with Escape.
- *HTML drag and drop for tabs and panels.* See the docks bullet above.

## 19. Notifications

File: `crates/vellum-core/src/notifications.rs`.

The `NotificationCenter` keeps the last 100 notifications for the session and sends the whole list on every change (`notifications:changed`), like jobs send full snapshots. It follows job events itself: a job that finishes gets a success notification with "Open" when its result has a `path`, a failure gets one with "Retry". Cancelled jobs stay quiet because the user asked for that. It remembers each job's last state so a repeated update does not notify twice, and a retried job can notify again. Start-up problems (settings reset, autosave off) arrive the same way.

## 20. Jobs panel

File: `ui/src/shell/panels/JobsPanel.tsx`.

Built on the phase 0 queue: pause, resume, cancel, retry and remove; waiting jobs reorder by dragging their handle or with "Start sooner" and "Start later" (both call `move_job`); finished jobs with a result path offer "Open" and "Show in folder"; and running jobs have an "Open when done" switch. That switch is a per window wish kept in the UI, not saved, because it only matters while the window watches the job.

## 21. End to end tests

Directory: `ui/e2e/`.

Playwright cannot drive the WebKit webview Tauri uses on Linux, so the e2e tests run the real UI in Chromium against a fake backend (`fakeBackend.ts`). It installs the same globals `@tauri-apps/api` talks to (`window.__TAURI_INTERNALS__`, the event plugin and window metadata) and answers the IPC commands the UI uses.

- **Real data, not copies.** The fake reads `e2e/fixtures/backend.json` (the registered commands, the keymap of every preset and the default settings) and `fileKinds.json`, both written by Rust tests. If a command or default changes in Rust, the e2e tests see it.
- **Drops use Tauri's event names** (`tauri://drag-enter` and so on) with physical pixel positions, so the drop code path is the one the desktop app runs.
- **A reload is a restart.** The fake keeps its state in `localStorage`; reloading without calling its `cleanExit()` leaves the session behind as a crash would, and the next load offers to restore it.
- The tests cover drop routing (workspaces, several files, tool cards, refusals, files already open), the palette (shortcut, keyboard navigation, every registered command, recent files, settings, shortcut presets) and crash recovery (restore, discard, quit with unsaved work, clean quit, undo and redo from the shortcut and the edit menu).

The Rust side of the same features (recovery folders, locks, routing, document history) is tested in `vellum-core`, and the whole app was also run on a virtual display to check it end to end, as noted in section 11.

Alternatives considered:

- *`tauri-driver` with WebDriver.* Drives the real app, but only on Linux and Windows, needs a built binary per run, and cannot fake a crash or a file drop from outside. It is worth adding for smoke tests in phase 18.
- *Tauri's `mockIPC` helper.* It mocks invoke well but events and window metadata less so, and it runs inside the bundle rather than before it loads. A small init script is clearer and lets the tests reload the page.
