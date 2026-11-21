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
 crates/vellum-media/         MediaService: ffmpeg, probing, images, cache,
                              thumbnails, waveforms, strip metadata (section 22)
         ▲
 crates/vellum-gpu, -image, -timeline, -audio, -ai,
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

- `AppState` holds the `AppServices` from `vellum-core` (section 12), the `MediaService` from `vellum-media` (section 22) and a counter for window labels. It is created in Tauri's `setup` hook and managed by Tauri. Everything that used to live here in phase 0 (job limit following, logging the ready event) moved into `AppServices` so it is tested.
- `TauriSink` forwards bus events to every window.
- IPC commands (`src/ipc.rs`) stay thin: parse, call into the core, map errors to strings with `IpcError`. The list covers settings, app commands (`list_commands`, `run_command`, `get_keymap`), documents and their history, recent files, recovery, notifications, job results and the job queue, plus `start_sample_job`, which only exists so the queue can be tried by hand until real jobs arrive in phase 2. The sample job now writes a small text file as its result so "Open" can be tried too.
- **Plugins:** `tauri-plugin-dialog` for the open file dialog and `tauri-plugin-opener` for opening job results. The opener is only called from Rust, from `open_result`, which refuses any path that is not the result of a job in the queue. The webview has no opener permission of its own, so it cannot be used to open arbitrary files.
- **Thumbnails** use the asset protocol. Its scope starts empty; Rust allows the media cache's blob folder (thumbnails and filmstrips, only ever written by Vellum) and each file the user opens or has in recent files, one at a time (`allow_asset`). Nothing else on disk can be loaded by the webview.
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

- `cargo test --workspace` covers commands, history, the event bus, settings (including migrations, corruption and failed writes), the job queue, and from phase 1 the document manager, file routing, shortcuts and presets, recent files, notifications, autosave and recovery (crashes, clean exits, live sessions in another window, broken folders, unknown document types), the app command registry and `AppServices` start-up and shutdown. From phase 2 `vellum-media` adds format sniffing, probing (rotation, variable frame rate, HDR), hardware detection, exact frame seeking, every image format, the cache (eviction, stale entries, broken indexes), thumbnails, filmstrips, waveforms, encoding with fallback and cancel, and strip metadata, mostly against real files FFmpeg makes during the test.
- Vitest with Testing Library covers the stores and pure logic listed in section 9, the shell, the jobs panel and the palette's item list.
- **Playwright** (`ui/e2e/`, `pnpm e2e`) covers drop routing, the command palette and crash recovery end to end (section 21).
- Rust tests also write generated files the UI reads: the TypeScript bindings, `ui/src/bindings/fileKinds.json` and `ui/e2e/fixtures/backend.json`. A file is only rewritten when its contents change.
- CI (`.github/workflows/ci.yml`) installs FFmpeg and runs on Windows, macOS and Linux: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, ESLint, Prettier, `tsc`, Vitest and a production build of the UI. The Linux job also installs the Tauri system packages and checks the generated files are committed and current. A separate Linux job installs Chromium and runs Playwright, keeping the report when something fails.

## 11. Known limits and notes for later phases

- **Placeholder documents.** Every workspace opens a `PlaceholderDocument` with a title and notes, so undo, redo, autosave and recovery can be tried for real. Save only marks it saved because there is no file format to write. Phase 2 and the workspace phases add real document types with `DocumentManager::register_type`.
- **Document state is JSON.** `DocumentModel::to_state` returns JSON, which is fine for placeholders. Big documents (a 50 megapixel image) should write binary sidecar files next to the manifest in their own phase, and autosave should then only write documents whose revision changed.
- **One lock for all documents.** `DocumentManager` holds one mutex. Edits are instant today; once a command can take long (a filter on a huge image), documents need their own locks or the work belongs in a job.
- **Autosave runs on its own thread, not as a job.** It writes a few small files and must not fill the jobs panel every minute. When document states get big it should become a hidden IO job so it shares the IO limit.
- **Thumbnails come from the media cache** (section 22) for every kind of file. A file with no picture (audio without cover art) or one that failed keeps its workspace icon tile.
- **Media limits for later phases** (section 22 has the detail): playback and scrubbing need an in-process decoder (phase 8); pause reaches a running FFmpeg on Linux and macOS but only between passes on Windows; HDR thumbnails are not tone mapped yet (phase 4 colour management); huge images are refused rather than tiled (phase 5); HEIC needs libheif tools or FFmpeg 7.1 until the installer bundles a new enough FFmpeg (phase 18); camera raw decoding is covered by rawler but has no test file, since no small raw can be made in a script.
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
- The tests cover drop routing (workspaces, several files, tool cards, refusals, files already open), the palette (shortcut, keyboard navigation, every registered command, recent files, settings, shortcut presets), crash recovery (restore, discard, quit with unsaved work, clean quit, undo and redo from the shortcut and the edit menu) and the info panel (media details, private tags, strip metadata). The fake answers `probe_media` with a plausible `MediaInfo` per file kind and never finishes thumbnails.

