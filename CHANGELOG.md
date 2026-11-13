# Changelog

Notable changes to Vellum, newest first.

## Unreleased

### Phase 1: app shell and UX foundations

Vellum now looks and behaves like the app it will become, even though the editors are still placeholders.

**Added**

- A home screen with quick tool cards (Export to 10 MB, Video to GIF, Remove background, Convert, Screen record), recent files with thumbnails for images, and pinned files.
- Drop files anywhere to open them in the workspace for their type. Dropping on a quick tool card opens that tool, and dropping on a workspace icon opens the file there. While files hover, an overlay says what will happen, including which files will be skipped and why.
- Document tabs: reorder by dragging, close with Ctrl+W or a middle click, reopen the last closed tab with Ctrl+Shift+T, and drag a tab out of the bar to open it in its own window. Closing that window sends its files back to the main one.
- Dockable panels on the left, right and bottom of every workspace, in tabbed stacks that can be resized, dragged to another place, closed and opened again. Layouts are saved per workspace, can be saved under a name, and reset to the default.
- A command palette on Ctrl+K that finds commands, settings, recent files and presets, with fuzzy search and the shortcut for each command.
- A settings window with search, covering appearance, general, performance and shortcuts.
- A shortcut editor: add, remove and reset keys, see and resolve conflicts, and start from the Vellum, Photoshop, Premiere Pro, DaVinci Resolve or Audacity presets.
- Dark, light and high contrast themes, any accent colour, interface size, compact mode and reduced motion.
- Autosave of open files (every 30 seconds by default, or off), and a "Restore your last session?" prompt after Vellum closes unexpectedly.
- A notification centre for finished and failed jobs, with short toasts that open the result when clicked.
- A better jobs panel: move waiting jobs up and down, open or show a finished job's file, and ask for a job's result to open by itself when it is done.
- Empty states for every workspace and panel, loading skeletons at startup, and tooltips with shortcuts on every button.
- A second launch of Vellum focuses the running one and opens any files it was given.
- Playwright tests for drop routing, the command palette, crash recovery, tabs and docking.

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
