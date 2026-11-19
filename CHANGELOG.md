# Changelog

All notable changes to Vellum are listed here, newest first.

## Unreleased

### Phase 0: repo bootstrap and architecture

- Tauri 2 desktop app with a React, TypeScript and Vite front end (zustand, Radix UI, Tailwind). `pnpm tauri dev` from the repo root starts it.
- Cargo workspace with every crate from the plan. `vellum-core` is real, the others are stubs.
- `vellum-core`:
  - `Command` trait with apply, undo, merge (one history step per slider drag) and serialisable records for macros.
  - Branching undo/redo history with named snapshots, jump to any step, unsaved tracking and macro recording.
  - Typed event bus, with TypeScript types generated from the Rust types by ts-rs.
  - Versioned JSON settings with migrations, atomic saves, a backup of unreadable files and read only mode for files from newer versions.
  - Job queue with priorities, per kind limits (CPU, GPU, IO), progress, ETA, pause, resume, cancel, retry and automatic retries.
- A first window with a live jobs panel and a sample job to try it.
- GitHub Actions CI on Windows, macOS and Linux.
- `ARCHITECTURE.md` explaining the design.
