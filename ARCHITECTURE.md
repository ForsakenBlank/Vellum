# Vellum architecture

This file explains how Vellum is put together and why. Read it with [PLAN.md](PLAN.md), which says what gets built and in which order. When a later phase changes one of these decisions, update this file in the same change.

## The big picture

```
 ┌──────────────────────────── ui/ (react, typescript) ────────────────────────────┐
 │  zustand stores  <──  reducers  <──  "vellum://event"         invoke("cmd")  ──┐ │
 └─────────────────────────────────────────▲──────────────────────────────────────┼─┘
                                           │ VellumEvent (json)                   │
 ┌──────────────────────── apps/desktop (tauri 2, thin) ──────────────────────────┼─┐
 │  AppState { EventBus, SettingsStore, JobQueue }   <── #[tauri::command] fns ◄──┘ │
 └─────────────────────────────────────────▲────────────────────────────────────────┘
                                           │
 ┌──────────────────────────────── crates/ (rust) ───────────────────────────────────┐
 │  vellum-core: Command, History, EventBus, Settings, JobQueue                      │
 │  vellum-media, vellum-gpu, vellum-image, vellum-timeline, ... (stubs for now)     │
 └───────────────────────────────────────────────────────────────────────────────────┘
```

- **All real logic lives in `crates/`.** The Tauri app only owns the long lived services and turns them into commands. That keeps the core testable with plain `cargo test`, and it means the CLI and plugin host in phase 16 can reuse everything without a webview.
- **`vellum-core` depends on nothing else in the workspace.** Every other crate may depend on it. Feature crates should not depend on each other unless the plan says so (for example `vellum-export` will use `vellum-media`).
- **The UI never holds the truth.** Rust owns documents, settings and jobs. The UI keeps a mirror in zustand stores, fed by events.

## Repo layout and tooling

- One **cargo workspace** at the root (`Cargo.toml`) with `apps/desktop` and every crate in `crates/`. Shared dependency versions live in `[workspace.dependencies]` so the crates never drift apart.
- One **pnpm workspace** at the root (`pnpm-workspace.yaml`) holding `ui/`. The root `package.json` only carries the Tauri CLI, so `pnpm tauri dev` works from the repo root.
- `rust-toolchain.toml` pins the stable channel with rustfmt and clippy.
- `.cargo/config.toml` sets `TS_RS_EXPORT_DIR` so generated TypeScript lands in `ui/src/bindings`.
- `.gitattributes` forces LF line endings so rustfmt and Prettier give the same answer on Windows.

## Commands

Every user action is a `Command<T>` (`crates/vellum-core/src/command.rs`), where `T` is whatever the command edits: an image document, a timeline, a project.

```rust
pub trait Command<T>: AsAny + Send {
    fn kind(&self) -> &'static str;          // "image.layer.rename"
    fn label(&self) -> String;               // "Rename layer"
    fn apply(&mut self, target: &mut T) -> Result<(), CommandError>;
    fn undo(&mut self, target: &mut T) -> Result<(), CommandError>;
    fn merge(&mut self, next: &dyn Command<T>) -> bool { false }
    fn record(&self) -> Result<CommandRecord, CommandError>;
}
```

Decisions:

- **`apply` and `undo`, not `do` and `undo`.** `do` is a reserved word in Rust. `apply` is also called for redo, so a command captures whatever it needs (usually the old value) on its first `apply` and reuses it later.
- **Commands store their own inverse data** instead of the history storing whole document snapshots. Snapshots of a 100 megapixel image per brush stroke would eat memory fast. Commands that touch pixels (phase 5) will store only the changed tiles.
- **`merge` turns a slider drag into one history step.** The history offers the new command to the previous one while a "merge window" is open. Calling `History::seal()` (on pointer up) closes it. Merging only happens between commands of the same `kind`, and never into a node that is saved or named, because the user expects that exact state back.
- **Serialisation through a registry, not a giant enum.** `CommandRecord { kind, args }` is the frozen form used by macros and the CLI. `CommandRegistry<T>` maps each `kind` to a decoder. We looked at two alternatives:
  - one big `enum AnyCommand` would force every crate's commands into `vellum-core`, which breaks the layering above.
  - the `typetag` crate does this automatically but relies on linker tricks that are fragile on some targets.
    An explicit `register::<C>()` call per command is a little more typing and much easier to debug.