The Rust side of the same features (recovery folders, locks, routing, document history) is tested in `vellum-core`, and the whole app was also run on a virtual display to check it end to end, as noted in section 11.

Alternatives considered:

- *`tauri-driver` with WebDriver.* Drives the real app, but only on Linux and Windows, needs a built binary per run, and cannot fake a crash or a file drop from outside. It is worth adding for smoke tests in phase 18.
- *Tauri's `mockIPC` helper.* It mocks invoke well but events and window metadata less so, and it runs inside the bundle rather than before it loads. A small init script is clearer and lets the tests reload the page.

## 22. Media core

Crate: `crates/vellum-media`. Phase 2.

```
 MediaService (service.rs)            one per app, created by the desktop shell
   ├─ FfmpegTools (ffmpeg/)           finds and runs ffmpeg and ffprobe
   │    probe.rs   ffprobe json  ──>  MediaInfo (info.rs)
   │    hw.rs      trial encodes ──>  HardwareSupport
   │    seek.rs    packet times  ──>  FrameIndex, exact frames
   │    encode.rs  progress, part files, hardware to software fallback
   ├─ images.rs                       every still and animated image format
   ├─ MediaCache (cache.rs)           sqlite index + blob files
   ├─ thumbnail.rs, waveform.rs       what the cache holds
   └─ strip.rs                        copies without metadata
```

### FFmpeg as programs, not libraries

The plan asked for `ffmpeg-next` (linking the FFmpeg libraries). Vellum runs the `ffmpeg` and `ffprobe` executables instead, for now, for these reasons:

- **Licensing.** An in-process FFmpeg has to be an LGPL build, which leaves out x264 and x265, the best H.264 and H.265 encoders and exactly what "Export to 10 MB" needs. A separate GPL `ffmpeg` program shipped next to the app is the usual way around that (HandBrake, Shotcut and many converters do the same).
- **Crash isolation.** A damaged or hostile file can crash FFmpeg. As a child process that is a failed job with a message; in process it takes the app down with unsaved work.
- **Cancel and pause.** A child process can be killed at once and, on Linux and macOS, suspended with `SIGSTOP`. A blocking library call can only be abandoned.
- **Builds.** CI and contributors need no FFmpeg development headers on three operating systems. Any FFmpeg from 6.0 on works, and users can point Vellum at a newer one.

The cost is speed for one specific job: real time playback and scrubbing (phase 8) need decoded frames in memory without starting a process per frame. Phase 8 should add an in-process **decoder** for that (an LGPL build is fine for decoding, since every decoder Vellum needs is in it) behind the same `FrameRequest` shape, and keep the programs for encoding, probing and everything else. `FrameIndex` already gives it exact frame times.

**Finding FFmpeg** (`FfmpegTools::locate`): the folder or binary in Settings > Media, then `VELLUM_FFMPEG_DIR`, then the folder Vellum's own executable is in (where the installer will put the bundled copy in phase 18), then Homebrew and MacPorts folders on macOS and WinGet, Scoop and Chocolatey folders on Windows (apps started from the dock or start menu do not get the shell's `PATH`), then `PATH`. Each candidate must answer `ffmpeg -version`.

