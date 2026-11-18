# Vellum architecture

This file explains how Vellum is put together and why. It is written for whoever works on the next phase, human or AI. Read `PLAN.md` first for the what; this file is the how.

Every section ends with the alternatives that were considered and why they lost, so a later phase can tell a deliberate choice from an accident before changing it.

## 1. The big picture

```
 ui/ (React, TypeScript)
   zustand stores  <──── tauri events ("job:changed", "settings:changed", ...)
   ipc.ts wrappers ────> tauri commands (invoke)
         │
 apps/desktop/ (thin tauri shell)
   AppState { EventBus, SettingsStore, JobQueue }
   TauriSink: forwards every bus event to the webview
         │
 crates/vellum-core/          commands, history, events, settings, jobs
         ▲
 crates/vellum-media, -gpu, -image, -timeline, -audio, -ai,
        -export, -capture, -vector, -automation   (stubs until their phase)
```

Rules that keep this healthy:

- **Dependencies point one way.** `vellum-core` depends on no other Vellum crate. Feature crates depend on core (and on each other only where the plan says so, for example timeline on media and gpu). Only `apps/desktop` knows about Tauri.
- **Logic lives in crates, not in the shell.** `apps/desktop` parses IPC arguments, calls into the core and maps errors to strings. Anything worth testing belongs in a crate so the CLI (phase 16) and tests can use it without a window.
- **The UI is a view.** Rust owns the state of settings, jobs and (from phase 1 on) documents. The UI renders what Rust reports and asks Rust to change things. It never keeps its own copy of the truth that could drift.

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

- `AppState` holds the event bus, the settings store and the job queue. It is created in Tauri's `setup` hook and managed by Tauri.
- `TauriSink` forwards bus events to the webview. A background thread follows `settings:changed` and updates job limits, so the limits follow settings however they get changed.
- IPC commands (`src/ipc.rs`) are thin: `app_info`, `get_settings`, `update_settings`, `reset_settings`, `list_jobs`, `cancel_job`, `pause_job`, `resume_job`, `retry_job`, `remove_job`, `clear_finished_jobs`, `set_job_priority`, `move_job`, `ui_event`, plus `start_sample_job`, which only exists so the queue can be tried by hand until real jobs arrive in phase 2. The UI only shows its buttons in development builds.
- Errors reach the UI as plain strings via `IpcError`.
- The content security policy only allows the app's own scripts and the IPC and asset protocols.
- **The crate has no tests, on purpose.** `tauri-build` embeds the Windows app manifest only into the main binary, and a test binary that links Tauri can then fail to start on Windows. Everything worth testing lives in `vellum-core` or the other crates. The lib and bin targets set `test = false` so `cargo test --workspace` does not try to run them.
- Logging uses `tracing`. Set `VELLUM_LOG=debug` (or any `tracing` filter) to change it.

## 9. UI

Directory: `ui/` (Vite, React 19, TypeScript strict with `noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`).

- `src/lib/ipc.ts` wraps every Rust command with its generated types. `src/lib/events.ts` maps event names to payload types and gives `onRustEvent` and `sendUiEvent`.
- `src/connect.ts` runs once at start-up: subscribe to events, then load settings and the jobs overview, then send `ui:ready`.
- State lives in small zustand stores (`src/state/`). The jobs store is a pure reducer (`applyJobEvent`, `mergeOverview`) wrapped in a store, so the tricky ordering logic is unit tested without React.
- Radix UI primitives (tooltip, toggle group) with Tailwind 4. Colours are CSS variables switched by `data-theme` on the root element, so dark, light and high contrast need no JavaScript beyond setting the attribute. Phase 1 replaces these placeholder tokens with the real design system.
- `components/Button.tsx` requires a `tooltip` prop, so the "every button has a tooltip" rule is enforced by the type checker for normal buttons. The few custom controls (theme toggle, workspace list) wrap themselves in `Tooltip` directly.
- Outside the desktop app (a plain browser, or tests) the UI says so instead of failing.
- ESLint uses `typescript-eslint`'s `strictTypeChecked` preset plus the React hooks rules. Generated bindings are excluded from ESLint and Prettier.

## 10. Testing and CI

- `cargo test --workspace` covers commands, history, the event bus, settings (including migrations, corruption and failed writes) and the job queue (limits per resource, priority, reordering, pause freeing slots, resume waiting for a slot, cancel in every state, automatic and manual retry, panics, throttling, event ordering, ETA). The job queue tests use real threads with generous timeouts and were run 60 times in parallel under load without a failure.
- Vitest with Testing Library covers the jobs reducer, formatting, the jobs panel and the shell.
- CI (`.github/workflows/ci.yml`) runs on Windows, macOS and Linux: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, ESLint, Prettier, `tsc`, Vitest and a production build of the UI. The Linux job also installs the Tauri system packages and checks the generated TypeScript bindings are committed and current.

## 11. Known limits and notes for later phases

- **Phase 1** should register app level actions in a `CommandRegistry` so the command palette has something to search, and keep one `History` per open document behind a document manager that emits `history:changed`.
- **Pause is cooperative.** Every long loop in a job must call `ctx.checkpoint()`. Code that blocks for minutes in one native call (an FFmpeg pass) needs its own way to stop, for example killing or signalling the child process when `ctx.is_cancelled()` turns true.
- **Snapshots pin history.** A user who keeps an early snapshot keeps every step after it. If that becomes a memory problem, store a full document state in the snapshot and unpin it.
- **History limits and settings.** `History::set_limits` exists, but nothing applies history settings to open documents yet, because there are no documents. Phase 1 or 5 should wire it.
- **Event subscriptions are unbounded channels.** Fine for the current subscribers; anything that subscribes and stops reading leaks memory.
- **The window was only launched on Linux,** under a virtual X display (Xvfb) in a headless container. It has not been opened on Windows or macOS yet.
