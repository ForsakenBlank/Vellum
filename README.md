# Vellum

> [!WARNING]
> **This app is vibe coded.** It is being built mostly by chatting with an AI coding assistant, not written
> line by line by an experienced developer. Expect rough edges and bugs.
> **Keep backups of your files:** Vellum is designed never to overwrite your originals, but keep your own copies
> of anything important anyway.

An all-in-one desktop media editor. Drop any file in and get the right tools straight away.

Vellum aims to replace a stack of separate apps and websites (image editors, video editors, audio editors, converters, GIF makers, screen capture tools) with one fast, local app. Every common job, like shrinking a clip for Discord, turning a video into a GIF or cutting the background out of a photo, should take one drag and one click.

> **Status:** phase 2 (media core) is done: Vellum reads, probes and previews real files (photos, camera raw, SVG, PSD, video, audio), shows everything about them in the Info panel, strips metadata, and has an encoding engine with hardware encoder detection. Phase 1 built the app shell (home screen, tabs, docks, command palette, settings, autosave and crash recovery). Workspaces and quick tools are still placeholders. The window has been run on Linux only; Windows and macOS still need a check by hand.

## Planned features

- **Quick Tools:** export to a target file size, video to GIF, format converter
- **Image editor:** layers, selections, brushes, filters, PSD round trips, easy mode
- **Video editor:** multi-track timeline, editing tools, colour grading
- **Audio editor:** waveform editing, effects, mixer
- **Motion graphics and compositing**
- **Vector, design and PDF tools**
- **Capture:** screen and audio recording
- **Local AI tools:** background removal, click to select, object removal, upscaling, subtitles, stem splitting
- **Automation:** batch jobs, macros, watch folders, CLI and plugins

## Tech stack

| Layer | Choice |
|---|---|
| App shell | Tauri 2 |
| Core engine | Rust (cargo workspace) |
| UI | React, TypeScript, Vite, Zustand, Radix UI, Tailwind |
| GPU | wgpu with WGSL shaders |
| Video and audio I/O | FFmpeg |
| AI | ONNX Runtime (local models, no cloud) |
| Database | SQLite |

## Repo layout

```
apps/desktop/            tauri app (rust side: commands, windows, menus)
ui/                      react + typescript front end
  src/shell/             tabs, panels, command palette, settings
  src/workspaces/        image, video, audio, color, motion, vector, quick-tools
  src/components/        shared design system
crates/
  vellum-core/           project model, undo/redo, commands, events, settings
  vellum-media/          ffmpeg decode/encode, probing, thumbnails, proxies
  vellum-gpu/            wgpu renderer, shaders, colour management, compositor
  vellum-image/          raster document, layers, selections, brushes, filters
  vellum-timeline/       nle data model, playback scheduler, frame cache
  vellum-audio/          audio engine, mixer, effects, waveform cache
  vellum-ai/             onnx runtime, model manager, ai tools
  vellum-export/         export presets, size targeting, render queue
  vellum-capture/        screen and audio capture
  vellum-vector/         vector document and path tools
  vellum-automation/     batch, macros, watch folders, cli, plugin host
models/                  downloaded ai models (gitignored)
tests/golden/            reference images and videos for regression tests
```

## Development

You need Rust (stable), Node 20 or newer with pnpm (`corepack enable`), the [Tauri system packages](https://tauri.app/start/prerequisites/) for your OS, and [FFmpeg](https://ffmpeg.org/download.html) on your `PATH` (or set `VELLUM_FFMPEG_DIR` to the folder with `ffmpeg` and `ffprobe`). Without FFmpeg the app still runs, but only images open and the media tests are skipped.

```bash
pnpm install
```

```bash
pnpm tauri dev
```

Checks that CI runs:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

```bash
pnpm lint && pnpm format:check && pnpm typecheck && pnpm test
```

End to end tests run the UI in Chromium against a fake backend (install the browser once with `pnpm --filter @vellum/ui exec playwright install chromium`):

```bash
pnpm e2e
```

`cargo test` also regenerates the TypeScript types in `ui/src/bindings`; commit them when they change. The media tests make their own small test files with FFmpeg; `scripts/make-test-media.sh` puts the same files in a folder so you can try them by hand.

## Roadmap

The full plan, broken into phases, lives in [PLAN.md](PLAN.md). How the code is put together, and why, is in [ARCHITECTURE.md](ARCHITECTURE.md). Changes are listed in [CHANGELOG.md](CHANGELOG.md).
