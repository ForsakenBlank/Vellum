# Changelog

All notable changes to Vellum are listed here. Dates are in YYYY-MM-DD.

## Unreleased

### Phase 2: media core (2026-11-20)

Added

- Vellum reads media through the FFmpeg and FFprobe programs. It finds them in a folder you choose in Settings > Media and cache, next to Vellum itself, in the usual install folders or on `PATH`, and says plainly when they are missing. Images still open without FFmpeg.
- Probing any file gives one description: format, size, length, bit rate, codecs, the size a video is shown at (rotation and non square pixels included), frame rate and whether it varies, colour, HDR (HDR10, HDR10+, HLG, Dolby Vision, with brightness details), audio channels and sample rate, and every metadata tag, with likely private ones (location, serial numbers, owner, camera) flagged.
- One image decoder for PNG, APNG, JPEG, GIF and animated WebP (every frame), TIFF, BMP, ICO, QOI, TGA, PNM, HDR, EXR, SVG at any size, Photoshop documents (flattened), camera raw, AVIF, JPEG XL and HEIC. Photos turn upright from their EXIF orientation, files are recognised by their contents rather than their name, and images too big for memory are refused with a clear message instead of freezing the computer.
- Hardware video encoders and decoders (NVIDIA NVENC, Intel Quick Sync, AMD AMF, Apple VideoToolbox, VA-API) are tried for real once per FFmpeg version, and encodes fall back to the processor if the graphics card fails part way.
- Frame accurate seeking that also works for variable frame rate phone and screen recordings.
- A media cache for thumbnails, filmstrips, waveforms and file details, with a size limit (2 GB by default) and a Clean cache button. Changed files are never served stale.
- Thumbnails for every kind of file on the home screen, including videos (skipping black openings) and music with cover art.
- The Info panel shows everything Vellum knows about the open file, with Copy info and Strip metadata. Strip metadata saves a copy without location, camera and other hidden details: photos keep every pixel, their orientation and their colour profile, and videos lose GPS tracks too. The original is never touched.
- Settings > Media and cache: FFmpeg folder, hardware acceleration, cache size, the largest image to open, the hardware encoders found and how much cache is used.
- `tests/media/generate.sh` makes a folder of test files in every format, and a benchmark example times 4K HEVC thumbnails.

Changed

- Small background work like thumbnails runs as background jobs: it waits its turn like any job but stays out of the jobs panel and notifications.
- CI installs FFmpeg on Windows, macOS and Linux and runs the media tests there.

Notes

- A thumbnail for a 10 second 4K HEVC 10-bit video takes about 0.57 s the first time on a 4 thread machine without hardware decoding (0.11 s to read the file, 0.45 s to decode one frame) and 0.02 ms after that. A 10 frame filmstrip of it takes 1.2 s.
- HEIC photos need libheif's tools or FFmpeg 7.1 or newer for now. Pausing a running FFmpeg pass works on Linux and macOS; on Windows the pause happens when the pass ends.

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