**Running them** (`ffmpeg/process.rs`): every call gets `-nostdin -loglevel error`, no console window on Windows, stdin closed, and stderr drained on a thread into a 30 line tail for error messages (with FFmpeg's `[mov @ 0x55...]` prefixes and repeated paths removed). Jobs pass their `JobContext` as a `Reporter`; quick calls from the UI pass `NoReporter`.

- `run_lines` reads `-progress pipe:1` output and checks cancel and pause ten times a second. Cancel kills the child. Pause sends `SIGSTOP`, blocks in `checkpoint` (the job gives up its slot like any paused job) and sends `SIGCONT` on resume. Windows has no way to suspend another process from safe Rust, so there a pause takes effect when the current pass ends; the job shows "Pausing" until then.
- `run_reading` hands stdout to a reader (raw frames, audio samples, PNG bytes). Readers call `checkpoint` between chunks, and while they wait the pipe fills and FFmpeg blocks, which pauses it on every platform.
- A `Running` child that is dropped early is always killed, so no FFmpeg is left behind a job that gave up.
- `JobContext::is_pause_requested` was added to the core for this: a job has to suspend its child before it blocks in `checkpoint`.

### Probing

`probe_media` runs `ffprobe -show_format -show_streams` and maps the JSON into `MediaInfo`: kind, format, size, duration, bit rate, the first video and audio streams, every stream, and every tag (flagged `sensitive` when it looks like location, serial numbers, owner or camera details). Video details include:

- **Shown size.** The display matrix (FFmpeg 5 and later) or the old `rotate` tag gives the clockwise rotation, and non square pixels widen the picture, so a portrait phone video reports 1080 × 1920.
- **Variable frame rate.** A second, quick ffprobe run reads the timestamps of the first 600 packets. Sorted, the gaps should all match the median within a tick or two (33 and 34 ms at 29.97 fps is still constant). A file whose average and base rates disagree by more than 2% also counts as variable, which catches rate changes later in the file.
- **HDR.** PQ is HDR10, HLG is HLG, a Dolby Vision configuration record wins. For HDR files a third run reads the first frame's side data for mastering display and content light levels, and HDR10+ metadata.

Images are probed without FFmpeg where possible (`images::probe_image`): header sizes, colour type, bit depth, ICC profile, EXIF (through `kamadak-exif`, including orientation and DPI) and, for GIF, animated WebP and APNG, frame counts, durations and loop counts read straight from the container (`animation.rs`) without decoding a pixel. Probe results are cached, so the info panel is instant the second time.

### Hardware encoders

`ffmpeg -encoders` lists what the build includes, not what works: NVENC is listed on machines without an NVIDIA card. `detect_hardware` therefore tries every listed NVENC, Quick Sync, AMF, VideoToolbox and VA-API encoder with a three frame encode, all in parallel with a 10 second limit, and opens each hardware decoder's device. It takes a second or two, so the result is saved as `hardware.json` next to the cache and only checked again when the FFmpeg version changes or the user presses "Check hardware again". `HardwareSupport::encoder_chain` gives the encoders to try for a codec, hardware first (when the setting allows) and software last, and `run_encode` walks that chain: if a hardware encode fails part way, the job says so and starts again in software. Encodes write to `.name.part.ext` and rename at the end, and never overwrite.

### Frame accurate seeking

`FrameIndex::build` lists the presentation timestamp of every packet in a video stream (`ffprobe -show_entries packet=pts,dts,flags`, no decoding, so a feature film takes seconds), sorted into display order, with keyframes marked and packets FFmpeg will discard (edit lists) left out. Frame `n` is then an exact time, which is right for variable frame rate video where `n / fps` drifts.

`extract_frame` asks for the frame at a time by seeking half a frame before it: FFmpeg decodes from the keyframe before and drops every frame that starts earlier than the seek point, so the first frame out is the one asked for, with no rounding risk at the boundary. A test encodes 50 frames whose grey level is their frame number, with B-frames and a 12 frame keyframe gap, and checks ten of them come back exactly. The quick mode (`accurate: false`) seeks to the keyframe before and decodes only that frame, for thumbnails. Raw output needs `-fps_mode passthrough`, or constant frame rate sync throws that keyframe away.

### Images

One call, `decode_image`, for every format, returning frames (all of them for animations when asked) as `DynamicImage` so 16 bit and float images keep their depth:

| Format | Decoder |
|---|---|
| PNG, APNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, TGA, PNM, HDR, EXR | `image` crate |
| SVG | resvg at any size, with system fonts loaded once |
| PSD | `psd` crate, the stored composite, or the layers flattened when the file has none |
| Camera raw | rawler: a full develop, or the embedded preview for thumbnails |
| AVIF, JPEG XL | FFmpeg |
| HEIC | libheif's `heif-dec`/`heif-convert`, then FFmpeg 7.1 or newer, then `sips` on macOS |

- **Formats are sniffed from the first bytes** (`sniff.rs`), with the extension as a fallback, so a JPEG saved as `.png` still opens. Raw extensions win over the TIFF magic that most raw files share.
- **Memory is checked before decoding.** The header gives the size, and an image that would need more than the "Largest image" setting (2 GB by default) is refused with a message saying how big it is. Huge images get tiled loading in phase 5, the image editor; until then this keeps a 30000 × 30000 TIFF from taking the machine down.
- **Third party decoders run inside `catch_unwind`.** Raw and PSD decoders can panic on odd files; that becomes "This file is damaged" rather than a crash.
- EXIF orientation is applied, so photos come out upright.
- HEIC goes through external tools because FFmpeg only reads HEIC from 7.1 (Ubuntu 24.04 ships 6.1), and the libheif crate would add another native library to every build. Phase 18 should bundle an FFmpeg new enough to make the fallbacks unnecessary.

### The cache

`MediaCache` keeps generated things on disk: an SQLite index (`index.sqlite`, through `rusqlite` with SQLite bundled, so every platform has the same version) plus blob files in `blobs/`.

- **Keys** are the file's canonical path, size and modified time, plus what was made (`kind`, eg `thumb`) and with which settings (`params`, eg `320`). Editing a file changes its size or time, so a stale entry is never served: a lookup that finds an entry for an older version deletes it.
- **Writes are atomic.** A blob is written under a temp name and the index row is only added once it is complete, so a cancelled job never leaves half a thumbnail. A crash between the two leaves an orphan file, which the next start sweeps up.
- **Least recently used eviction** whenever the total goes over the limit (Settings > Media, 2 GB by default). Last-used times are only written once a minute per entry, so browsing a folder of thumbnails does not turn every read into a write.
- **A broken index is replaced, not fatal.** If it cannot be opened, fails `PRAGMA quick_check` or comes from a newer schema, it is moved aside, the blobs are cleared and a fresh index starts. The cache is disposable by design.
- WAL mode and a two second busy timeout cover two Vellum windows or instances sharing the cache.
- It lives in the OS cache folder (`app_cache_dir()/media`), which backup tools and system cleaners know they may skip or empty.

### Thumbnails, filmstrips and waveforms

- **Thumbnails** fit a square (320 for the home screen). Images go through `decode_image` with the raw preview shortcut. Videos use the quick keyframe grab at 10% in, then 30% and 50% if the frame is nearly black, so fade-ins do not give black tiles. Audio files show their cover art when they have one; otherwise the cache remembers there is nothing to show so the file is not opened again. FFmpeg returns the frame as PNG, so it does the rotation and pixel aspect maths. Opaque thumbnails are stored as JPEG, ones with transparency as PNG.
- **Filmstrips** are evenly spaced frames tiled into one JPEG, with the tile size and times as JSON. Frames are grabbed four at a time, since decoding one keyframe barely uses more than one core.
- **Waveforms** decode the first audio stream to 32 bit float (mixed down to at most stereo) and keep the minimum and maximum of every 256 samples, about 5 ms, as 16 bit pairs. Very long files use coarser peaks so no entry passes about 8 MB. The UI asks for as many columns as it has pixels and gets them merged down, so zooming out never misses a peak.

All three run as **background jobs**, a small addition to the phase 0 queue: `JobSpec::in_background()` jobs wait for slots and can be cancelled like any job, but the UI leaves them out of the jobs panel, they never notify, and the queue removes them as soon as they finish (listeners still see the final snapshot first). Without that, opening a folder of 200 photos would fill the jobs panel with 200 thumbnail jobs. They run at low priority, so a user's export starts before waiting thumbnails.

### Strip metadata

`strip_metadata` writes `name-clean.ext` next to the original (`name-clean 2.ext` and so on if that exists) and never touches the original.

- **JPEG, PNG, WebP and GIF are rewritten at the container level,** so not one pixel changes (the tests compare decoded pixels before and after). EXIF, XMP, IPTC, comments, PNG text and time chunks, WebP EXIF and XMP chunks and GIF comment and application blocks go. What is needed to show the picture right stays: JFIF and Adobe markers, ICC profiles, gamma and colour chunks, transparency, animation control and the GIF loop count. A photo whose EXIF said "turn me 90 degrees" gets a one tag EXIF block holding only the orientation, so it still shows upright. Anything after a JPEG's end marker (phones append extra "multi picture" images with their own EXIF) is dropped.
- **Video and audio** go through FFmpeg with stream copy (fast, lossless): global, stream and chapter metadata removed, data tracks dropped (GoPro and some phones keep GPS in one), and `bitexact` so FFmpeg does not add its own encoder tags.
- Other formats say they are not supported yet and suggest converting first.
- It runs as a normal visible job whose result is `{ "path": ... }`, so the notification offers "Open" like any export.

### MediaService and the UI

`MediaService` owns the cache, the tools, the hardware check and a copy of the media settings, which it follows from the settings events (a new FFmpeg folder relocates the tools and checks hardware again; a new cache limit evicts at once). Its requests never wait on slow work: `thumbnail`, `filmstrip` and `waveform` answer from the cache, or start one background job (a pending set stops a second identical job) and answer `None`; the result arrives as a `media:changed` event. The pending entry is cleared before that event is sent, so asking again right after a failure tries again.

The desktop shell creates the service in `AppState`, allows the cache's blob folder in the asset protocol scope (the only folder allowed as a whole, since only Vellum writes there), and exposes `media_status`, `probe_media`, `media_thumbnail`, `media_filmstrip`, `media_waveform`, `strip_metadata`, `clean_media_cache` and `recheck_hardware`. The media commands are `async` and run on a blocking worker, because probing can start ffprobe and Tauri runs plain commands on the main thread.

In the UI, `state/media.ts` holds the status and a table of thumbnails by path and size (`applyMediaEvent` is pure and tested), `useThumbnail` asks once per path and size, the home screen shows cached thumbnails for every file type, the Info panel shows `describeMedia` (in `lib/mediaInfo.ts`) with "Copy info" and "Strip metadata", and Settings > Media and cache shows FFmpeg, the working hardware encoders and cache use, with "Check hardware again" and "Clean cache".

### Testing and numbers

Tests make their own media with FFmpeg (`testing::make`) instead of committing files, and skip with a note when FFmpeg is missing; CI installs FFmpeg on all three systems and sets `VELLUM_REQUIRE_FFMPEG=1` so it fails there instead. `tests/media/generate.sh` makes a folder of files in every format (including an HLG phone clip filmed sideways, variable frame rate, cover art, damaged files) for trying the app by hand, and `cargo run -p vellum-media --example probe_folder -- tests/media/out` runs each through probing and thumbnails.

The real app was also run on Linux under a virtual display with the generated files in its recent list: every format showed a thumbnail (WAV, which has no picture, kept its icon), the Info panel read the HLG phone clip correctly, Strip metadata wrote a clean copy that kept its rotation, and Clean cache emptied the blob folder. That run caught one bug the fake backend could not (the settings section showed the cache as it was at start-up).

`cargo run --release -p vellum-media --example thumbnail_bench` times a 10 second 4K HEVC 10-bit clip. On a 4 thread cloud container with FFmpeg 6.1 and software decoding: probe 110 to 130 ms (three ffprobe runs), thumbnail 435 to 460 ms (almost all of it FFmpeg decoding one 4K keyframe, the same as running the command by hand), about 570 ms in all for the first view; 0.02 ms once cached; a 10 frame filmstrip 1.2 s.

Alternatives considered:

- *`ffmpeg-next` in process.* See above. Right for real time decoding, wrong for everything else today.
- *libvips for large images.* Fast and memory frugal, but another large native dependency per platform for something the image editor's own tiles (phase 5) will do. Phase 2 refuses images over the memory limit with a clear message instead.
- *The cache in one SQLite file with blobs inside.* Simpler to clean, but the webview cannot load a thumbnail from inside a database, and large blobs make SQLite files fragment and grow.
- *Hashing file contents for cache keys.* Survives renames, but hashing a 4 GB video before showing its thumbnail defeats the point. Path, size and modified time is what file managers use.
- *Thumbnail jobs in the jobs panel.* Honest but noisy; see background jobs above.
