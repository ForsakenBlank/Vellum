# Changelog

All notable changes to Vellum are listed here. Dates are in YYYY-MM-DD.

## Unreleased

### Phase 3: quick tools (2026-11-20)

Added

- Export to X MB. Drop a video, photo, GIF or song and get a copy under the size, 10 MB unless you pick another. Chips for Discord, Discord Nitro Basic, Discord Nitro, email, WhatsApp, Telegram and X, editable in Settings. MB or MiB, a safety margin, and a choice of what to keep when something has to give. Before anything is encoded it says what will happen, like "1280×720 at 30 fps, 1,850 kbps picture, 128 kbps sound", and after it shows the final size, how much smaller it got and what changed. Files that already fit are left alone.
- Video to GIF. Pick the moment on a filmstrip with handles that land on real frames (arrow keys move one frame), drag a crop on the picture, add a classic meme caption or a caption bar, and set the width, frame rate, speed, colours, dithering, lossy compression and how many times it plays. Reverse and boomerang. It shows roughly how big the result will be before making it, and can step the settings down until it fits a size. Also makes animated WebP, APNG, AVIF and silent MP4 loops, and says when an MP4 would be much smaller.
- GIF tools: make a GIF smaller, edit it frame by frame (delete, duplicate, move and retime frames, all undoable), split it into pictures, or turn it into an MP4.
- Convert: any format to any format that makes sense. Changing only the container copies the streams, so it is instant and lossless, and the tool says so before you start. Pull the sound out of a video, save frames as pictures, turn photos into a slideshow, make Windows icons with every size, draw SVGs at any width, and convert camera raw, HEIC and Photoshop files. Resize, turn, flip, trim, mute and change the speed on the way.
- Every quick tool takes many files at once, shows progress and the result next to each file, and takes more files dropped onto its list. Results open, show in their folder, compare before and after, or copy as a file you can paste straight into Discord or any chat.
- Settings, Export and quick tools: the starting size, the margin, where results go (next to the original by default) and the size presets.
- Quick tool options undo and redo, are autosaved and come back after a crash, like any other change.
- Convert can join a list of videos or sounds into one file. Files recorded the same way join instantly without re-encoding; others are fitted to the shape of the first, with silence added where a clip has no sound.
- Convert can cut videos and sounds into parts of any length. It is instant by default, with an "Exact cuts" option that re-encodes so every part is exactly the same length.
- Convert can rename files from a pattern like `holiday {n}`, with numbering, the date, find and replace, and lowercase or uppercase. A preview shows every new name first, names that are already taken are skipped, and undo puts the old names back.

Fixed

- Two Vellum sessions started in the same millisecond in one process could claim the same recovery folder, and the second then waited forever for the first one's lock. It only showed up in the test suite under heavy load, but it could have frozen start-up. Each session now claims a folder of its own.

Notes

- Nothing a quick tool makes ever replaces an existing file.
- GIFs are made with FFmpeg's palette tools rather than gifski, which would force Vellum to be open source. Captions use the bundled Inter font, so they look the same on every computer.
- Explorer and Finder right click menus are left for phase 16.

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
