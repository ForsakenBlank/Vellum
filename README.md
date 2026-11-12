# Vellum

> [!WARNING]
> **This app is vibe coded.** It is being built mostly by chatting with an AI coding assistant, not written
> line by line by an experienced developer. Expect rough edges and bugs.
> **Keep backups of your files:** Vellum is designed never to overwrite your originals, but keep your own copies
> of anything important anyway.

An all-in-one desktop media editor. Drop any file in and get the right tools straight away.

Vellum aims to replace a stack of separate apps and websites (image editors, video editors, audio editors, converters, GIF makers, screen capture tools) with one fast, local app. Every common job, like shrinking a clip for Discord, turning a video into a GIF or cutting the background out of a photo, should take one drag and one click.

> **Status:** early planning. The repo currently holds the folder skeleton and the project plan. No code yet.

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

## Roadmap

The full plan, broken into phases, lives in [PLAN.md](PLAN.md).
