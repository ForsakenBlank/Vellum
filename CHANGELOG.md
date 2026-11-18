# Changelog

All notable changes to Vellum are listed here. Dates are in YYYY-MM-DD.

## Unreleased

### Phase 0: repo bootstrap and core architecture (2026-11-18)

Added

- Tauri 2 desktop app in `apps/desktop` with a React, TypeScript and Vite front end in `ui/` (pnpm workspace, zustand, Radix UI, Tailwind).
- Cargo workspace with every crate from the plan. `vellum-core` is real, the others are stubs for their phases.
- Command system: the `Command` trait (apply, revert, merge for slider drags, readable names, serialisable records) and a command registry for the palette, macros, the CLI and plugins.
- Undo history as a tree: branching history, named snapshots, checkout of any step, merge windows, memory and step limits, unsaved tracking and macro recording.
- Typed event bus between Rust and the UI, with TypeScript types generated from Rust.
- Settings store: versioned JSON on disk, migrations, atomic writes, backups before upgrades and recovery from unreadable files.
- Job queue: priorities, manual reordering, progress, time left, cancel, pause and resume (paused jobs free their slot), automatic and manual retry, separate limits for CPU, GPU and IO jobs, and live events to the UI.
- A placeholder window with a working jobs panel and theme switcher, plus sample jobs in development builds.
- CI on Windows, macOS and Linux: rustfmt, clippy, cargo test, ESLint, Prettier, type check, Vitest and a UI build.
- `ARCHITECTURE.md` explaining the design decisions.
