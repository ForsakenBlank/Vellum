# Vellum: All-in-One Media Editor (Project Plan)

One desktop app that replaces Photoshop/Photopea, Premiere Pro, DaVinci Resolve, Audacity, HandBrake, ShareX, ezgif, Illustrator-lite and a pile of small converter websites. Drop any file in and get the right tools straight away. Every common job (shrink this for Discord, turn this clip into a GIF, cut the background out of this photo) should take **one drag and one click**.

---

## Table of contents

1. [How to use this file](#1-how-to-use-this-file)
2. [Model and effort cheat sheet](#2-model-and-effort-cheat-sheet)
3. [Tech stack](#3-tech-stack)
4. [Architecture overview](#4-architecture-overview)
5. [Conventions (every phase follows these)](#5-conventions-every-phase-follows-these)
6. [UX principles](#6-ux-principles)
7. [Global QoL features](#7-global-qol-features)
8. [Feature catalogue](#8-feature-catalogue)
9. [Phases with prompts](#9-phases-with-prompts)
10. [Reusable prompts](#10-reusable-prompts)
11. [Risks and things to watch](#11-risks-and-things-to-watch)

---

## 1. How to use this file

1. Make an empty folder for the project and save this file in its root as `PLAN.md`.
2. Work through the phases **in order**. Each phase (from phase 6 on, each step) has a goal, a "done when" checklist, a recommended model and effort level, and a prompt.
3. Start a **fresh coding session** for each phase or step (Claude Code works best for this). Paste the phase prompt in. Each prompt tells the model to read `PLAN.md` first, so it always has the full picture.
4. When a phase finishes (its last step), run the **Phase review prompt** (section 10) before moving on. Fixing problems early is far cheaper than fixing them five phases later.
5. Tick off the "done when" boxes in this file as you go so every future session knows where things stand.

**Session size:** phases 0 to 5 were each one big session, and phase 6 ran out of usage twice. From phase 6 on, every phase is split into lettered steps (6a, 6b, ...) that each fit in one session. Run one step per session, in order, and read "How a step runs" at the top of the phase 6 part of section 9.

---

## 2. Model and effort cheat sheet

Fable 5.2 is no longer available, so every remaining step uses Opus 5.5 or Sonnet 5.5. Steps marked "(was)" record what built them.

| Step | What | Status | Model | Effort | Weight | Done after it |
|---|---|---|---|---|---|---|
| 0 | Repo bootstrap | Done (Windows and macOS not checked by hand) | Fable 5.2 (was) | high | 3% | 3% |
| 1 | App shell | Done | Opus 5.5 | high | 4% | 7% |
| 2 | Media core | Done | Opus 5.5 | xhigh | 5% | 12% |
| 3 | Quick Tools | Done | Opus 5.5 | high | 5% | 17% |
| 4 | GPU render engine and colour | Done | Fable 5.2 (was) | max | 8% | 25% |
| 5 | Image editor core | Done | Fable 5.2 (was) | xhigh | 8% | 33% |
| 6a | Image tools: engine and document side | Done | Opus 5.5 | high | 2.4% | 35.4% |
| 6b | Adjustments panel and Camera Raw | Not started | Sonnet 5.5 | high | 0.6% | 36% |
| 6c | Filter gallery, smart objects, smart filters | Not started | Sonnet 5.5 | high | 0.6% | 36.6% |
| 6d | Retouching tools, liquify, puppet warp | Not started | Sonnet 5.5 | high | 0.6% | 37.2% |
| 6e | Text tool and fonts | Not started | Opus 5.5 | high | 0.6% | 37.8% |
| 6f | Shapes, pen tools and paths | Not started | Opus 5.5 | high | 0.6% | 38.4% |
| 6g | Layer styles, PSD saving, export dialogs | Not started | Sonnet 5.5 | high | 0.6% | 39% |
| 7a | AI runtime and model manager | Not started | Opus 5.5 | high | 1% | 40% |
| 7b | Background remover | Not started | Opus 5.5 | high | 1.2% | 41.2% |
| 7c | Click to select (SAM 2) | Not started | Opus 5.5 | high | 1% | 42.2% |
| 7d | Batch background removal, AI selection extras | Not started | Sonnet 5.5 | high | 0.8% | 43% |
| 7e | Object remover, upscale, face restore | Not started | Sonnet 5.5 | high | 1% | 44% |
| 7f | Colourise, depth blur, sky replace, OCR | Not started | Sonnet 5.5 | high | 1% | 45% |
| 8a | Timeline data model | Not started | Opus 5.5 | xhigh | 1.8% | 46.8% |
| 8b | Playback engine | Not started | Opus 5.5 | xhigh | 2% | 48.8% |
| 8c | Scrubbing, frame cache, proxies | Not started | Opus 5.5 | high | 1.5% | 50.3% |
| 8d | Media pool and source monitor | Not started | Sonnet 5.5 | high | 1% | 51.3% |
| 8e | Timeline UI | Not started | Opus 5.5 | high | 1.7% | 53% |
| 8f | Sequence export and stress test | Not started | Sonnet 5.5 | high | 1% | 54% |
| 9a | Edit tools | Not started | Opus 5.5 | high | 1.2% | 55.2% |
| 9b | Transitions and effects | Not started | Sonnet 5.5 | high | 1% | 56.2% |
| 9c | Keyframe editor and speed | Not started | Opus 5.5 | high | 1% | 57.2% |
| 9d | Titles and templates | Not started | Sonnet 5.5 | high | 0.8% | 58% |
| 9e | Auto subtitles, text based editing | Not started | Opus 5.5 | high | 1.2% | 59.2% |
| 9f | Smart tools, part one | Not started | Sonnet 5.5 | high | 0.8% | 60% |
| 9g | Smart tools, part two | Not started | Opus 5.5 | high | 1% | 61% |
| 10a | Grading nodes and primaries | Not started | Opus 5.5 | xhigh | 1.5% | 62.5% |
| 10b | Secondaries | Not started | Opus 5.5 | high | 1.2% | 63.7% |
| 10c | Scopes | Not started | Opus 5.5 | high | 1% | 64.7% |
| 10d | LUTs, colour management, HDR, shot match | Not started | Opus 5.5 | high | 1.3% | 66% |
| 11a | Audio engine | Not started | Opus 5.5 | high | 1% | 67% |
| 11b | Audio workspace | Not started | Sonnet 5.5 | high | 1% | 68% |
| 11c | Audio effects | Not started | Opus 5.5 | high | 1% | 69% |
| 11d | Loudness and timeline mixer | Not started | Opus 5.5 | high | 1% | 70% |
| 11e | AI audio and extras | Not started | Sonnet 5.5 | high | 1% | 71% |
| 12a | Compositions | Not started | Opus 5.5 | high | 1.2% | 72.2% |
| 12b | Shape and text animation | Not started | Opus 5.5 | high | 1% | 73.2% |
| 12c | Graph editor and expressions | Not started | Opus 5.5 | high | 1% | 74.2% |
| 12d | Masks, mattes, AI roto | Not started | Opus 5.5 | high | 1% | 75.2% |
| 12e | Node view, particles, camera | Not started | Opus 5.5 | high | 1% | 76.2% |
| 12f | Lottie and templates | Not started | Sonnet 5.5 | high | 0.8% | 77% |
| 13a | Export dialog and presets | Not started | Sonnet 5.5 | high | 1.5% | 78.5% |
| 13b | Render queue and after export | Not started | Sonnet 5.5 | high | 1.5% | 80% |
| 14a | Screenshots, hotkeys, tray | Not started | Sonnet 5.5 | high | 1.2% | 81.2% |
| 14b | Screen recorder | Not started | Opus 5.5 | high | 1.6% | 82.8% |
| 14c | Replay buffer and screen tools | Not started | Sonnet 5.5 | high | 1.2% | 84% |
| 15a | Vector workspace | Not started | Sonnet 5.5 | high | 1.2% | 85.2% |
| 15b | Trace, templates, mockups | Not started | Sonnet 5.5 | high | 1% | 86.2% |
| 15c | PDF tools | Not started | Sonnet 5.5 | high | 1% | 87.2% |
| 15d | Misc tools | Not started | Sonnet 5.5 | medium | 0.8% | 88% |
| 16a | Macros, recipes, watch folders | Not started | Sonnet 5.5 | high | 1.3% | 89.3% |
| 16b | CLI and context menus | Not started | Sonnet 5.5 | high | 1% | 90.3% |
| 16c | Plugin API | Not started | Opus 5.5 | high | 1.5% | 91.8% |
| 16d | Asset library | Not started | Sonnet 5.5 | high | 1.2% | 93% |
| 17a | Onboarding and help | Not started | Sonnet 5.5 | high | 1% | 94% |
| 17b | Accessibility | Not started | Sonnet 5.5 | high | 1% | 95% |
| 17c | Performance | Not started | Opus 5.5 | high | 1.2% | 96.2% |
| 17d | Copy and consistency review | Not started | Sonnet 5.5 | medium | 0.8% | 97% |
| 18a | Installers and release workflow | Not started | Sonnet 5.5 | medium | 1.5% | 98.5% |
| 18b | Updates, crash reports, licences, README | Not started | Sonnet 5.5 | medium | 1.5% | 100% |

| Other work | Model | Effort |
|---|---|---|
| Phase review (after the last step of a phase) | Opus 5.5 | high |
| Bug fixing on a single feature | Sonnet 5.5 | medium |
| Docs, changelogs, README, small copy fixes | Haiku 5.5 | low |
| Weird crash / memory leak / race condition | Opus 5.5 | high (xhigh if high fails twice) |

**Rule of thumb:**
- **Opus 5.5** for engines, data models, threading, GPU and anything where a wrong design is expensive later.
- **Sonnet 5.5** for UI on top of an engine that already exists, wiring, polish and most bug fixes.
- **Haiku 5.5** for docs and tiny edits.
- **high** is the default effort. **xhigh** only for the three steps marked above (8a, 8b, 10a), or when a step has failed twice at high. Never use **max**: it burns usage far faster than it helps.
- If a step runs out of usage, start a new session with the same prompt. The step rules make it commit as it goes and leave unticked boxes for what is left.

---

## 3. Tech stack

| Layer | Choice | Why |
|---|---|---|
| App shell | **Tauri 2** | Small installer, native speed, Rust backend, web UI. Windows, macOS and Linux. |
| Core engine | **Rust** (cargo workspace) | Fast, safe threading, great for media pipelines. |
| UI | **React + TypeScript + Vite**, Zustand for state, Radix UI primitives, Tailwind | Fast to build polished UI. |
| GPU | **wgpu** (Rust) with WGSL shaders, rendered into a native surface or shared texture | One GPU path for image, video and compositing. |
| Video/audio I/O | **FFmpeg** (via `ffmpeg-next` bindings, FFmpeg shipped as LGPL build) | Reads and writes basically everything. Hardware encode via NVENC, QuickSync, AMF, VideoToolbox. |
| Images | `image`, `libvips` for huge files, `libheif`, `libraw` for camera RAW, `resvg` for SVG | Broad format support. |
| GIF | **gifski** (high quality GIF encoder) + gifsicle-style lossy pass | Best looking GIFs at small sizes. |
| PSD | `psd` crate for reading, custom writer for export | Photopea-style PSD round trips. |
| Audio DSP | `cpal` for playback/recording, `rubato` for resampling, custom DSP, optional LV2/VST3/CLAP hosting later | Real-time audio. |
| AI | **ONNX Runtime** (`ort` crate) with DirectML / CUDA / CoreML backends | Runs local models, no cloud needed. |
| AI models | SAM 2 (click to select), BiRefNet or U2-Net (background removal), LaMa (inpaint / object removal), Real-ESRGAN (upscale), Whisper (subtitles), Demucs (stem split), RIFE (frame interpolation) | All have permissive licences; re-check each licence before shipping. |
| Database | SQLite (`rusqlite`) for library, cache index, history of exports | Simple and reliable. |
| Tests | `cargo test`, Vitest, Playwright for UI, golden-image tests for renders | Catch visual regressions. |

---

## 4. Architecture overview

```
vellum/
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

**Key ideas:**
- **One command system.** Every action (button, menu, shortcut, macro, CLI, plugin) goes through the same `Command` type. That gives free undo/redo, macro recording, command palette search and scripting.
- **One job queue.** Anything slow (export, AI, proxy generation, conversions) is a `Job` with progress, ETA, cancel, pause, and a notification when done. The UI never freezes.
- **One render graph.** Images, video frames and motion graphics all render through the same GPU node graph in `vellum-gpu`, so an effect written once works everywhere.
- **Non-destructive by default.** Adjustments, effects and AI results are layers or nodes that can be edited or removed later. Originals are never overwritten unless the user says so.
- **Projects are folders** (`.vellum` bundle) with a JSON manifest plus media cache, so they are diffable, recoverable and portable.

---

## 5. Conventions (every phase follows these)

Every prompt below tells the model to obey this section.

**Code style**
- Comments are lowercase, short, and have a space after the comment symbol. Example: `let x = 5; // number of retries` or `print("Hello World") # this prints hello world`.
- No em dashes anywhere: not in code, comments, UI text, commit messages or docs.
- Code should read like a careful human wrote it: sensible names, no giant banner comments, no comments that just repeat the code, no filler like "this function handles the logic for".
- Rust: `cargo fmt` and `cargo clippy -- -D warnings` must pass. TypeScript: strict mode, ESLint and Prettier must pass.
- No `unwrap()` in non-test Rust code unless the reason is obvious and commented.

**Engineering rules**
- All slow work runs as a `Job` on the job queue with progress and cancel.
- All user actions go through the `Command` system so they are undoable and recordable.
- Never block the UI thread. Never lose user work: autosave and crash recovery must keep working after every phase.
- Write tests for the core logic of every feature. Rendering features get golden-image tests.
- When a phase is done, update the checkboxes in `PLAN.md` and add a short entry to `CHANGELOG.md`.
- If something in this plan is wrong or a better approach exists, say so and explain before changing direction.

**UI copy rules**
- Plain words. "Remove background", not "Execute segmentation".
- Every button has a tooltip with its shortcut.
- Errors say what happened and what to do next.

---

## 6. UX principles

1. **Drop anything anywhere.** Dragging a file onto the window, the taskbar icon or the tray icon opens it in the right workspace. Dragging onto a specific tool (for example the GIF maker card) opens it in that tool.
2. **Zero-config fast paths, full control one click away.** Every quick tool has a big obvious default button plus an "Advanced" drawer.
3. **Live previews everywhere.** Sliders preview in real time. Export to size shows the estimated result before encoding.
4. **Never surprise the user.** No silent overwrites, clear unsaved markers, undo for everything (including deletes in the asset library).
5. **Everything is searchable.** Command palette (`Ctrl+K`) finds every tool, setting, effect, preset, recent file and help article.
6. **Consistent everywhere.** Same shortcuts for the same ideas in every workspace (space = play/pan, `Ctrl+Z` undo, `[`/`]` brush size, `J/K/L` shuttle).
7. **Fast feels good.** App opens in under 2 seconds, panels animate in under 150 ms, scrubbing never stutters (proxies kick in automatically).
8. **Respect muscle memory.** Optional shortcut presets for Photoshop, Premiere, Resolve and Audacity users.

---

## 7. Global QoL features

**Files and projects**
- Home screen with recent files (with thumbnails), pinned projects, and big "quick tool" cards.
- Open from clipboard (paste an image or URL straight into the app).
- Autosave every N seconds plus crash recovery with a "restore your session" prompt.
- Version history per project with visual thumbnails, restore any version.
- "Reveal in folder", "Copy path", "Copy as file" (paste into Discord directly), "Copy as image".
- Relink missing media automatically by searching nearby folders.
- Smart file naming templates for exports (`{name}_{preset}_{date}`), never overwrite by default.
- Open the same file in two workspaces (for example a video frame in the image editor) with live round-trip.

**Interface**
- Dockable, resizable, tear-off panels. Save and switch workspace layouts.
- Light, dark and high-contrast themes, accent colour, UI scale, compact mode.
- Command palette with fuzzy search and recent actions.
- Customisable shortcuts with conflict detection and presets (Photoshop, Premiere, Resolve, Audacity).
- Right-click radial menu (optional) for the most used tools.
- History panel with named snapshots and branching history (undo, change something, still able to get back).
- Tool tips with short looping video demos for complex tools.
- Context-aware "What can I do with this?" button suggesting relevant tools.
- Notifications centre for finished jobs; click to open the result.
- Multi-monitor support: full-screen preview on a second display.
- Tabs for multiple open documents, drag tabs between windows.

**Productivity**
- Global job queue panel: pause, reorder, cancel, retry, "open when done", "shut down PC when done".
- Presets everywhere: save any tool's settings as a preset, share presets as small files.
- Macro recorder: record actions, replay on other files.
- Batch anything: drop 200 files on any tool to process them all.
- Watch folders: anything dropped into a folder gets auto-processed.
- Tray icon with quick actions (screenshot, record, compress clipboard file).
- Global hotkeys for capture tools even when the app is minimised.
- Context menu integration in Windows Explorer / macOS Finder: "Compress to 10 MB", "Make GIF", "Remove background", "Convert to...".
- Built-in file info panel: resolution, codec, bitrate, colour space, metadata, with a "strip metadata" button.

**Performance and safety**
- GPU acceleration toggle with automatic fallback to CPU.
- Hardware encoder detection with a speed vs quality slider.
- Disk cache manager with size limit and one-click clean.
- Low-memory mode for big files and older machines.
- Fully offline: AI models are downloaded once, then everything runs locally. No account needed.

---

## 8. Feature catalogue

### 8.1 Quick Tools (the "just do it" hub)

**Export to [X] MB** (must have)
- One input box: target size, default **10 MB** (Discord free tier). Works on video, GIF, image and audio.
- Preset chips: Discord, Discord Nitro Basic, Discord Nitro, email attachment, WhatsApp, Telegram, X/Twitter, custom. Values are editable in settings because platforms change limits; the phase prompt checks the current numbers.
- MB vs MiB toggle, plus a safety margin (default 3%) so files never land just over the limit.
- Strategy picker: "Best quality" (auto picks resolution and fps), "Keep resolution", "Keep fps", "Keep audio quality".
- **Video:** two-pass encode with bitrate calculated from duration and target size, minus audio. If the bitrate would be too low for the resolution, it auto-downscales (1080p to 720p to 480p) and/or drops fps, then shows what it changed. If the result overshoots, it re-encodes at a corrected bitrate. Options: trim before compressing, mute audio to save space, mono audio, codec choice (H.264 for compatibility, H.265, AV1, VP9).
- **Image:** binary search over quality for JPEG/WebP/AVIF, then step down resolution if needed. PNG gets palette quantisation (pngquant-style) and optimisation.
- **GIF:** reduce colours, lossy compression, frame skipping, resize, shorten. Binary search to hit the size. Suggests "this would be much smaller as MP4/WebP" when relevant.
- **Audio:** bitrate calculated from duration, Opus or MP3.
- Live estimate before encoding, before/after preview slider, final size shown with a green tick.
- "Copy to clipboard as file" button after export so you can paste straight into Discord.
- Batch mode: make every dropped file fit the limit.

**Video to GIF** (must have)
- Drop a video, pick start/end on a mini timeline with frame-accurate handles.
- Crop, resize, fps (with a live size estimate), speed, reverse, boomerang (ping-pong), loop count.
- Text/caption overlay with meme-style presets (impact top/bottom text, modern caption bar).
- Quality engine: gifski with dithering options, colour count, lossy level.
- Also export as animated WebP, APNG, AVIF or MP4 loop (often far smaller).
- "Fit to X MB" checkbox that hooks into the Export to size engine.
- GIF to video, GIF frame editor (delete, duplicate, reorder, change per-frame delay), GIF optimiser, GIF splitter.

**Universal converter**
- Any to any for image, video, audio, GIF and documents where possible, with format-aware options.
- Change container without re-encoding when possible (instant MKV to MP4).
- Extract audio from video, extract frames as images, images to video/slideshow.
- Resize, rotate, flip, crop, trim, merge, split, mute, speed change as single-click tools.
- HEIC to JPG, WebP to PNG, RAW to JPG, SVG to PNG at any size, ICO maker (multi-size icons).
- Batch rename with patterns and preview.

### 8.2 Image editor (Photoshop / Photopea parity, plus easy mode)

**Canvas and documents**
- Infinite zoom (smooth, GPU), rotate canvas view, flip view, rulers, guides, smart guides, grid, pixel grid at high zoom.
- Multiple artboards, canvas size, image size (with resampling options), crop with straighten, perspective crop, content-aware crop fill.
- 8, 16 and 32-bit per channel, RGB, Grayscale, CMYK preview (soft proof), ICC colour profiles.
- Open PSD, PSB (big files), TIFF, RAW, HEIC, AVIF, WebP, SVG, PDF pages, XCF, Sketch/Figma image layers where possible.

**Layers**
- Raster, text, shape, smart object (linked or embedded), adjustment, fill (solid, gradient, pattern), group layers.
- All blend modes from Photoshop (Normal through Luminosity, plus Divide, Subtract, Linear Light etc.).
- Opacity and fill opacity, layer masks, vector masks, clipping masks, blend-if sliders.
- Layer styles: drop shadow, inner shadow, outer/inner glow, bevel and emboss, stroke, colour/gradient/pattern overlay, satin. Copy/paste styles.
- Layer comps, linked layers, lock options, colour labels, search/filter layers, auto-name layers.

**Selections**
- Marquee (rect, ellipse, single row/column), lasso, polygon lasso, magnetic lasso.
- Magic wand, quick selection, object selection (AI, hover to highlight objects), select subject, select sky, colour range, focus area.
- **Smart Select (AI, SAM 2):** click things to include, Alt-click to exclude, live mask preview.
- Select and mask workspace: refine edge, smooth, feather, contrast, shift edge, decontaminate colours, hair refine brush.
- Quick mask mode, save/load selections, transform selection, grow/shrink/border.

**Background remover** (must have)
- One click: remove background automatically.
- Then **you choose what is deleted**: the result shows as a mask overlay, and you get:
  - **Keep brush** (paint back anything that was removed) and **Erase brush** (paint away anything that stayed).
  - **Click to keep / click to remove** objects (SAM 2): click a person to keep them, click a chair to remove it.
  - Edge slider (soft to sharp), hair/fur refine, colour decontamination (removes background colour fringes).
  - Toggle between transparent, solid colour, blur original background, or a replacement image as the new background.
  - Shadow options: keep original shadow, add a fake drop shadow, or remove shadows.
- Result is a layer mask, so it is fully editable later. Batch background removal for product photos.

**Painting and retouching**
- Brush engine: pressure (size, opacity, flow), tilt, smoothing/stabiliser, spacing, scatter, texture, dual brush, colour dynamics. Import ABR brushes.
- Brush, pencil, mixer brush, eraser, background eraser, magic eraser, gradient (linear, radial, angle, reflected, diamond, freeform), paint bucket.
- Clone stamp, healing brush, spot healing, patch tool, content-aware move, red-eye.
- **Object remover (AI, LaMa):** paint over something and it disappears.
- **Generative fill (optional, local or user-provided API):** select area, type what to put there.
- Dodge, burn, sponge, blur, sharpen, smudge.
- Liquify (forward warp, pucker, bloat, face-aware sliders), puppet warp, perspective warp, warp mesh.
- Symmetry painting, colour sampler, eyedropper with averaging, swatches, colour harmonies, recent colours.

**Adjustments (all non-destructive)**
- Brightness/contrast, levels, curves, exposure, vibrance, hue/saturation, colour balance, black and white, photo filter, channel mixer, colour lookup (LUTs), invert, posterize, threshold, gradient map, selective colour, shadows/highlights, HDR toning, match colour, replace colour, equalize.
- Camera RAW style panel: white balance, tone, texture, clarity, dehaze, HSL, colour grading wheels, detail, lens corrections, vignette, grain.
- Auto tone, auto contrast, auto colour, one-click "enhance".

**Filters**
- Blur gallery (gaussian, motion, radial, lens, tilt-shift, field, iris, path, spin, surface), sharpen and unsharp mask, smart sharpen, noise reduction, add noise, high pass, distort (wave, twirl, polar, ripple, displace), stylize (emboss, find edges, oil paint, glowing edges), render (clouds, lens flare, lighting effects), pixelate, halftone, film grain, chromatic aberration, glitch.
- Smart filters on smart objects (editable and maskable).
- Filter gallery with live thumbnails.

**Text and shapes**
- Point and paragraph text, text on path, warp text, variable fonts, OpenType features, font preview, Google Fonts browser, missing font replacement.
- Shape tools with live corners, booleans, pen tool, curvature pen, path selection.

**AI and smart extras**
- Upscale 2x/4x (Real-ESRGAN), face restore, denoise, colourise black-and-white photos, depth map blur (fake portrait mode), auto straighten, sky replacement, relight.
- Remove text from image, extract text (OCR) and copy it.

**Output**
- Export as (with size preview), Save for Web (side by side quality comparison), export layers to files, export artboards, quick export PNG, copy merged.
- Hook into Export to X MB.

### 8.3 Video editor (Premiere Pro / DaVinci Resolve parity)

**Project and media**
- Media pool with bins, smart bins (auto group by type, date, camera), metadata columns, ratings, keywords, search.
- Hover-scrub thumbnails, proxy generation (automatic for heavy codecs), relink, consolidate (collect files).
- Source monitor with in/out, markers, subclips, sync clips by audio waveform or timecode.
- Multicam editing with automatic sync.

**Timeline**
- Unlimited video/audio tracks, nested sequences (compound clips), adjustment layers, track targeting, sync lock, track lock, solo/mute, track height presets.
- Edit tools: selection, ripple, roll, slip, slide, razor, track select forward/back, rate stretch, hand, zoom.
- Insert, overwrite, replace, fit to fill, three-point and four-point editing, lift and extract.
- Magnetic timeline option (Final Cut style) and classic track mode.
- Snapping, ripple delete gaps, close all gaps, J/K/L shuttle, frame-step, markers with colours and notes, chapter markers.
- Auto-reframe (AI subject tracking) for vertical/square versions.
- Scene cut detection: split a single long file into clips.
- Silence detection: auto-cut silent parts (great for talking-head videos), with a preview before applying.
- Beat detection: markers on music beats, auto-cut to the beat.
- Clip speed: constant speed, speed ramps with curves, reverse, freeze frame, optical-flow slow motion (RIFE frame interpolation).

**Effects and transitions**
- Video transitions: dissolves, dips, wipes, slides, zooms, glitch, whip pan, light leaks, with duration handles on the timeline.
- Effects: transform, crop, opacity, blend modes, blur, sharpen, chroma key (with spill suppression), luma key, masks with tracking, stabiliser (warp-stabiliser style), lens distortion, vignette, noise reduction, film grain, glow, chromatic aberration, mirror, tile, picture in picture presets.
- Keyframe everything with an easing curve editor (bezier handles, presets like ease-in-out).
- Motion tracking: point tracker and planar tracker, attach text or effects to tracked objects, blur faces automatically (privacy blur).
- Effect presets and favourites, drag effect onto multiple clips at once, copy/paste attributes.

**Titles and captions**
- Title templates (lower thirds, end cards, subscribe buttons) that are editable.
- **Auto subtitles (AI, Whisper):** transcribe, edit text in a transcript view, style (TikTok-style word-by-word highlights, karaoke), translate, export SRT/VTT or burn in.
- Text-based editing: delete words in the transcript and the video cuts automatically.

**Colour (Resolve-style colour page)**
- Primary wheels (lift, gamma, gain, offset), log wheels, curves (custom, hue vs hue, hue vs sat, hue vs lum, lum vs sat, sat vs sat), HSL qualifier, power windows with tracking, colour warper.
- Node-based grading (serial, parallel, layer nodes), grade versions, copy grade, shot match, auto colour, colour chart match.
- LUT import/export (.cube), ACES and DaVinci Wide Gamut style colour-managed workflow, HDR grading support.
- Scopes: waveform, parade, vectorscope, histogram, CIE chromaticity, false colour, zebras.
- Gallery stills with wipe compare against reference frames.

**Audio in the timeline (Fairlight-style)**
- Clip gain, keyframed volume, crossfades, track mixer with pan, sends, buses, meters with LUFS.
- Auto ducking (music lowers when someone talks), voice isolation, noise removal, loudness normalisation to platform targets (-14 LUFS for YouTube etc.).
- Mic recording onto a track (voiceover), with punch-in.

**Delivery**
- Render queue with presets: YouTube, TikTok/Reels/Shorts (vertical), Discord (size targeted), Twitter/X, Instagram, ProRes master, DNxHR, lossless, image sequence, audio only.
- Export a range, export selected clips individually, export every marker as a still, export frame as image.
- Smart render (only re-encode changed parts), hardware encoding, background rendering.

### 8.4 Audio editor (Audacity / Audition parity)

- Waveform and spectral views, spectral editing (paint out a cough or a phone ring).
- Cut, copy, paste, trim, silence, fade in/out, crossfades, envelope tool, zero-crossing snapping.
- Multitrack mode with mixer, plus single-file destructive mode for quick edits.
- Effects: EQ (parametric and graphic), compressor, limiter, gate, de-esser, reverb, delay, chorus, flanger, phaser, distortion, pitch shift, time stretch (without pitch change), normalise, amplify, reverse, bass boost.
- AI: noise removal, voice isolation, **stem separation** (vocals, drums, bass, other via Demucs), transcription, filler-word removal ("um", "uh").
- Loudness meter and normalisation to LUFS presets, true peak limiting.
- Batch convert, batch normalise, metadata/tag editor, cover art.
- Record from mic or system audio, with input level meters and monitoring.
- Ringtone maker, audio trimmer, merger, BPM and key detection.
- Plugin hosting (VST3, CLAP, LV2) in a later phase.

### 8.5 Motion graphics and compositing (After Effects / Resolve Fusion lite)

- Layer-based composition mode and a node view for advanced users.
- Shape layers with animated paths, trim paths, repeaters, wiggle.
- Text animators (per-character position, scale, opacity, blur), presets like typewriter, bounce, fade up.
- Graph editor with speed and value curves.
- Masks, track mattes, rotoscoping helped by AI segmentation (SAM 2 propagated across frames).
- Particles (simple emitter), lights, camera with 2.5D depth.
- Lottie import and export, templates you can drag onto the video timeline.

### 8.6 Vector and design tools (Illustrator / Figma lite)

- Pen, pencil, shapes, booleans, gradients, strokes with profiles, align and distribute.
- Image trace (bitmap to vector), SVG import/export with clean output.
- Artboards for social media sizes, thumbnail templates (YouTube thumbnail, Discord banner, Twitch overlays, profile pictures).
- Simple mockup generator (put a design onto a phone, laptop, T-shirt).
- Colour palette extractor from images, gradient generator, icon/favicon pack export.

### 8.7 Capture tools (OBS / ShareX lite)

- Screenshot: region, window, full screen, scrolling capture, delayed capture, with instant annotate (arrows, boxes, blur, numbered steps, highlight, text).
- Screen recording: region/window/screen, system audio and mic, webcam overlay, cursor highlight and click effects, keystroke display.
- Instant replay buffer: always recording the last N seconds, hotkey saves a clip (like ShadowPlay), then one click to "Export to 10 MB".
- Recording straight to GIF.
- Colour picker anywhere on screen, on-screen ruler, OCR any part of the screen.

### 8.8 Document and misc tools

- PDF: merge, split, reorder, compress, convert to images, images to PDF, sign, annotate.
- Image to text (OCR), QR code generator and reader.
- Metadata viewer/stripper (privacy: remove GPS from photos before sharing).
- Contact sheet / collage maker, meme maker, sprite sheet maker, image comparison slider export.
- 3D model viewer (GLB/OBJ/FBX) with turntable render to video or GIF.

### 8.9 Asset library

- Global library of your media, fonts, LUTs, presets, brushes, templates, sound effects.
- Tags, collections, ratings, colour labels, smart search (search "dog" finds pictures of dogs via local AI tagging).
- Duplicate finder (visually similar images), storage stats.

### 8.10 Automation and extensibility

- Macro recorder and editor (every Command is recordable).
- Batch processor: chain steps (resize, then watermark, then compress to 10 MB) and save as a "recipe".
- Watch folders, CLI (`vellum compress clip.mp4 --size 10mb`), shell context menu entries.
- Plugin API (TypeScript/JS in a sandbox, or WASM) for tools, effects, importers and exporters.
- Scripting console for power users.

---

## 9. Phases with prompts

Each prompt is self-contained. Paste it into a fresh session opened in the project folder.

---

### Phase 0: Repo bootstrap and architecture

**Goal:** a running Tauri app with the full folder structure, core crates stubbed, CI, and the Command, Job and Event systems designed properly.

**Done when:**
- [ ] `pnpm tauri dev` opens a window on Windows, macOS and Linux (checked on Linux; Windows and macOS still need a hand check)
- [x] Cargo workspace with every crate from section 4 (stubs are fine)
- [x] `vellum-core` has a working Command trait, undo/redo stack, event bus and settings store, with tests
- [x] Job queue with progress, cancel and pause, with tests
- [x] CI runs fmt, clippy, tests, ESLint and Vitest on all three OSes
- [x] `ARCHITECTURE.md` explains the design decisions

**Model:** Fable 5.2 · **Effort:** high

```text
read PLAN.md in the project root fully before doing anything. you are building phase 0 of vellum, an all in one desktop media editor.

goal: set up the repo and the core architecture that every later phase depends on. get this right, mistakes here are very expensive later.

do this:
1. create a tauri 2 app with a react + typescript + vite front end (pnpm), zustand, radix ui and tailwind, matching the folder layout in section 4 of PLAN.md.
2. create a cargo workspace with every crate in section 4. stubs are fine except vellum-core.
3. in vellum-core, design and implement:
   - a Command trait (do, undo, merge-with-previous for things like slider drags, a human readable name, serialisable so macros can record it)
   - an undo/redo history that supports named snapshots and branching history
   - a typed event bus between rust and the ui
   - a settings store (json on disk, versioned, with migration support)
4. design and implement the job queue: priority, progress, eta, cancel, pause/resume, retry, max parallel jobs by type (gpu jobs vs cpu jobs), and events to the ui.
5. write unit tests for all of the above.
6. set up github actions ci for windows, macos and linux running cargo fmt --check, cargo clippy -D warnings, cargo test, eslint, prettier check and vitest.
7. write ARCHITECTURE.md explaining every major decision and the tradeoffs you considered.

follow section 5 (conventions) of PLAN.md exactly, including the comment style and no em dashes. when finished, tick the phase 0 boxes in PLAN.md, add a CHANGELOG.md entry, and give me a short summary of what to check by hand.
```

---

### Phase 1: App shell and UX foundations

**Goal:** the app feels premium before it does anything: home screen, tabs, docking panels, command palette, settings, shortcuts, themes, autosave and crash recovery.

**Done when:**
- [x] Home screen with recent files, pinned projects and quick tool cards
- [x] Drag and drop anywhere routes files to the right workspace (placeholder workspaces OK)
- [x] Dockable panels with saved layouts, tabs for documents
- [x] Command palette (`Ctrl+K`) searching commands, settings and recent files
- [x] Shortcut editor with conflict detection and Photoshop/Premiere/Resolve presets
- [x] Light/dark/high-contrast themes, UI scale, accent colour
- [x] Autosave, crash recovery prompt, notification centre, job queue panel

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md (especially sections 5, 6 and 7) and ARCHITECTURE.md before starting. you are building phase 1 of vellum: the app shell and ux foundations.

the app should feel fast, calm and premium, like a mix of figma, linear and davinci resolve. use the frontend-design approach: pick a distinctive but professional visual direction, dark theme first, with a proper design token system (colours, spacing, radius, type scale, motion durations).

build:
1. home screen: recent files with thumbnails, pinned projects, big quick tool cards ("export to 10 mb", "video to gif", "remove background", "convert", "screen record"), and a drop zone.
2. global drag and drop: a file dropped anywhere opens in the right workspace based on type. dropping onto a quick tool card opens that tool. show a nice drop overlay that tells you what will happen.
3. document tabs, tear-off windows, and a dockable panel system with saved and switchable layouts.
4. command palette on ctrl+k with fuzzy search over every registered command, settings, recent files and presets. commands come from the vellum-core command registry.
5. settings window with search, and a shortcut editor with conflict detection plus photoshop, premiere, resolve and audacity presets.
6. themes (dark, light, high contrast), accent colour, ui scale, compact mode, reduced motion.
7. autosave every n seconds (setting), crash recovery prompt on next launch, notification centre, and a job queue panel wired to the phase 0 job system (pause, cancel, reorder, retry, open result when done).
8. empty states, loading skeletons and tooltips with shortcuts for everything.

workspaces (image, video, audio, etc) can be placeholders for now. write playwright tests for drag and drop routing, the command palette and crash recovery. follow section 5 of PLAN.md exactly. tick the phase 1 boxes and update CHANGELOG.md when done.
```

---

### Phase 2: Media core

**Goal:** read anything, probe it, make thumbnails and waveforms, cache intelligently, and encode with hardware acceleration.

**Done when:**
- [x] FFmpeg integration for decode and encode with hardware acceleration detection
- [x] Image decoding for PNG, JPG, WebP, AVIF, HEIC, TIFF, GIF, RAW, SVG, PSD (flattened preview) (HEIC photos stored in tiles, like most iPhone photos, need FFmpeg 7.1 or newer)
- [x] Media probe returns a unified `MediaInfo` struct
- [x] Thumbnail, filmstrip and waveform generation as cached jobs
- [x] SQLite-backed cache with size limit and cleanup
- [x] File info panel in the UI with "strip metadata"

**How it was built, where it differs from the prompt below** (details and reasons in `ARCHITECTURE.md` section 22):
- FFmpeg runs as the `ffmpeg` and `ffprobe` programs instead of being linked through `ffmpeg-next`, so builds need nothing native, a crashing decoder cannot take the app down, and the LGPL story stays simple. Phase 8 decides whether playback needs a linked decoder.
- Camera raw uses `rawler` (pure Rust) instead of libraw, and AVIF, HEIC and JPEG XL are decoded by FFmpeg instead of separate native libraries.
- Big images are previewed by scaling while decoding (JPEG DCT scaling, row by row PNG) instead of libvips. Tiled editing of huge images is phase 5's canvas.
- Benchmark, 4K H.265, software decode on 4 cores: probe about 120 ms, thumbnail 230 to 350 ms, 8 frame filmstrip about 1.1 s.

**Model:** Opus 5.5 · **Effort:** xhigh

```text
read PLAN.md and ARCHITECTURE.md first. you are building phase 2 of vellum: the media core, in crates/vellum-media.

build:
1. ffmpeg integration (ffmpeg-next, lgpl build) for decode and encode. detect available hardware encoders and decoders (nvenc, quicksync, amf, videotoolbox, vaapi) at startup and expose them. always fall back to software cleanly.
2. image decoding through one api for png, jpg, webp, avif, heic, tiff, gif (all frames), camera raw (libraw), svg (resvg, any size) and psd (flattened preview for now). use libvips or tiling for very large images so memory stays sane.
3. a probe function returning one MediaInfo struct: type, dimensions, duration, fps (including variable frame rate detection), codecs, bitrate, audio channels and sample rate, colour space, hdr metadata, rotation, and all metadata tags.
4. thumbnail, filmstrip (n frames across a video) and waveform peak generation, all as jobs, all cached.
5. a sqlite backed cache index (crates/vellum-media/cache) keyed by file path + size + modified time, with a size limit setting and a one click clean in settings.
6. a frame accurate seek api for video that later phases will use for scrubbing.
7. a file info panel in the ui showing MediaInfo nicely, with "copy info" and "strip metadata" (writes a copy, never overwrites).

include a folder of small test media files (generate them with ffmpeg in a script, do not commit big files) and test every format. benchmark thumbnail generation for a 4k h265 file and report the number. follow section 5 of PLAN.md exactly. tick the phase 2 boxes and update CHANGELOG.md.
```

---

### Phase 3: Quick Tools (Export to X MB, Video to GIF, Converter)

**Goal:** ship the must-have features early so the app is already useful.

**Done when:**
- [x] Export to X MB works for video, image, GIF and audio, default 10 MB, lands under the limit every time in tests
- [x] Platform presets (editable), MB/MiB toggle, safety margin, strategy picker
- [x] Video to GIF tool with trim, crop, fps, speed, boomerang, captions, and WebP/APNG/MP4 alternatives (and AVIF)
- [x] GIF frame editor and optimiser
- [x] Universal converter with instant remux where possible
- [x] Batch mode for all three, and "copy result as file" to clipboard

**How it was built, where it differs from the prompt below** (details and reasons in `ARCHITECTURE.md` section 23):
- GIFs use FFmpeg's palette filters and Vellum's own lossy pass instead of gifski, which is AGPL. PNG palettes use quantette and color_quant instead of libimagequant, which is GPL.
- The export code lives in `crates/vellum-export` as `size/`, `anim/` and `convert.rs`, not `vellum-export/size_target`.
- Discord's free limit is 20 MB since August 2026, so the Discord preset is 20 MiB while the starting size stays 10 MB as asked. That is a choice for the owner to revisit.
- GIF tools is its own quick tool card, since the frame editor, optimiser, splitter and GIF to video did not fit inside Video to GIF.
- The converter also joins files into one, cuts files into parts and renames files from a pattern with a preview (section 8.1). Rename is an undoable command rather than a job.
- Not done yet: Explorer and Finder context menu entries, which are left for phase 16 as the prompt allows.

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md (especially section 8.1) and ARCHITECTURE.md first. you are building phase 3 of vellum: the quick tools. these are the features the owner cares about most, so the ux must be excellent.

1. export to x mb (crates/vellum-export/size_target):
   - target size input defaulting to 10 mb, preset chips (discord, discord nitro basic, discord nitro, email, whatsapp, telegram, x/twitter, custom). look up the current official upload limits for each platform and use those as editable defaults in settings.
   - mb vs mib toggle and a safety margin (default 3 percent).
   - video: compute target video bitrate from duration, target size and audio bitrate, then two pass encode. if the bitrate per pixel is too low for good quality, step down resolution and/or fps automatically and tell the user what changed. if the output is still over, re-encode with a corrected bitrate. options: trim first, mute, mono, codec choice (h264 default for compatibility, h265, av1, vp9), strategy (best quality, keep resolution, keep fps, keep audio).
   - image: binary search quality for jpg, webp and avif, then resize if needed. png gets palette quantisation and optimisation.
   - gif: binary search over colours, lossy level, fps and size using gifski. suggest mp4 or animated webp when it would be much smaller.
   - audio: compute bitrate, encode opus or mp3.
   - show a size estimate before encoding, a before/after compare slider after, and the final size with a clear success state. add "copy as file" to the clipboard so it can be pasted into discord, plus "show in folder".
   - write tests proving that a range of inputs always lands under the target.
2. video to gif: mini timeline with frame accurate in/out handles, crop, resize, fps with live size estimate, speed, reverse, boomerang, loop count, caption presets (classic meme top/bottom text, modern caption bar), dithering and colour options, and export to gif, animated webp, apng, avif or mp4 loop. "fit to x mb" checkbox reuses the size engine.
3. gif tools: gif to video, frame editor (delete, duplicate, reorder, per frame delay), optimiser, splitter.
4. universal converter: any to any where it makes sense, remux without re-encoding when the codecs allow it, extract audio, extract frames, images to video, heic to jpg, svg to png at any size, ico maker.
5. batch mode for every tool: drop many files, see a list with per file progress and results.
6. add explorer/finder context menu entries ("compress to 10 mb", "make gif", "convert to...") if feasible now, otherwise leave a clear todo for phase 16.

follow section 5 of PLAN.md exactly. tick the phase 3 boxes and update CHANGELOG.md.
```

---

### Phase 4: GPU render engine and colour management

**Goal:** one fast GPU node graph that the image editor, video editor, colour page and motion graphics all share.

**Done when:**
- [x] wgpu renderer drawing into the Tauri window (native surface or shared texture) at 60+ fps (as display tiles drawn with WebGL, see below)
- [x] Node graph: sources, effects, blend, mask, transform, output; cached and invalidated correctly
- [x] All Photoshop blend modes in WGSL with golden tests
- [x] Colour management: linear working space, sRGB/Rec.709/Rec.2020/P3/ACES transforms, ICC profiles, 16f and 32f pipelines
- [x] Tiled rendering for huge images, CPU fallback path
- [x] Benchmarks recorded in `ARCHITECTURE.md` (measured without a graphics card; the GPU rows need a run on real hardware)

**How it was built, where it differs from the prompt below** (details and reasons in `ARCHITECTURE.md` section 24):
- Frames reach the window as 256 pixel display tiles that the webview draws with WebGL2, not through a native child surface or a shared texture. Pan, zoom and rotate never wait for Rust, and overlays drawn in the page (phase 5's selections, handles and guides) can never drift from the picture. Full-rate video playback in phase 8 will want a native surface presenter for the playback viewer; the engine side is ready for either.
- Every node has a WGSL kernel and a CPU twin of the same maths in one file, and the tests compare them, so the CPU fallback always gives the same picture.
- ICC profiles use moxcms (pure Rust) rather than lcms2. Profiles embedded in PNG, JPEG, WebP and TIFF are read; the display profile is a file picked in Settings, Graphics and display, since reading the monitor's profile needs per-OS code.
- The image workspace now shows the opened picture through the engine (view only; editing is phase 5), so the renderer can be tried in the real app.
- Not exact yet: Photoshop's special handling of Fill for eight blend modes (Color Burn, Linear Burn, Color Dodge, Linear Dodge, Vivid Light, Linear Light, Hard Mix, Difference) below 100 percent. Phase 5 should compare those with Photoshop's own output.

**Model:** Fable 5.2 · **Effort:** max

```text
read PLAN.md and ARCHITECTURE.md first. you are building phase 4 of vellum: the gpu render engine in crates/vellum-gpu. this is the most important technical piece in the project. the image editor, video timeline, colour grading page and motion graphics will all render through it, so design it for all four.

requirements:
1. wgpu based renderer that displays inside the tauri window with smooth 60+ fps pan, zoom and rotate of the viewport. pick the best approach for getting gpu frames into the tauri ui (native child surface vs shared texture vs other) and justify it in ARCHITECTURE.md.
2. a render node graph: source nodes (image, video frame, solid, gradient, text later), effect nodes, blend nodes, mask nodes, transform nodes, and output. nodes declare parameters in a typed way so the ui can auto generate controls and so everything can be keyframed later. result caching with correct invalidation when parameters change.
3. every photoshop blend mode implemented in wgsl, matching photoshop maths as closely as possible, with golden image tests.
4. colour management: linear light working space, transforms between srgb, rec.709, rec.2020, display p3 and aces (cg and cct), icc profile support for images and the display, 8 bit, 16 bit float and 32 bit float pipelines, correct premultiplied alpha handling.
5. tiled rendering for images bigger than gpu limits (eg 30000x30000), with memory budgets.
6. a cpu fallback path for machines without a usable gpu, and automatic device lost recovery.
7. a small set of example effect nodes (gaussian blur, levels, hue/sat, sharpen) so later phases have patterns to copy. make adding a new effect a single file plus a registration line.
8. benchmarks: 4k frame through a 10 node graph, and a 20000x20000 pan. record the results in ARCHITECTURE.md.

think carefully about threading, gpu memory, and how video playback will feed frames in phase 8. follow section 5 of PLAN.md exactly. tick the phase 4 boxes and update CHANGELOG.md.
```

---

### Phase 5: Image editor core

**Goal:** a Photopea-grade canvas: documents, layers, masks, selections, transforms, brushes.

**Done when:**
- [x] Raster documents with layers, groups, masks, clipping masks, blend modes, opacity/fill
- [x] Selection tools: marquee, lasso, polygon, magnetic lasso, magic wand, quick selection, colour range, transform selection
- [x] Free transform, warp, perspective, rotate, flip, canvas/image size, crop with straighten
- [x] Brush engine with pressure/tilt, smoothing, eraser, gradient, bucket, eyedropper
- [x] Undo/redo for everything, history panel with snapshots
- [x] PSD import with layers; native `.vellum` save; PNG/JPG/WebP export (and AVIF, PSB and export to X MB)
- [x] Rulers, guides, smart guides, grid (and the navigator)

**How it was built, where it differs from the prompt below** (details and reasons in `ARCHITECTURE.md` section 25):
- Phase 5 was split in two to keep each session a sensible size. The first half is the document model, compositing through the phase 4 engine, every selection tool and modifier, the brush engine and painting tools, undo for everything, the native file and the workspace UI. The second half is transforms, crop, canvas and image size, transform selection, layered PSD import, export with export to X MB, rulers, guides, smart guides, the grid and the navigator.
- Free transform and warp preview through the phase 4 transform node and commit by rendering the same graph, so the preview is the result. The node gained a mesh mode (a grid saying where each output point reads from) so warps cost what a matrix costs.
- PSD and PSB are read by Vellum's own reader, since the `psd` crate has no masks, fill or 16 and 32 bit layers. CMYK, Lab, bitmap and indexed files still open flattened. Text layers keep Photoshop's pixels and are marked as text (editable type is phase 6).
- Export embeds the colour profile in PNG, JPEG and WebP. AVIF is converted to sRGB instead, because FFmpeg cannot write a profile into it.
- Guides are part of the document and undo; rulers, the grid and snapping are view settings per window.
- The `.vellum` file is a zip holding the project folder (manifest, one file per raster, profile, thumbnail) rather than a bare folder, so it can be picked in an open dialog and attached to an email; an unzipped one opens too.
- Undo keeps the document before and after each command, which costs a few pointer copies because tiles are shared and copied only when written.
- A workspace's own shortcut now takes over a global one inside that workspace (Ctrl+E merges down in the image editor, exports to a size elsewhere). The window already behaved this way; the conflict checker now agrees.
- Brush latency on a 4K canvas, without a graphics card: about 8 ms median from pointer samples to finished display tiles fitted to the screen, 11 ms at 100%, after the second half gave rasters a revision per 64 pixel block (it was 14 and 22 ms).

**Model:** Fable 5.2 · **Effort:** xhigh

```text
read PLAN.md (especially section 8.2) and ARCHITECTURE.md first. you are building phase 5 of vellum: the image editor core, in crates/vellum-image and ui/src/workspaces/image. it renders through the phase 4 gpu engine.

build:
1. document model: raster layers, groups, layer masks, clipping masks, all blend modes, opacity and fill opacity, lock options, colour labels, layer search. 8, 16 and 32 bit per channel.
2. selection system with an actual selection mask (not just shapes): marquee (rect, ellipse), lasso, polygon lasso, magnetic lasso, magic wand (tolerance, contiguous, sample all layers), quick selection brush, colour range. add, subtract and intersect modes, feather, grow, shrink, border, invert, transform selection, save/load selections, quick mask mode, marching ants that stay smooth at any zoom.
3. transforms: free transform with handles, rotate, scale, skew, distort, perspective, warp mesh, flip, numeric input, interpolation choice. canvas size and image size dialogs. crop tool with straighten, aspect presets and perspective crop.
4. brush engine: pressure and tilt (pen tablets via pointer events), size, hardness, opacity, flow, spacing, smoothing/stabiliser, scatter, texture, and [ / ] shortcuts. tools: brush, pencil, eraser, gradient (all types), paint bucket, eyedropper with sample size. brush strokes must be smooth and low latency on a 4k canvas.
5. every action as an undoable command, history panel with named snapshots.
6. psd import keeping layers, masks, blend modes and text (as raster for now). native save to the .vellum project format. export png, jpg, webp, avif with the export to x mb option from phase 3.
7. rulers, guides, smart guides with snapping, grid, pixel grid at high zoom, navigator panel.
8. photoshop style shortcuts by default (v, m, l, w, b, e, g, i, ctrl+t etc).

write golden image tests for layer compositing and selection maths, and benchmark brush latency. follow section 5 of PLAN.md exactly. tick the phase 5 boxes and update CHANGELOG.md.
```

---

### How a step runs (every step prompt points here)

From phase 6 on, each phase is split into lettered steps (6a, 6b, ...). One step is one session. The phase numbers did not change, so notes elsewhere that say "phase 16" still mean the same thing.

- Do only your step. Do not start the next one, even if there is time left.
- Read only what you need: section 5, your step, the section 8 part it names, and the `ARCHITECTURE.md` sections it names. Find your way around the code by searching it (or `graphify-out/` if it exists) rather than reading whole crates.
- Work on a branch named after the step (for example `step-6b-adjustments`), started from `main`.
- Commit after each numbered item and push. A session that runs out of usage then loses nothing.
- Before the last commit, run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test` for the crates you touched, and `pnpm typecheck`, `pnpm lint`, `pnpm format:check` and `pnpm test` if the UI changed. Fix what fails.
- If you are running low, stop at a clean commit and write what is left as unticked boxes under your step, so the same prompt can finish it next session.
- When done: tick the step's boxes, add a short `CHANGELOG.md` entry, add to `ARCHITECTURE.md` only when you made a design decision worth keeping, and open a pull request to `main`.

---

### Phase 6: Image editor tools

**Goal:** fill in the long tail: adjustments, filters, retouching, text, shapes, layer styles, smart objects, PSD export.

The first session built the engine and document side of the whole phase (now step 6a). The window side is split into 6b to 6g.

#### 6a: Engine and document side (done)

- [x] Every adjustment and a Camera Raw node, as render nodes with GPU and CPU paths, with golden images
- [x] The filter gallery's filters as render nodes, with golden images
- [x] Fill layers, smart objects (embedded and linked) and smart filters in the document
- [x] Retouching, liquify and puppet warp engines
- [x] Text, shape and path layers, vector masks, fonts and Google Fonts downloads
- [x] Layer styles, PSD and PSB saving with layers, artboards, export layers and artboards, save for web previews
- [x] IPC commands, app commands and shortcuts for all of the above

**Model:** Opus 5.5 · **Effort:** high

#### 6b: Adjustments panel and Camera Raw

**Done when:**
- [ ] Adjustments panel adds any adjustment layer; a Properties panel edits it live, with presets
- [ ] Curves and levels have proper graph editors with histograms
- [ ] Camera Raw dialog with every section and a before and after view
- [ ] Auto tone, auto contrast, auto colour, equalize and one click enhance in the Image menu

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 6b, and section 8.2 (adjustments). read ARCHITECTURE.md sections 24 and 25. you are doing step 6b of vellum.

the engine side already exists (adjustment nodes, adjustment layers, presets, auto tone and friends, the node catalogue ipc that describes every parameter). build the window side in ui/src/workspaces/image:
1. an adjustments panel that adds any adjustment layer above the current layer, and a properties panel that edits the selected adjustment layer live. build controls from the node catalogue so new adjustments need no new ui code. presets: pick, save, delete.
2. curves (per channel, add and drag points, histogram behind) and levels (input, output, gamma, per channel, histogram) as proper editors, not sliders.
3. the camera raw dialog: every section from the plan, a before and after toggle and split view, and apply as a smart filter or an adjustment layer.
4. auto tone, auto contrast, auto colour, equalize and enhance in the image menu and the palette.

add vitest tests for the control builders and curve maths, and a playwright test that adds a curves layer and changes it.
```

#### 6c: Filter gallery, smart objects and smart filters

**Done when:**
- [ ] Filter gallery dialog with live thumbnails of every filter on the current picture
- [ ] Filter dialogs preview live on the canvas
- [ ] Layers panel shows smart objects and their smart filters (edit, hide, fade, reorder, mask)
- [ ] Convert to smart object, place embedded or linked, edit contents, relink missing files

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 6c, and section 8.2 (filters, smart objects). read ARCHITECTURE.md sections 24 and 25. you are doing step 6c of vellum.

the engine side exists: filter nodes, filter thumbnails ipc, live filter preview through the preview request, smart objects and smart filters in the document, image_place. build the window side:
1. the filter gallery: categories, a grid of live thumbnails of the current layer, search, and a settings pane built from the node catalogue. the canvas previews live while you drag; ok applies it as one undoable step.
2. in the layers panel, smart objects show a badge and their smart filters underneath, each with an eye, a blend and fade option, drag to reorder, double click to edit, and the smart filter mask.
3. convert to smart object, place embedded, place linked, edit contents (opens the inner document in a tab and updates the parent on save), relink when a linked file is missing, rasterize.

add vitest tests for the filter list logic and a playwright test that applies a smart filter, edits it and hides it.
```

#### 6d: Retouching tools, liquify and puppet warp

**Done when:**
- [ ] Clone, healing, spot healing, patch, content-aware move, red eye, dodge, burn, sponge, blur, sharpen and smudge in the toolbox with their options
- [ ] Clone and healing show the source point and an overlay while painting
- [ ] Liquify dialog (forward warp, pucker, bloat, reconstruct, smooth, twirl, freeze mask)
- [ ] Puppet warp with pins on the canvas

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 6d, and section 8.2 (retouching). read ARCHITECTURE.md section 25 (brush engine, live tools). you are doing step 6d of vellum.

the engines exist (retouch settings in the brush, heal, liquify and puppet warp sessions over ipc). build the window side:
1. toolbox groups and options bar for clone stamp (aligned, sample all layers, alt click to set the source), healing brush, spot healing, patch, content-aware move, red eye, dodge, burn, sponge, blur, sharpen and smudge, with photoshop shortcuts (s, j, o and their shift cycles).
2. a source crosshair and a clone overlay that follows the brush.
3. the liquify dialog: tools on the left, brush size, pressure and rate, a freeze mask brush, reconstruct and restore all. the preview runs through the existing gpu session; ok commits it as one step.
4. puppet warp: a mesh shown over the layer, click to add pins, drag to bend, alt click to remove, rigidity and density options.

vitest for the tool settings and pin maths, and a playwright test that clones a few dabs.
```

#### 6e: Text tool and fonts

**Done when:**
- [ ] Point and paragraph text typed straight on the canvas, with a caret, selection and IME support
- [ ] Character and paragraph panels, OpenType features and variable font axes
- [ ] Text on a path and warp text
- [ ] Font browser with previews, favourites and Google Fonts download; missing fonts dialog

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 6e, and section 8.2 (text). read ARCHITECTURE.md section 25. you are doing step 6e of vellum.

text layers, warp, opentype, variable axes, the font library and google fonts downloads exist in the engine and over ipc. build the window side:
1. the type tool (t): click for point text, drag for a paragraph box, type on the canvas with a caret, selection, arrow keys, copy and paste, and ime input for non latin scripts. enter commits, esc cancels, double click a text layer to edit.
2. character and paragraph panels: font, style, size, leading, tracking, kerning, colour, alignment, indents, spacing, opentype features and variable font axes as sliders.
3. text on a path (click a path with the type tool) and the warp text dialog.
4. a font browser: live previews in the chosen font, search, filters, favourites, recent, and google fonts with a download button that runs as a job. when a file uses fonts you do not have, a dialog lists them and offers to download or replace each.

vitest for caret and selection maths, and a playwright test that types, styles and edits a text layer.
```

#### 6f: Shapes, pen tools and paths

**Done when:**
- [ ] Rectangle (live corners), ellipse, polygon and star, line and custom shape tools
- [ ] Pen and curvature pen, path selection and direct selection, anchor add and delete
- [ ] Paths panel, path booleans, vector masks
- [ ] Shape fill and stroke options in the options bar

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 6f, and section 8.2 (shapes and paths). read ARCHITECTURE.md section 25. you are doing step 6f of vellum.

shape layers, vector paths, path operations, vector masks and saved paths exist in the engine. build the window side:
1. shape tools (u and shift u): rectangle with live corner handles, ellipse, polygon and star, line with arrowheads, custom shapes from a small built in library. shift and alt work like photoshop. fill, stroke, stroke width, dashes and alignment in the options bar.
2. pen (p) and curvature pen: click for corners, drag for curves, close on the first point, alt to break handles. path selection (a) and direct selection to move whole paths, anchors and handles.
3. a paths panel: work path, saved paths, make selection from path, stroke and fill a path, and booleans (combine, subtract, intersect, exclude).
4. vector masks: add from the current path, edit with the pen tools, shown in the layers panel next to pixel masks.

vitest for the bezier hit testing and handle maths, and a playwright test that draws a pen path and turns it into a shape.
```

#### 6g: Layer styles, Photoshop saving and export dialogs

**Done when:**
- [ ] Layer style dialog with every effect, live on the canvas, with presets
- [ ] Copy, paste and clear styles, and a styles panel
- [ ] Save as PSD or PSB, export layers to files, export artboards, artboard tool
- [ ] Save for web with a side by side comparison and the real file size
- [ ] Phase 6 checked in the real app; all phase 6 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 6g, and section 8.2 (layer styles, export). read ARCHITECTURE.md section 25. you are doing step 6g of vellum, the last step of phase 6.

the engine side exists (layer styles, style presets, psd saving, artboards, export layers and artboards, web previews). build the window side:
1. the layer style dialog: a list of effects with checkboxes on the left (drop shadow, inner shadow, outer and inner glow, bevel and emboss, satin, colour, gradient and pattern overlay, stroke), settings on the right, live on the canvas, ok and cancel as one step. fx badges in the layers panel.
2. copy, paste and clear styles from the layer menu, and a styles panel with presets (apply, save, delete).
3. file menu: save as psd or psb, export layers to files, export artboards, and the artboard tool (shift v). all saving runs as jobs.
4. save for web: original and optimised side by side with a shared zoom and pan, format and quality controls, the real encoded size and time, and the export to x mb option.
5. run the app, open a psd, use a tool from each of steps 6b to 6f, save as psd and reopen it. fix what you find, then tick the phase 6 boxes.

playwright test for the layer style dialog and save for web.
```

---

### Phase 7: AI toolkit

**Goal:** local, offline AI tools, led by the background remover where **you choose what gets deleted**.

#### 7a: AI runtime and model manager

**Done when:**
- [ ] ONNX Runtime (`ort`) with DirectML, CUDA, CoreML and CPU, picked automatically
- [ ] Model manager: download on first use with progress, checksum, a settings page with sizes and delete
- [ ] `THIRD_PARTY_LICENSES.md` lists every model and whether its licence allows shipping
- [ ] Out of GPU memory falls back to tiles, then the CPU, with a clear message

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 7a, and section 8.2 (background remover, ai extras). read ARCHITECTURE.md sections 22 and 24. you are doing step 7a of vellum, in crates/vellum-ai.

1. onnx runtime through the ort crate, with directml on windows, cuda where present, coreml on macos and a cpu fallback. pick the best provider at start, and let settings override it.
2. a model registry and manager: each model has a name, url, size, sha256, licence and what it is for. models download on first use as jobs with progress and cancel, are verified, and live in the app data folder. a settings page lists installed models with their size and a delete button.
3. tiling helpers for big images, and out of memory handling: retry in tiles, then on the cpu, and tell the user what happened.
4. check the licence of every model the later steps need (birefnet or u2net, sam 2, lama, real-esrgan, gfpgan or codeformer, a colouriser, depth anything, an ocr model) and record them in THIRD_PARTY_LICENSES.md. flag any that cannot ship and suggest a replacement.

tests for the registry, checksum checking and tiling maths. no ui beyond the settings page.
```

#### 7b: Background remover

**Done when:**
- [ ] One click background removal into a layer mask
- [ ] Keep and erase brushes on the mask with a live overlay
- [ ] Edge softness, hair refine and colour decontamination
- [ ] New background: transparent, colour, blurred original, image; shadow keep, add or remove

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 7b, and section 8.2 (background remover). read the ARCHITECTURE.md section step 7a added. you are doing step 7b of vellum. this is a headline feature, so the ux must be excellent.

1. one click remove background on the current layer with the best quality permissive model from 7a. the result is always a layer mask, never deleted pixels.
2. a remove background mode: the mask shown as an overlay, a keep brush and an erase brush, and a toggle to see the result on a checkerboard, colour or the original.
3. edge softness, hair and fur refine (guided or matting refinement on the mask edge) and colour decontamination.
4. new background options (transparent, solid colour, blurred original, an image) and shadow options (keep, add a drop shadow, remove).
5. the remove background quick tool card on the home screen opens this mode.

runs as a job with progress and cancel. tests for the mask refine maths; a playwright test of the mode with a fake model result.
```

#### 7c: Click to select with SAM 2

**Done when:**
- [ ] SAM 2 encoder and decoder running, image embedding cached per layer
- [ ] Click to keep and click to remove in the background remover, with a hover preview
- [ ] Smart Select tool: hover highlights, click adds, Alt click subtracts, gives a normal selection

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 7c, and section 8.2 (smart select). read the ARCHITECTURE.md sections 7a and 7b added. you are doing step 7c of vellum.

1. sam 2 through vellum-ai: run the image encoder once per layer as a job and cache the embedding; the prompt decoder must be fast enough to run on hover.
2. in the background remover from 7b: click to keep and click to remove objects, with a live preview of what the click would pick while hovering.
3. a smart select tool in the image toolbox: hover highlights the object under the cursor, click adds it to the selection, alt click subtracts, shift click adds more. the result is an ordinary selection mask that every other tool understands.

tests for prompt building and mask merging. measure hover latency on cpu and record it.
```

#### 7d: Batch background removal and AI selection extras

**Done when:**
- [ ] Batch remove background for many photos with per file results
- [ ] Select subject and select sky in the Select menu
- [ ] Liquify face-aware sliders (eyes, nose, mouth, face shape)

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 7d, and section 8.2. read the ARCHITECTURE.md sections steps 7a to 7c added. you are doing step 7d of vellum.

1. batch remove background: drop many files, pick the background option and output format, see a list with per file progress and thumbnails, and open any result in the editor to fix its mask. reuse the phase 3 batch list.
2. select subject and select sky in the select menu, using the 7b and 7c models.
3. face aware liquify: detect faces and landmarks with a small permissive model, and add sliders for eyes, nose, mouth and face shape to the 6d liquify dialog.

tests for the batch list and landmark to warp maths.
```

#### 7e: Object remover, upscale and face restore

**Done when:**
- [ ] Object remover (LaMa): paint over something or use the selection, it disappears
- [ ] Upscale 2x and 4x (Real-ESRGAN), tiled for big images
- [ ] Face restore and denoise

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 7e, and section 8.2 (ai extras). read the ARCHITECTURE.md section step 7a added. you are doing step 7e of vellum.

1. object remover with lama: a brush that marks what to remove, or use the current selection. the result goes on a new layer so it can be undone or masked.
2. upscale 2x and 4x with real-esrgan, tiled with overlap so seams never show, as an image menu command and a quick tool.
3. face restore (gfpgan or codeformer, whichever can ship) and ai denoise.

all as jobs with progress and cancel. tests for tile overlap blending. measure each on a 12 megapixel image on cpu (and gpu if present) and record the numbers.
```

#### 7f: Colourise, depth blur, sky replace and OCR

**Done when:**
- [ ] Colourise black and white photos
- [ ] Depth map with fake portrait blur
- [ ] Sky replacement with colour matching
- [ ] OCR to copy text out of an image
- [ ] Phase 7 checked in the real app; all phase 7 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 7f, and section 8.2 (ai extras). read the ARCHITECTURE.md section step 7a added. you are doing step 7f of vellum, the last step of phase 7.

1. colourise, as a new layer.
2. depth map (depth anything) and a portrait blur that uses it, with a focus point you click and a blur amount, as a smart filter if possible.
3. sky replacement: find the sky (7d select sky), swap in a picked image, match the foreground colour to it, all on layers.
4. ocr on the whole image or a selection, with the text shown in a panel to copy.
5. run the app, try every phase 7 tool once, fix what you find and tick the phase 7 boxes.

tests where the maths is ours (blur weights, colour matching).
```

---

### Phase 8: Video timeline engine

**Goal:** the beating heart of the video editor: data model, smooth playback, scrubbing, proxies, frame cache, audio sync.

#### 8a: Timeline data model

**Done when:**
- [ ] Projects, sequences, tracks, clips, nested sequences, adjustment layers, transitions and effect stacks
- [ ] A keyframe system with bezier easing that any parameter can use
- [ ] Rational time maths that is frame and sample exact, with tests
- [ ] Every change is a vellum-core command; projects save and load

**Model:** Opus 5.5 · **Effort:** xhigh

```text
read PLAN.md section 5, "how a step runs" in section 9, step 8a, and section 8.3. read ARCHITECTURE.md sections 22 and 24. you are doing step 8a of vellum in crates/vellum-timeline. everything in phases 9 to 13 builds on this model, so design it carefully and write down the reasons in ARCHITECTURE.md.

1. rational time (no floats for positions): frame rates like 23.976 and 29.97 drop frame, sample rates, conversions and rounding rules.
2. the model: projects, sequences (resolution, fps, colour space), unlimited video and audio tracks, clips (source range, timeline range, speed), nested sequences, adjustment layers, transitions and effect stacks.
3. a generic keyframe system with linear, hold and bezier easing that any parameter can use.
4. every change as a vellum-core command so it is undoable and recordable; save and load the project inside the .vellum format.

no ui and no playback in this step. thorough tests for the time maths and for the model's invariants (no overlaps on a track, ranges stay inside the source).
```

#### 8b: Playback engine

**Done when:**
- [ ] Decode ahead on background threads, composite through the phase 4 graph
- [ ] Frames presented locked to the audio clock, with drift correction
- [ ] Mixed frame rates and variable frame rate sources play correctly
- [ ] Dropped frames counted and shown; a viewer in the video workspace

**Model:** Opus 5.5 · **Effort:** xhigh

```text
read PLAN.md section 5, "how a step runs" in section 9, step 8b, and section 8.3. read ARCHITECTURE.md section 24 (render engine, getting pixels into the window) and the step 8a section. you are doing step 8b of vellum.

1. a playback scheduler: decode ahead on background threads through vellum-media, composite each frame through the phase 4 render graph, and present at the right time locked to the audio clock (a simple cpal output is enough for now; phase 11 builds the full audio engine).
2. mixed frame rates, variable frame rate sources and drift correction.
3. drop frames gracefully on weak machines and show a dropped frame counter.
4. a viewer in the video workspace. phase 4 noted full rate playback may need a native surface presenter instead of display tiles; measure first and pick, and record why in ARCHITECTURE.md.

tests for the scheduler's timing decisions with a fake clock. record playback numbers for 1080p and 4k.
```

#### 8c: Scrubbing, frame cache and proxies

**Done when:**
- [ ] Frame cache with a memory budget
- [ ] Instant scrubbing while dragging the playhead, with audio scrub
- [ ] Automatic proxies for heavy codecs as background jobs, switched in without the user noticing

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 8c, and section 8.3. read the ARCHITECTURE.md sections steps 8a and 8b added. you are doing step 8c of vellum.

1. a frame cache with a memory budget from settings, keyed so edits invalidate only what changed.
2. scrubbing: the viewer answers instantly while dragging, showing the nearest cached or keyframe first and refining; short audio snippets play while scrubbing.
3. proxies: detect heavy sources (h265, raw, long gop 4k and up), make proxies as background jobs, use them for playback and the original for export, with a toggle in the viewer.

tests for cache eviction and proxy selection.
```

#### 8d: Media pool and source monitor

**Done when:**
- [ ] Media pool with bins, smart bins, list and grid views and metadata columns
- [ ] Hover scrub thumbnails
- [ ] Source monitor with in and out points and markers

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 8d, and section 8.3. read the ARCHITECTURE.md sections steps 8a to 8c added. you are doing step 8d of vellum, in ui/src/workspaces/video.

1. a media pool panel: import by drop or dialog, bins, smart bins (rules on metadata), list and grid views, metadata columns from mediainfo, sort and search.
2. hover scrub on grid thumbnails using the phase 2 filmstrips.
3. a source monitor using the 8b viewer: play, in and out points, markers with notes, and drag to the timeline area (the timeline itself is 8e).

vitest for smart bin rules, playwright for importing and setting in and out.
```

#### 8e: Timeline UI

**Done when:**
- [ ] Clips with filmstrips and waveforms; drag, trim edges, razor, snapping
- [ ] Zoom, scroll, playhead, J/K/L shuttle, frame step
- [ ] Track mute, solo, lock and height presets
- [ ] Smooth with 1000+ clips

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 8e, and section 8.3. read the ARCHITECTURE.md sections steps 8a to 8d added. you are doing step 8e of vellum.

1. a timeline panel drawing tracks and clips with filmstrips and waveforms. virtualise drawing so it stays smooth with 1000 or more clips.
2. drag clips, trim edges, razor, snapping to clips, the playhead and markers, all through 8a commands.
3. zoom and scroll, the playhead, j, k and l shuttle, frame step, home and end.
4. track headers with mute, solo, lock and height presets; add and remove tracks.

vitest for hit testing and snapping maths, playwright for drag, trim and razor. measure frame times with a 1000 clip project.
```

#### 8f: Sequence export and stress test

**Done when:**
- [ ] A sequence renders to a file through the media core
- [ ] A stress test project and recorded playback and export benchmarks
- [ ] Phase 8 checked in the real app; all phase 8 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 8f, and section 8.3. read the ARCHITECTURE.md sections steps 8a to 8e added. you are doing step 8f of vellum, the last step of phase 8.

1. a simple export: render a sequence (or its in to out range) to h264 or h265 through vellum-media as a job. the full export dialog and render queue come in phase 13.
2. a generated stress test project (many tracks, mixed frame rates, a nested sequence) and benchmarks for playback and export, recorded in ARCHITECTURE.md.
3. run the app, build a small edit by hand, play, scrub and export it. fix what you find, then tick the phase 8 boxes.
```

---

### Phase 9: Video editing tools

**Goal:** Premiere-level editing on top of the engine.

#### 9a: Edit tools

**Done when:**

- [ ] Ripple, roll, slip, slide, rate stretch, track select forward and back
- [ ] Insert, overwrite, replace, fit to fill, three and four point edits, lift and extract, ripple delete, close gaps
- [ ] Magnetic timeline option; markers with colours and notes

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 9a, and section 8.3. read the phase 8 sections of ARCHITECTURE.md. you are doing step 9a of vellum.

build every edit operation listed in step 9a as a timeline command with its tool and shortcut (premiere defaults). edge cases matter: ripple and roll at track ends, linked audio, locked tracks, gaps, nested sequences. write a test for every operation and its edge cases before wiring the ui.
```

#### 9b: Transitions and effects

**Done when:**

- [ ] Transitions library with handles on the timeline
- [ ] Effects library from section 8.3 as GPU nodes, drag onto one or many clips
- [ ] Copy and paste attributes, presets and favourites

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 9b, and section 8.3. read ARCHITECTURE.md section 24 (adding an effect) and the phase 8 sections. you are doing step 9b of vellum.

1. transitions (dissolves, dips, wipes, slides, zooms, whip pan, glitch, light leaks) as gpu nodes, dragged between clips, with length handles.
2. the effects library from section 8.3 as gpu nodes, reusing phase 6 nodes where they exist. drag onto one or many clips; an effect controls panel built from the node catalogue.
3. copy and paste attributes, effect presets and favourites.

golden frame tests for each new node.
```

#### 9c: Keyframe editor and speed

**Done when:**

- [ ] Keyframe curve editor, inline on clips and in its own panel
- [ ] Constant speed, speed ramps, reverse, freeze frame
- [ ] Optical flow slow motion (RIFE)

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 9c, and section 8.3. read the phase 8 sections of ARCHITECTURE.md. you are doing step 9c of vellum.

1. a keyframe curve editor with bezier handles and easing presets, shown inline on clips and in its own panel, over the 8a keyframe system.
2. speed: constant, ramps drawn as a curve, reverse, freeze frame, with audio pitch kept or not.
3. optical flow slow motion with rife through vellum-ai as a job, cached.

tests for time remapping maths (a ramp must land on exact frames).
```

#### 9d: Titles and templates

**Done when:**

- [ ] Title layers with all phase 6 text features
- [ ] Editable templates: lower thirds, end cards, captions

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 9d, and section 8.3. read the phase 6 and 8 sections of ARCHITECTURE.md. you are doing step 9d of vellum.

1. title clips that reuse the phase 6 text engine (fonts, paragraph, warp, styles), edited in the viewer.
2. a template system: lower thirds, end cards and caption styles with editable fields in a simple form, plus save your own.

golden frame tests for a few templates.
```

#### 9e: Auto subtitles and text based editing

**Done when:**

- [ ] Whisper transcription with word timing as a job
- [ ] Transcript panel, caption styles, translate, SRT and VTT export, burn in
- [ ] Deleting words in the transcript cuts the video

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 9e, and section 8.3. read the phase 7 and 8 sections of ARCHITECTURE.md. you are doing step 9e of vellum.

1. whisper through vellum-ai: transcribe a sequence or clip with word level timing, as a job.
2. a transcript panel: edit text, speakers, styles (word by word highlight, karaoke, classic), translate, export srt or vtt, or burn in as caption clips.
3. text based editing: selecting and deleting words in the transcript ripple deletes that media.

tests for word timing to frame mapping and the cut list.
```

#### 9f: Smart tools, part one

**Done when:**

- [ ] Scene cut detection
- [ ] Silence removal with a preview
- [ ] Beat detection markers and cut to the beat
- [ ] Auto reframe for vertical and square

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 9f, and section 8.3. read the phase 8 sections of ARCHITECTURE.md. you are doing step 9f of vellum.

build scene cut detection (cuts the clip or adds markers), silence removal with a threshold and a preview before it cuts, beat detection that adds markers and a cut to the beat command, and auto reframe that follows the subject to make vertical and square versions. each runs as a job. tests against small generated media.
```

#### 9g: Smart tools, part two

**Done when:**

- [ ] Multicam with audio sync
- [ ] Chroma key with spill suppression
- [ ] Stabiliser
- [ ] Point and planar tracking, face blur
- [ ] Phase 9 checked in the real app; all phase 9 boxes ticked

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 9g, and section 8.3. read the phase 8 sections of ARCHITECTURE.md. you are doing step 9g of vellum, the last step of phase 9.

build multicam (sync angles by audio, switch live while playing), chroma key with spill suppression as a gpu node, a stabiliser, point and planar tracking that text and effects can attach to, and automatic face blur that follows faces. then run the app, try every phase 9 tool once, fix what you find and tick the phase 9 boxes.
```

---

### Phase 10: Colour grading page

**Goal:** a Resolve-style colour page.

#### 10a: Grading nodes and primaries

**Done when:**

- [ ] Per clip node graph (serial, parallel, layer), grade versions, copy and apply to many
- [ ] Lift, gamma, gain, offset wheels, log wheels, contrast, pivot, saturation, temperature, tint
- [ ] All curve types

**Model:** Opus 5.5 · **Effort:** xhigh

```text
read PLAN.md section 5, "how a step runs" in section 9, step 10a, and section 8.3 (colour). read ARCHITECTURE.md section 24 (colour) and the phase 8 sections. you are doing step 10a of vellum.

1. a colour page with a per clip grading graph: serial, parallel and layer nodes, labels, enable and disable, grade versions, copy and paste grades, apply to many clips, and a timeline level grade.
2. primaries: lift, gamma, gain and offset wheels, log wheels, contrast, pivot, saturation, temperature and tint.
3. curves: custom rgb, hue vs hue, hue vs sat, hue vs lum, lum vs sat, sat vs sat.

the maths must match reference values; write those tests first.
```

#### 10b: Secondaries

**Done when:**

- [ ] HSL qualifier with matte view and refine
- [ ] Power windows (circle, linear, polygon, bezier) with tracking
- [ ] Colour warper

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 10b, and section 8.3 (colour). read the step 10a section of ARCHITECTURE.md. you are doing step 10b of vellum.

build the hsl qualifier with a matte view and refine controls, power windows (circle, linear, polygon, bezier) drawn in the viewer with softness and tracking (reuse 9g tracking), and the colour warper. all as nodes in the 10a graph. tests for qualifier and window mask maths.
```

#### 10c: Scopes

**Done when:**

- [ ] Waveform, RGB parade, vectorscope, histogram and CIE on the GPU in real time
- [ ] False colour and zebras in the viewer

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 10c, and section 8.3 (colour). read ARCHITECTURE.md section 24. you are doing step 10c of vellum.

compute waveform, rgb parade, vectorscope, histogram and cie chromaticity on the gpu in real time at 4k, draw them in dockable panels, and add false colour and zebras to the viewer. tests comparing gpu scope output with a cpu reference on test patterns.
```

#### 10d: LUTs, colour management, HDR and shot match

**Done when:**

- [ ] LUT import and export (.cube), LUT nodes
- [ ] Input transforms per clip, timeline space, output transform, HDR PQ and HLG preview and export
- [ ] Shot match, auto colour, gallery stills with wipe compare
- [ ] Grading tools usable as an image adjustment layer; all phase 10 boxes ticked

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 10d, and section 8.3 (colour). read ARCHITECTURE.md section 24 and the phase 10 sections. you are doing step 10d of vellum, the last step of phase 10.

1. luts: import and export .cube, apply as nodes.
2. a colour managed workflow: input transform per clip, timeline space, output transform (rec.709, rec.2020, p3, hdr pq and hlg). hdr sources must preview and export correctly.
3. shot match between two clips, auto colour, gallery stills with split and wipe compare.
4. the grade as an adjustment layer in the image editor.
5. run the app, grade a short edit, export it, fix what you find and tick the phase 10 boxes.
```

---

### Phase 11: Audio editor and timeline audio

**Goal:** Audacity/Audition-level audio editing plus a proper mixer in the video editor.

#### 11a: Audio engine

**Done when:**

- [ ] cpal playback and recording, device choice, buffer size
- [ ] Sample rate conversion (rubato), mic and system audio recording

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 11a, and section 8.4. read the step 8b section of ARCHITECTURE.md (the audio clock). you are doing step 11a of vellum in crates/vellum-audio.

build a real time audio engine with cpal: low latency playback and recording, device selection, buffer size setting, sample rate conversion with rubato, and system audio capture where the os allows. move the 8b playback clock onto it. tests for the mixing graph and resampling against reference output.
```

#### 11b: Audio workspace

**Done when:**

- [ ] Waveform and spectral views, single file and multitrack modes
- [ ] Selection editing: cut, copy, paste, trim, silence, fades, crossfades, envelope, zero crossing snap
- [ ] Spectral editing

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 11b, and section 8.4. read the step 11a section of ARCHITECTURE.md. you are doing step 11b of vellum in ui/src/workspaces/audio.

build the audio workspace: waveform and spectral views, single file and multitrack modes, selection editing (cut, copy, paste, trim, silence, fades, crossfades, envelope) with zero crossing snapping, and spectral editing (select or paint a region and turn it down). every edit is a command. tests for the edit maths.
```

#### 11c: Audio effects

**Done when:**

- [ ] Every effect in section 8.4 with live preview, presets and bypass
- [ ] Good pitch shift and time stretch

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 11c, and section 8.4. read the step 11a section of ARCHITECTURE.md. you are doing step 11c of vellum.

build every effect in section 8.4 with a live preview, presets and bypass. pitch shift and time stretch must use a proper algorithm (phase vocoder with phase locking, or a library that does it well), not resampling. tests comparing each effect with reference output.
```

#### 11d: Loudness and the timeline mixer

**Done when:**

- [ ] LUFS meters (momentary, short term, integrated), true peak, normalise presets
- [ ] Clip gain, keyframed volume, crossfades on the video timeline
- [ ] Mixer with pan, sends, buses, meters, auto ducking, voiceover with punch in

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 11d, and sections 8.3 (fairlight) and 8.4. read the phase 8 and 11 sections of ARCHITECTURE.md. you are doing step 11d of vellum.

1. loudness: lufs meters (momentary, short term, integrated), true peak, and normalise to presets (youtube, spotify, podcast, broadcast). check against the ebu r128 test signals.
2. timeline audio: clip gain, keyframed volume, crossfades, a mixer with pan, sends, buses and meters, auto ducking, and voiceover recording with punch in.
```

#### 11e: AI audio and extras

**Done when:**

- [ ] Noise removal, voice isolation, stem separation (Demucs), filler word removal
- [ ] Batch convert and normalise, tags and cover art, BPM and key, ringtone maker
- [ ] All phase 11 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 11e, and section 8.4. read the step 7a and phase 11 sections of ARCHITECTURE.md. you are doing step 11e of vellum, the last step of phase 11.

1. through vellum-ai, as jobs: noise removal, voice isolation, stem separation with demucs (vocals, drums, bass, other), and filler word removal ("um", "uh") using 9e's transcription.
2. extras: batch convert and normalise, a tag and cover art editor, bpm and key detection, a ringtone maker.
3. run the app, try each phase 11 feature once, fix what you find and tick the phase 11 boxes.
```

---

### Phase 12: Motion graphics and compositing

**Goal:** After Effects-lite and Fusion-lite.

#### 12a: Compositions

**Done when:**

- [ ] Composition documents, standalone or as a live linked timeline clip
- [ ] Solid, image, video, null, shape and text layers with parenting

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 12a, and section 8.5. read ARCHITECTURE.md section 24 and the phase 8 sections. you are doing step 12a of vellum.

build composition documents that render through the phase 4 graph and use the 8a keyframes: solid, image, video, null, shape and text layers, parenting, a comp viewer and layer stack. a comp can be dropped on the video timeline as a clip that stays live linked. golden frame tests.
```

#### 12b: Shape and text animation

**Done when:**

- [ ] Trim paths, repeaters, wiggle on shape layers
- [ ] Text animators with range selectors, and presets

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 12b, and section 8.5. read the step 12a section of ARCHITECTURE.md. you are doing step 12b of vellum.

add animated shape paths, trim paths, repeaters and wiggle, and text animators (per character position, scale, rotation, opacity, blur with range selectors) with presets (typewriter, bounce in, fade up, glitch). golden frame tests.
```

#### 12c: Graph editor and expressions

**Done when:**

- [ ] Graph editor with value and speed curves and easy ease
- [ ] Expressions lite (wiggle, loop, time maths) in a safe sandbox

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 12c, and section 8.5. read the phase 12 sections of ARCHITECTURE.md. you are doing step 12c of vellum.

build a graph editor (value and speed graphs, easy ease, handles) and expressions lite: wiggle, loop, time based maths, and links between properties, run in a sandbox with no file or network access and a time limit. tests for the expression evaluator.
```

#### 12d: Masks, mattes and AI roto

**Done when:**

- [ ] Layer masks and track mattes (alpha, luma, inverted)
- [ ] AI rotoscoping with SAM 2 across frames, with correction keyframes

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 12d, and section 8.5. read the step 7c and phase 12 sections of ARCHITECTURE.md. you are doing step 12d of vellum.

add masks and track mattes (alpha, luma, inverted), and ai rotoscoping: a sam 2 selection on one frame propagated across frames as a job, with correction keyframes the user can add. tests for matte maths.
```

#### 12e: Node view, particles and camera

**Done when:**

- [ ] Node view over the same graph
- [ ] Simple particle emitter
- [ ] Lights and a 2.5D camera with depth of field

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 12e, and section 8.5. read ARCHITECTURE.md section 24 and the phase 12 sections. you are doing step 12e of vellum.

add a node view that shows and edits the same composition graph, a simple particle emitter, lights, and a 2.5d camera with depth of field. golden frame tests.
```

#### 12f: Lottie and templates

**Done when:**

- [ ] Lottie import and export
- [ ] Templates with editable fields used on the video timeline
- [ ] All phase 12 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 12f, and section 8.5. read the phase 12 sections of ARCHITECTURE.md. you are doing step 12f of vellum, the last step of phase 12.

add lottie import and export for the features lottie supports (say clearly what is dropped), and a template system: mark parameters as editable so templates show a simple form on the video timeline. then run the app, build one animated title, use it as a template, fix what you find and tick the phase 12 boxes.
```

---

### Phase 13: Export, render queue and share targets

**Goal:** one great export experience for every workspace.

#### 13a: Export dialog and presets

**Done when:**

- [ ] One export dialog for every workspace with a live size and time estimate and Export to X MB built in
- [ ] Platform presets, saved, edited, imported and exported

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 13a, and sections 8.1 and 8.3 (delivery). read ARCHITECTURE.md section 23. you are doing step 13a of vellum in crates/vellum-export.

1. one export dialog used by the image, video, audio, motion and vector workspaces, with format options, a live estimate of size and time, and the phase 3 export to x mb engine as a toggle.
2. presets: youtube (several sizes), tiktok, reels and shorts, discord, x, instagram feed and story, prores, dnxhr, lossless, image sequence, audio only. look up current upload specs. users can save, edit, import and export presets.

tests for presets.
```

#### 13b: Render queue and after export

**Done when:**

- [ ] Background render queue, smart render, hardware encoding speed and quality slider
- [ ] Ranges, each clip, markers as stills, several presets from one render
- [ ] Copy as file, show in folder, notify, shut down when done; naming templates

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 13b, and section 8.3 (delivery). read the step 13a section of ARCHITECTURE.md. you are doing step 13b of vellum, the last step of phase 13.

1. a render queue: renders in the background while you edit, smart render (copy unchanged gop segments), hardware encoding with a speed and quality slider, reordering, shut down when done.
2. variants: in to out, every selected clip, every marker as a still, the current frame, several presets in one go.
3. after export: copy as file, show in folder, open, a notification with a thumbnail. file naming templates with tokens, never overwriting by default.
4. tick the phase 13 boxes.

tests for naming and variants.
```

---

### Phase 14: Capture tools

**Goal:** ShareX/OBS-lite built in.

#### 14a: Screenshots, hotkeys and tray

**Done when:**

- [ ] Region (with magnifier), window, full screen, all screens, scrolling and delayed screenshots
- [ ] Instant annotate overlay, then copy, save or open in the editor
- [ ] Global hotkeys (conflict checked) and a tray menu

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 14a, and section 8.7. you are doing step 14a of vellum in crates/vellum-capture.

build screenshots (region with a magnifier and pixel coordinates, window, full screen, all screens, scrolling, delayed), an annotate overlay (arrows, boxes, highlight, blur and pixelate, numbered steps, text, crop) and copy, save or open in the image editor. add customisable global hotkeys with conflict checks and a tray menu. explain macos permission prompts clearly.
```

#### 14b: Screen recorder

**Done when:**

- [ ] Region, window or screen with system audio and mic on separate tracks
- [ ] Webcam overlay, cursor highlight, click ripples, keystrokes, pause and resume
- [ ] Native capture per OS with hardware encoding

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 14b, and section 8.7. read the step 14a section of ARCHITECTURE.md. you are doing step 14b of vellum.

build screen recording with the best native api per os (windows graphics capture, screencapturekit, pipewire) and hardware encoding: region, window or screen, system audio and mic on separate tracks, webcam overlay, cursor highlight, click ripples, keystroke display, pause and resume. note which parts you could only test on one os.
```

#### 14c: Replay buffer and screen tools

**Done when:**

- [ ] Instant replay buffer with a hotkey, then Export to 10 MB
- [ ] Record to GIF, screen colour picker, on screen ruler, screen OCR
- [ ] All phase 14 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 14c, and section 8.7. read the phase 14 sections of ARCHITECTURE.md. you are doing step 14c of vellum, the last step of phase 14.

build the instant replay buffer (last n seconds in memory or a ring file, a hotkey saves it, a toast offers export to 10 mb and open in editor), record straight to gif, a screen colour picker, an on screen ruler and ocr of any screen region. tick the phase 14 boxes.
```

---

### Phase 15: Vector, design and PDF tools

**Goal:** Illustrator/Figma-lite plus everyday document tools.

#### 15a: Vector workspace

**Done when:**

- [ ] Pen, pencil, shapes, booleans, gradients, stroke profiles, align and distribute, layers, artboards
- [ ] SVG import and clean SVG export

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 15a, and section 8.6. read the phase 6 sections of ARCHITECTURE.md (paths and shapes). you are doing step 15a of vellum in crates/vellum-vector.

build a vector workspace that reuses the phase 6 path code: pen, pencil, shapes, booleans, gradients, stroke profiles, align and distribute, layers and artboards, svg import and a clean, minimal svg export. tests for svg round trips.
```

#### 15b: Trace, templates and mockups

**Done when:**

- [ ] Image trace with presets
- [ ] Template artboards (thumbnails, banners, overlays, posts, avatars)
- [ ] Mockup generator; palette extractor, gradient generator, icon and favicon packs

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 15b, and section 8.6. read the step 15a section of ARCHITECTURE.md. you are doing step 15b of vellum.

build image trace (bitmap to vector) with presets, template artboards (youtube thumbnail, discord banner and avatar, twitch overlays, instagram posts and stories, profile pictures), a mockup generator (phone, laptop, t-shirt with perspective), a palette extractor, a gradient generator, and icon and favicon pack export. each as a quick tool card and in the palette.
```

#### 15c: PDF tools

**Done when:**

- [ ] Merge, split, reorder pages with thumbnails, compress
- [ ] PDF to images, images to PDF, sign, annotate

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 15c, and section 8.8. you are doing step 15c of vellum.

build pdf tools: merge, split, reorder pages with thumbnails, compress, pdf to images, images to pdf, add a signature, annotate. pick a pdf library with a licence that can ship and record it. as quick tool cards and in the palette. tests on small generated pdfs.
```

#### 15d: Misc tools

**Done when:**

- [ ] QR generator and reader, collage and contact sheet, meme maker, sprite sheets, before and after export
- [ ] 3D model viewer with a turntable render
- [ ] All phase 15 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** medium

```text
read PLAN.md section 5, "how a step runs" in section 9, step 15d, and section 8.8. you are doing step 15d of vellum, the last step of phase 15.

build a qr generator and reader, collage and contact sheet maker, meme maker, sprite sheet maker, before and after comparison export, and a 3d model viewer (glb, obj, fbx) with a turntable render to video or gif. each as a quick tool card and in the palette. tick the phase 15 boxes.
```

---

### Phase 16: Automation, batch, CLI and plugins

**Goal:** power-user superpowers.

#### 16a: Macros, recipes and watch folders

**Done when:**

- [ ] Macro recorder and editor with parameters
- [ ] Batch recipe builder, saved and shared as files
- [ ] Watch folders with a log

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 16a, and section 8.10. read ARCHITECTURE.md (the command system and jobs). you are doing step 16a of vellum in crates/vellum-automation.

build a macro recorder over vellum-core commands (record, edit steps, parameterise values, replay on a file or a batch), a batch recipe builder (a visual list of steps saved as shareable files), and watch folders that run a recipe on new files with a log. tests for record and replay.
```

#### 16b: CLI and context menus

**Done when:**

- [ ] `vellum` CLI for every quick tool and for recipes
- [ ] Explorer and Finder context menu entries

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 16b, and section 8.10. read the step 16a section of ARCHITECTURE.md. you are doing step 16b of vellum.

build a vellum cli sharing the app's engine: a subcommand for every quick tool and for running recipes (for example `vellum compress clip.mp4 --size 10mb`, `vellum gif clip.mp4 --start 3 --end 8`), with progress output and exit codes. add explorer and finder context menu entries for the main quick tools (phase 3 left this here). tests for the cli.
```

#### 16c: Plugin API

**Done when:**

- [ ] Plugins in TypeScript or JavaScript (sandboxed) or WASM, with permissions
- [ ] Plugins can add tools, effects, importers, exporters and panels
- [ ] Docs and three example plugins

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 16c, and section 8.10. read ARCHITECTURE.md (commands, events, the render engine). you are doing step 16c of vellum.

design and build a plugin api: plugins in typescript or javascript (sandboxed) or wasm that can add tools, effects, importers, exporters and panels, with a permission model the user approves. write the design reasons in ARCHITECTURE.md, docs for plugin authors, and three example plugins. tests for the sandbox and permissions.
```

#### 16d: Asset library

**Done when:**

- [ ] Index of media, fonts, LUTs, presets, brushes, templates and sounds, with tags, collections and ratings
- [ ] Local AI tagging and a duplicate finder
- [ ] All phase 16 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md section 5, "how a step runs" in section 9, step 16d, and section 8.9. read the step 7a section of ARCHITECTURE.md. you are doing step 16d of vellum, the last step of phase 16.

build the asset library: an index of media, fonts, luts, presets, brushes, templates and sounds with tags, collections and ratings; local ai tagging (a small clip style model) so searching "dog" finds dog photos; and a visually similar duplicate finder. tick the phase 16 boxes.
```

---

### Phase 17: Polish, onboarding, accessibility and performance

**Goal:** turn a big app into a lovely app.

#### 17a: Onboarding and help

**Done when:**

- [ ] First run: pick what you are coming from, which sets shortcuts and layouts; a skippable tour
- [ ] Rich tooltips with short demo clips, "What can I do with this?", help in the palette

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md sections 5, 6 and 7, and "how a step runs" in section 9, step 17a. you are doing step 17a of vellum.

build first run onboarding (coming from photoshop, photopea, premiere, resolve, audacity or nothing, which sets shortcut presets and layouts) with a skippable 60 second tour, rich tooltips with short looping demo clips for complex tools, a "what can i do with this?" button that suggests tools for the current file, and searchable help in the command palette.
```

#### 17b: Accessibility

**Done when:**

- [ ] Full keyboard navigation and focus rings
- [ ] Screen reader labels, contrast checks for every theme, reduced motion respected

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md sections 5 and 6, and "how a step runs" in section 9, step 17b. you are doing step 17b of vellum.

go through every workspace: full keyboard navigation, visible focus rings, screen reader labels, contrast checks for every theme, reduced motion respected everywhere. add axe checks to the playwright tests. list anything you could not fix.
```

#### 17c: Performance

**Done when:**

- [ ] Startup under 2 seconds, idle CPU and memory audited
- [ ] No panel animation jank, large projects load fast; before and after numbers recorded

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md section 5 and "how a step runs" in section 9, step 17c. read ARCHITECTURE.md (benchmarks). you are doing step 17c of vellum.

measure, then fix: startup time (target under 2 seconds), idle cpu and memory, panel animation jank, and load times for big projects in each workspace. record before and after numbers in ARCHITECTURE.md.
```

#### 17d: Copy and consistency review

**Done when:**

- [ ] Every button, tooltip, dialog, empty state and error reviewed
- [ ] Shortcuts, icons, spacing and naming consistent across workspaces
- [ ] All phase 17 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** medium

```text
read PLAN.md sections 5 and 6, and "how a step runs" in section 9, step 17d. you are doing step 17d of vellum, the last step of phase 17.

review every button, tooltip, dialog, empty state and error message: plain words, consistent terms, errors that say what to do next, no em dashes. then look across workspaces for inconsistent shortcuts, icons, spacing and naming and fix them. list anything you could not fix, and tick the phase 17 boxes.
```

---

### Phase 18: Packaging and release

**Goal:** installers, auto-update, crash reporting (opt-in), licences, website-ready README.

#### 18a: Installers and release workflow

**Done when:**

- [ ] Windows MSI and NSIS, macOS DMG (signing and notarisation documented), Linux AppImage, deb and Flatpak
- [ ] File associations for media and `.vellum`
- [ ] GitHub Actions release workflow on a version tag

**Model:** Sonnet 5.5 · **Effort:** medium

```text
read PLAN.md section 5 and "how a step runs" in section 9, step 18a. you are doing step 18a of vellum.

set up installers (windows msi and nsis, macos dmg with signing and notarisation steps documented, linux appimage, deb and flatpak), file associations for common media types and .vellum projects, and a github actions workflow that builds and uploads all installers when a version tag is pushed.
```

#### 18b: Updates, crash reports, licences and README

**Done when:**

- [ ] Auto update with release notes and roll back one version
- [ ] Opt in crash reports, nothing sent by default
- [ ] `THIRD_PARTY_LICENSES.md` complete (FFmpeg LGPL, AI models); README with features and install steps
- [ ] All phase 18 boxes ticked

**Model:** Sonnet 5.5 · **Effort:** medium

```text
read PLAN.md section 5 and "how a step runs" in section 9, step 18b. you are doing step 18b of vellum, the last step of the plan.

add auto update with the tauri updater (release notes in the app, roll back one version), opt in crash reporting with clear privacy text and nothing sent by default, finish THIRD_PARTY_LICENSES.md (ffmpeg lgpl compliance, every dependency and ai model, flag anything that cannot ship), and write the README (features, screenshot placeholders, install and build steps). tick the phase 18 boxes.
```

---

## 10. Reusable prompts

### Phase review prompt (run after every phase)

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md and ARCHITECTURE.md. phase [N] was just completed. review it as a strict senior engineer who did not write it.

check:
1. every "done when" box for phase [N] is truly done. run the app and the tests to confirm, do not trust the ticks.
2. bugs, race conditions, memory leaks, ui thread blocking, unhandled errors, missing undo support, missing job progress or cancel.
3. section 5 conventions: comment style (lowercase, space after the comment symbol), no em dashes, code that reads human written, clippy and eslint clean.
4. ux: does it follow section 6? anything confusing, slow or inconsistent with other workspaces?
5. anything that will cause pain in later phases.

fix everything you can, then give me a short list of what you fixed and anything that needs my decision.
```

### Bug fix prompt

**Model:** Sonnet 5.5 · **Effort:** medium (bump to Opus 5.5 high if it is not fixed in one go)

```text
read PLAN.md section 5 and ARCHITECTURE.md. bug: [describe what happened, what you expected, steps to reproduce, and attach any error text].

find the root cause (not just the symptom), fix it, add a test that would have caught it, and tell me in two sentences what was wrong.
```

### New feature prompt

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md and ARCHITECTURE.md. i want to add: [feature]. it belongs in [workspace].

first, tell me in a few bullets how you plan to build it and which existing systems it uses (commands, jobs, gpu nodes, export engine). wait for my ok. then build it with tests, add it to the command palette and the feature catalogue in PLAN.md, and follow section 5 exactly.
```

### Nightmare bug prompt (crashes, leaks, heisenbugs)

**Model:** Opus 5.5 · **Effort:** high (xhigh if high fails twice)

```text
read ARCHITECTURE.md. there is a hard bug: [describe it, include logs, how often it happens, which os and gpu].

investigate methodically: form hypotheses, add instrumentation, reproduce it reliably, then fix the root cause. explain the cause and the fix, add a regression test, and remove any temporary logging. follow section 5 of PLAN.md.
```

### Docs and changelog prompt

**Model:** Haiku 5.5 · **Effort:** low

```text
read CHANGELOG.md and the git log since the last release tag. write user friendly release notes grouped into new, improved and fixed. plain words, no em dashes, no hype.
```

---

## 11. Risks and things to watch

- **Scope.** This is enormous. After phase 3 you already have a genuinely useful app. Treat everything after that as a stream of releases, not one big launch.
- **GPU in a web UI.** Getting wgpu frames into Tauri smoothly is the trickiest technical bet. Phase 4 must prove it with benchmarks before you build on it. If it fails, the fallback is a native canvas window per workspace.
- **Licences.** FFmpeg must be an LGPL build (avoid GPL-only encoders like x264 in the bundled build, or ship GPL and open source the app). Check every AI model's licence; some popular background removal models are non-commercial only.
- **Platform upload limits change.** That is why all size presets are editable and looked up during phase 3 and 13.
- **Model sizes.** AI models can be hundreds of MB each. Download on first use, never bundle them in the installer.
- **Patents and codecs.** H.265 and AAC can have licensing implications for commercial distribution. Use OS-provided encoders where possible and keep AV1/Opus/VP9 as free options.
- **Session size.** Keep each step to one session. If a step keeps running out, split it in this file before trying again rather than turning the effort up.
