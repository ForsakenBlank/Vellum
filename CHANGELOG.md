# Changelog

Notable changes to Vellum, newest first.

## Unreleased

### Phase 0: repo bootstrap and architecture

The foundations every later phase builds on. Nothing here is a user facing tool yet.

**Added**

- A Tauri 2 desktop app (`apps/desktop`) with a React, TypeScript and Vite front end (`ui`), using Zustand, Radix UI and Tailwind. `pnpm tauri dev` opens the window.
- A cargo workspace with all eleven crates from the plan. `vellum-core` is implemented, the other ten are stubs.
- The command system: one `Command` type for every user action, with undo, redo, merging of rapid changes such as slider drags, serialisable records for macros, batches, and a registry that feeds the command palette.
- Undo history as a tree, so undoing and then making a change keeps the old branch. Named snapshots, jumping to any state, unsaved change tracking, and limits on entries and memory.
- A typed event bus between Rust and the UI. The TypeScript types are generated from the Rust types and checked in CI.
- A settings store: one JSON file of versioned sections with migrations. Only changed values are stored, unknown sections are preserved, and an unreadable file is moved aside instead of overwritten.
- The job queue: priorities, progress, time remaining, cancel, pause and resume, automatic and manual retry, separate limits for CPU, GPU and IO work, and events for the UI.
- A small shell to prove the pieces work together: a workspace rail with placeholders, a jobs panel, a theme switch saved through settings, and sample jobs to try the queue.
- CI on Windows, macOS and Linux: Prettier, ESLint, TypeScript, Vitest, rustfmt, Clippy, Rust tests, a check that the generated bindings are current, and a smoke test that starts the built app and waits for the UI to connect.
- `ARCHITECTURE.md`, explaining the design decisions and their trade offs.