- **`CommandInfo`** (title, category, default shortcut) sits next to the decoder so the palette and shortcut editor in phase 1 can list everything from one place.

## History

`History<T>` (`crates/vellum-core/src/history.rs`) is a **tree**, not a stack.

- Undoing a few steps and then doing something new starts a new branch. Nothing is thrown away, so "I undid too far and then clicked something" is always recoverable.
- `redo` follows the branch you most recently came from.
- `jump_to(node)` walks up to the common ancestor (undoing) and down to the target (redoing). Named snapshots are just labels on nodes, so restoring one is a `jump_to`.
- If a command fails while walking, the history stops at the last node that worked and returns the error. The document and the history pointer always agree.
- `mark_saved` and `is_dirty` drive the unsaved marker.
- Macro recording lives here because the history already sees every command. A merged drag records as one step with its final value.
- `entries()` and `state()` return serialisable views (`HistoryEntry`, `HistoryState`) for the history panel and the undo/redo buttons.

Not done yet, on purpose:

- **Memory limits and pruning.** Nothing in phase 0 is large enough to need it. Phase 5 adds a memory budget and prunes the oldest branches that are not on the current path.
- **Persisting history to disk.** Belongs with the project format and autosave in phase 1.

## Events

There is one event type, `VellumEvent` (`crates/vellum-core/src/events.rs`), and one Tauri channel, `vellum://event`.

```json
{ "topic": "job", "payload": { "type": "progress", "id": 4, "progress": 0.4, "etaSecs": 12, "message": null } }
```

- **Adjacently tagged JSON** (`topic` + `payload`) makes a clean TypeScript union the UI can `switch` on.
- **Types are generated, not hand written.** Every type that crosses the bridge derives `ts_rs::TS`. Running `cargo test` regenerates `ui/src/bindings/*.ts`, and CI fails if the committed bindings are stale. Renaming a Rust field therefore breaks the TypeScript build instead of breaking the app at runtime.
- **JSON is camelCase** on the wire and on disk (`#[serde(rename_all = "camelCase")]`), matching TypeScript style.
- **`EventBus<E>`** is a small in process pub/sub. Channel subscribers get every event in order and unsubscribe by dropping their receiver. Callback subscribers run on the publishing thread **after the bus lock is released**, so a callback can publish again or call back into a service without deadlocking.
- The desktop app registers one callback that forwards every event to the webview.
- We considered `tauri-specta` for typed commands and events. It ties the core types to Tauri, and the core must stay usable from the CLI, so `ts-rs` on plain serde types won.

## Settings

`SettingsStore` (`crates/vellum-core/src/settings.rs`) keeps settings as pretty JSON in the OS config folder (`settings.json`).

- **Versioned.** The file has a `version` field. `MIGRATIONS[i]` upgrades raw JSON from version `i + 1` to `i + 2`. Migrations work on `serde_json::Map`, not on the typed struct, because the old shape no longer exists as a Rust type.
- **Forgiving reads.** Every field has a default (`#[serde(default)]`), so a file missing new settings still loads. `sanitize()` clamps hand edited values (UI scale, job limits and so on) back into range.
- **Never lose the user's file.**
  - Writes go to `settings.json.tmp` and are renamed into place, so a crash mid write cannot leave half a file.
  - A file that will not parse is copied to `settings.json.broken-<timestamp>` before defaults are used, and the UI tells the user where the copy is.
  - A file written by a **newer** Vellum is loaded (fields we know are used) but the store goes read only, so an older build never saves over newer settings.
