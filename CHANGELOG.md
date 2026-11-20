# Changelog

All notable changes to Vellum are listed here. Dates are in YYYY-MM-DD.

## Unreleased

### Phase 2: media core (2026-11-20)

Added

- Vellum now reads real files. Recent files on the home screen show real thumbnails for photos, videos, GIFs, drawings and camera raws, made in the background and kept in a cache.
- The Info panel shows everything about the open file: format, size, dimensions, length, codecs, frame rate (and whether it varies), bit depth, colour, HDR, rotation, audio tracks, subtitles and every metadata tag. Videos get a thumbnail and a filmstrip, audio gets a waveform.
- "Copy info" copies those details as text, for a bug report or a chat.
- "Strip metadata" saves a copy without camera details, GPS location, comments and tags. JPEG, PNG and WebP are copied without re-encoding, colour profiles and photo orientation are kept, and video and audio are stream copied. The original is never changed. The Info panel warns when a file records where it was made.
- Image support for PNG, JPEG, WebP, AVIF, HEIC, JPEG XL, TIFF, GIF, APNG and animated WebP (every frame), BMP, ICO, TGA, QOI, HDR, EXR, camera raw, SVG at any size and Photoshop files (the flattened image). Photos are turned upright from their EXIF orientation.
- Big photos and scans make thumbnails without loading the whole image into memory.
- Frame accurate seeking, including variable frame rate phone videos.
- Hardware encoders (NVIDIA, Intel, AMD, Apple, VA-API) are tested at start-up and only working ones are used, with an automatic fallback to the processor. The encoding engine is ready for the quick tools in phase 3.
- Settings, Media and cache: hardware encoding on or off, which encoders were found, the FFmpeg in use, the preview cache size limit (2 GB by default) and a Clean button.
- If FFmpeg is missing, Vellum still starts, images still open, and a notification says what to install.

Changed

- Background work such as making previews no longer appears in the jobs panel or as notifications.

Notes

- Vellum uses the FFmpeg installed on the computer for now (or the one in `VELLUM_FFMPEG_DIR`). The installer will bundle one in phase 18.
- HEIC photos stored in tiles, which includes most iPhone photos, need FFmpeg 7.1 or newer.
- No media files are committed: the tests make about 30 small files with FFmpeg on demand. Run `scripts/make-test-media.sh` to get them in a folder.

### Phase 1: app shell and UX foundations (2026-11-19)

Added

- A "darkroom" design system: graphite surfaces, warm text and an amber accent, with dark, light and high contrast themes, any accent colour, interface size, compact mode and reduced motion.
- Home screen with a drop zone, big quick tool cards (Export to 10 MB, Video to GIF, Remove background, Convert, Screen record), pinned files and recent files with thumbnails.
- Drop files anywhere to open them in the right workspace, or drop them on a quick tool card to open that tool. An overlay says what will happen before you let go.
- Document tabs that reorder by dragging and tear off into their own windows, a workspace rail, and dockable panels (Info, History, Jobs) with built in and saved layouts.
- Command palette on Ctrl+K that searches every command, setting, recent file and preset.
- Settings window with search, and a shortcut editor with conflict warnings and Photoshop, Premiere Pro, DaVinci Resolve and Audacity presets.
- Autosave every few seconds (configurable) and a prompt to restore your work after a crash or after quitting with unsaved changes.
- Notification centre and toasts for finished and failed jobs, with Open and Retry.
- The jobs panel can reorder waiting jobs, open or show a result, and open a result as soon as its job finishes.
- Undo and redo for every open document from the Edit menu, the toolbar and shortcuts, with a History panel that shows branches.
- Empty states, loading skeletons and tooltips with shortcuts throughout.
- Playwright tests for drop routing, the command palette and crash recovery, run in CI.

Changed

- The default accent colour is now amber. Settings files that already chose an accent keep it.

Notes

- Workspaces and quick tools are placeholders until their own phases. Placeholder documents have a name and notes so undo, autosave and recovery can be tried, and Save only marks them as saved.

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