- **Changes** go through `update(closure)` or `patch(json)`. `patch` deep merges a partial object from the UI and validates it by deserialising, so the UI cannot store something the Rust side does not understand. A successful change publishes `VellumEvent::SettingsChanged`. A failed save rolls back the in memory value.

## Job queue

`JobQueue` (`crates/vellum-core/src/jobs/`) runs everything slow: exports, conversions, AI, proxies, thumbnails.

```rust
let id = queue.submit(JobSpec::new("Export clip", JobKind::Gpu), |ctx: &JobContext| {
    for chunk in chunks {
        ctx.checkpoint()?;               // pause blocks here, cancel returns Err here
        encode(chunk)?;
        ctx.set_progress(done / total);
    }
    Ok(Some(json!({ "path": out })))
});
```

Decisions:

- **Kinds with separate limits** (`Cpu`, `Gpu`, `Io`). A long GPU export must never stop a quick CPU thumbnail from starting. Limits come from settings and can change at runtime.
- **One OS thread per running job**, bounded by those limits, instead of an async runtime or a rayon pool. Media work is blocking FFI (FFmpeg, ONNX, wgpu readbacks). Async would add nothing, and a shared rayon pool would let one heavy job starve others. Jobs are free to use rayon inside themselves.
- **Cooperative pause and cancel.** A job calls `ctx.checkpoint()` in its loops. Cancel makes it return `Err(JobError::Cancelled)`; pause makes it block until resumed. Killing threads from outside is not safe in Rust (or with FFmpeg), so this is the only sound option. A paused running job keeps its slot so resume is instant. Pausing a queued job just keeps it from starting.
- **Scheduling** picks the highest priority first, then the oldest. Priority can be changed while a job waits. A retried job goes to the back of its priority band.
- **Retry.** `JobSpec::max_attempts` gives automatic retries. `retry(id)` re runs a failed or cancelled job by hand. This works because the job object is handed back to the queue after every run, so jobs must be safe to run again.
- **Panics are caught** and turned into a failed job with the panic message. One bad decoder cannot take the app down.
- **ETA** comes from progress and active run time (time spent paused does not count). It is a straight line estimate. Phase 2 can smooth it once real encoders report uneven progress.
- **Events are ordered and lock free for subscribers.** Status changes and progress go into a channel while the queue lock is held, and a small dispatcher thread publishes them onto the event bus. Subscribers therefore see `queued`, `running`, `completed` in order, and a subscriber can call back into the queue without deadlocking. Progress events are throttled (100 ms by default) so a chatty job cannot flood the webview.
- **Shutdown** cancels everything and joins the worker threads. The desktop app calls it on exit, and `Drop` calls it too.

Not done yet: reordering inside a priority band (phase 1 job panel), persisting the queue across restarts, and "open when done" or "shut down when done" actions (phase 1 and 13).

## Threads and the UI

- Tauri commands run off the main thread and must return quickly. Anything that can take more than a few milliseconds becomes a job.
- Errors cross into the UI as plain strings written for users (see section 5 of PLAN.md).
- The UI subscribes to `vellum://event` **before** it fetches the first snapshot (`list_jobs`, `get_settings`), so nothing that happens in between is missed. Store updates go through pure reducers (`ui/src/stores/jobs.ts`), which are what the Vitest tests cover.

## Testing and CI

- `cargo test --workspace` covers commands, history, the event bus, settings (including corrupt, newer and migrated files) and the job queue (limits, priority, pause, cancel, retry, panics, throttling, shutdown).
- `pnpm --filter vellum-ui test` runs Vitest on the reducers and formatting helpers.
- GitHub Actions (`.github/workflows/ci.yml`) runs on Windows, macOS and Linux: typecheck, ESLint, Prettier, Vitest, UI build, `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test`, plus the stale bindings check on Linux.
- `clippy.toml` allows `unwrap()` in tests only. The workspace warns on `unwrap_used` and CI turns warnings into errors.
