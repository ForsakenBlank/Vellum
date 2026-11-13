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
2. Work through the phases **in order**. Each phase has a goal, scope, a "done when" checklist, a recommended model and effort level, and a prompt.
3. Start a **fresh coding session** for each phase (Claude Code works best for this). Paste the phase prompt in. Each prompt tells the model to read `PLAN.md` first, so it always has the full picture.
4. When a phase finishes, run the **Phase review prompt** (section 10) before moving on. Fixing problems early is far cheaper than fixing them five phases later.
5. Tick off the "done when" boxes in this file as you go so every future session knows where things stand.

**Fable 5.2 tip:** you said you only have Fable for a while, so the plan front-loads it onto the hardest foundation work (architecture, GPU engine, image canvas, timeline engine, colour pipeline). Do those phases first while you have it. If access ends early, use **Opus 5.5 at max** for any remaining Fable phase.

---

## 2. Model and effort cheat sheet

| Phase | What | Model | Effort |
|---|---|---|---|
| 0 | Repo bootstrap and architecture | Fable 5.2 | high |
| 1 | App shell and UX foundations | Opus 5.5 | high |
| 2 | Media core (decode, probe, cache, job queue) | Opus 5.5 | xhigh |
| 3 | Quick Tools: convert, Export to X MB, video to GIF | Opus 5.5 | high |
| 4 | GPU render engine and colour management | Fable 5.2 | max |
| 5 | Image editor core (canvas, layers, selections, brushes) | Fable 5.2 | xhigh |
| 6 | Image editor tools (adjustments, filters, text, PSD) | Opus 5.5 | high |
| 7 | AI toolkit (background remover, inpaint, upscale) | Opus 5.5 | xhigh |
| 8 | Video timeline engine (playback, proxies, scrubbing) | Fable 5.2 | max |
| 9 | Video editing tools (trim, transitions, titles, keyframes) | Opus 5.5 | high |
| 10 | Colour grading page | Fable 5.2 | xhigh |
| 11 | Audio editor and timeline audio | Opus 5.5 | high |
| 12 | Motion graphics and compositing | Opus 5.5 | xhigh |
| 13 | Export / render queue / share targets | Sonnet 5.5 | high |
| 14 | Capture: screen recorder, screenshots, clipping | Sonnet 5.5 | high |
| 15 | Vector, design and PDF tools | Sonnet 5.5 | high |
| 16 | Automation: batch, macros, CLI, plugins | Opus 5.5 | high |
| 17 | Polish: onboarding, accessibility, performance | Sonnet 5.5 | high |
| 18 | Packaging, installer, auto-update, release | Sonnet 5.5 | medium |
| any | Bug fixing on a single feature | Sonnet 5.5 | medium |
| any | Docs, changelogs, README, small copy fixes | Haiku 5.5 | low |
| any | Weird crash / memory leak / race condition | Fable 5.2 (or Opus 5.5) | max |

**Rule of thumb:**
- **Fable 5.2** for anything where a wrong design decision is expensive later (engines, data models, threading, GPU).
- **Opus 5.5** for big features built on top of a solid core.
- **Sonnet 5.5** for UI-heavy features, wiring, polish, and most bug fixes.
- **Haiku 5.5** for docs and tiny edits.
- Turn effort **up** if a phase keeps failing tests; turn it **down** if it is just moving buttons around.

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
- [x] `pnpm tauri dev` opens a window on Windows, macOS and Linux
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
- [ ] Home screen with recent files, pinned projects and quick tool cards
- [ ] Drag and drop anywhere routes files to the right workspace (placeholder workspaces OK)
- [ ] Dockable panels with saved layouts, tabs for documents
- [ ] Command palette (`Ctrl+K`) searching commands, settings and recent files
- [ ] Shortcut editor with conflict detection and Photoshop/Premiere/Resolve presets
- [ ] Light/dark/high-contrast themes, UI scale, accent colour
- [ ] Autosave, crash recovery prompt, notification centre, job queue panel

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
- [ ] FFmpeg integration for decode and encode with hardware acceleration detection
- [ ] Image decoding for PNG, JPG, WebP, AVIF, HEIC, TIFF, GIF, RAW, SVG, PSD (flattened preview)
- [ ] Media probe returns a unified `MediaInfo` struct
- [ ] Thumbnail, filmstrip and waveform generation as cached jobs
- [ ] SQLite-backed cache with size limit and cleanup
- [ ] File info panel in the UI with "strip metadata"

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
- [ ] Export to X MB works for video, image, GIF and audio, default 10 MB, lands under the limit every time in tests
- [ ] Platform presets (editable), MB/MiB toggle, safety margin, strategy picker
- [ ] Video to GIF tool with trim, crop, fps, speed, boomerang, captions, and WebP/APNG/MP4 alternatives
- [ ] GIF frame editor and optimiser
- [ ] Universal converter with instant remux where possible
- [ ] Batch mode for all three, and "copy result as file" to clipboard

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
- [ ] wgpu renderer drawing into the Tauri window (native surface or shared texture) at 60+ fps
- [ ] Node graph: sources, effects, blend, mask, transform, output; cached and invalidated correctly
- [ ] All Photoshop blend modes in WGSL with golden tests
- [ ] Colour management: linear working space, sRGB/Rec.709/Rec.2020/P3/ACES transforms, ICC profiles, 16f and 32f pipelines
- [ ] Tiled rendering for huge images, CPU fallback path
- [ ] Benchmarks recorded in `ARCHITECTURE.md`

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
- [ ] Raster documents with layers, groups, masks, clipping masks, blend modes, opacity/fill
- [ ] Selection tools: marquee, lasso, polygon, magnetic lasso, magic wand, quick selection, colour range
- [ ] Free transform, warp, perspective, rotate, flip, canvas/image size, crop with straighten
- [ ] Brush engine with pressure/tilt, smoothing, eraser, gradient, bucket, eyedropper
- [ ] Undo/redo for everything, history panel with snapshots
- [ ] PSD import with layers; native `.vellum` save; PNG/JPG/WebP export
- [ ] Rulers, guides, smart guides, grid

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

### Phase 6: Image editor tools

**Goal:** fill in the long tail: adjustments, filters, retouching, text, shapes, layer styles, smart objects, PSD export.

**Done when:**
- [ ] All adjustment layers from section 8.2, plus a Camera RAW style panel
- [ ] Filter gallery with smart filters
- [ ] Clone, healing, spot healing, patch, dodge/burn/sponge, blur/sharpen/smudge, liquify
- [ ] Text tool with paragraph text, warp, Google Fonts browser
- [ ] Shapes, pen tool, vector masks
- [ ] Layer styles, smart objects, PSD export

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md (section 8.2) and ARCHITECTURE.md first. phase 5 built the image editor core. you are building phase 6: the rest of the photoshop/photopea toolset. reuse the gpu effect node pattern from phase 4 for every adjustment and filter.

1. every adjustment layer listed in section 8.2, non destructive, with live preview and presets, plus a camera raw style panel (white balance, tone, texture, clarity, dehaze, hsl, colour grading wheels, detail, lens corrections, vignette, grain). auto tone, auto contrast, auto colour and one click enhance.
2. filters from section 8.2 with a filter gallery showing live thumbnails. smart objects (embedded and linked) with smart filters that stay editable and maskable.
3. retouching: clone stamp (with aligned and sample all layers), healing brush, spot healing, patch, content aware move, red eye, dodge, burn, sponge, blur, sharpen, smudge, liquify (forward warp, pucker, bloat, reconstruct, face aware sliders can wait for phase 7), puppet warp.
4. text: point and paragraph text, text on path, warp text, variable fonts, opentype features, a font browser with previews and google fonts download, and missing font handling.
5. shapes and paths: rectangle (live corners), ellipse, polygon, line, custom shapes, pen and curvature pen, path selection, booleans, vector masks.
6. layer styles: drop shadow, inner shadow, outer and inner glow, bevel and emboss, stroke, colour/gradient/pattern overlay, satin. copy and paste styles, style presets.
7. psd export that photoshop and photopea can open with layers intact. export layers to files, export artboards, save for web with side by side comparison.

golden image tests for every adjustment and filter. follow section 5 of PLAN.md exactly. tick the phase 6 boxes and update CHANGELOG.md.
```

---

### Phase 7: AI toolkit

**Goal:** local, offline AI tools, led by the background remover where **you choose what gets deleted**.

**Done when:**
- [ ] ONNX Runtime with GPU backends and a model manager (download on first use, progress, verify, delete)
- [ ] Background remover with keep/erase brushes, click-to-keep/remove (SAM 2), edge and hair refine, decontamination, background replacement
- [ ] Smart Select (click to add, Alt-click to subtract)
- [ ] Object remover (LaMa), upscale (Real-ESRGAN), face restore, colourise, depth blur, sky replace, OCR
- [ ] Batch background removal

**Model:** Opus 5.5 · **Effort:** xhigh

```text
read PLAN.md (sections 8.2 background remover and ai extras) and ARCHITECTURE.md first. you are building phase 7 of vellum: the ai toolkit in crates/vellum-ai, plus its ui in the image editor.

1. onnx runtime (ort crate) with directml on windows, cuda where available, coreml on mac, and cpu fallback. a model manager: models download on first use with a progress bar, checksum verification, a settings page listing installed models and their disk size, and delete buttons. verify each model's licence allows use in a distributed app and list them in THIRD_PARTY_LICENSES.md.
2. background remover (this is a headline feature, make the ux excellent):
   - one click auto removal (birefnet or u2net, pick the best quality permissive model).
   - result shown as an editable mask overlay. the user chooses what is deleted with: keep brush, erase brush, and click to keep / click to remove objects using sam 2 (clicks become prompts, live preview while hovering).
   - edge softness slider, hair and fur refine, colour decontamination.
   - new background options: transparent, solid colour, blurred original, replacement image. shadow options: keep, add drop shadow, remove.
   - always outputs a layer mask so it stays editable. batch mode for many product photos.
3. smart select tool using sam 2: hover highlights objects, click adds, alt click subtracts, outputs a normal selection.
4. object remover using lama: paint over something, it disappears. works on the selection too.
5. upscale 2x and 4x (real-esrgan), face restore, denoise, colourise, depth map with fake portrait blur, sky replacement, ocr to copy text from an image.
6. every ai tool runs as a job with progress and cancel, never freezes the ui, and shows a clear message if the gpu runs out of memory (then retries on cpu or in tiles).

measure speed on gpu and cpu for a 12 megapixel image and record results. follow section 5 of PLAN.md exactly. tick the phase 7 boxes and update CHANGELOG.md.
```

---

### Phase 8: Video timeline engine

**Goal:** the beating heart of the video editor: data model, smooth playback, scrubbing, proxies, frame cache, audio sync.

**Done when:**
- [ ] NLE data model: sequences, tracks, clips, nested sequences, transitions, effects, keyframes
- [ ] Real-time playback of multi-track 4K with GPU compositing
- [ ] Instant scrubbing with frame cache and automatic proxies
- [ ] Audio/video sync with drift correction, variable frame rate handled
- [ ] Media pool with bins and source monitor
- [ ] Basic timeline UI: drag clips, trim edges, razor, snapping, zoom

**Model:** Fable 5.2 · **Effort:** max

```text
read PLAN.md (section 8.3) and ARCHITECTURE.md first, including how the phase 4 gpu engine and phase 2 media core work. you are building phase 8 of vellum: the video timeline engine in crates/vellum-timeline. this must be rock solid, everything in phases 9 to 13 builds on it.

1. data model: projects, sequences (resolution, fps, colour space), unlimited video and audio tracks, clips (source range, timeline range, speed), nested sequences, adjustment layers, transitions, effects stacks, and a generic keyframe system (with bezier easing) that any parameter can use. everything goes through vellum-core commands so it is undoable.
2. playback scheduler: decode ahead on background threads, composite through the gpu node graph, present at the right time locked to the audio clock. handle mixed frame rates, variable frame rate sources and drift correction. real time multi track 4k on a decent machine; drop frames gracefully on weak ones and show a dropped frame indicator.
3. scrubbing: frame cache with memory budget, instant response while dragging the playhead, audio scrub. automatic proxy generation for heavy codecs (h265, raw, long gop 4k+) as background jobs, switched transparently.
4. media pool with bins, smart bins, metadata columns, hover scrub thumbnails, and a source monitor with in/out points and markers.
5. timeline ui: clips with filmstrip and waveform, drag, trim edges, razor, snapping, zoom and scroll, playhead, j/k/l shuttle, frame step, track mute/solo/lock, track height presets. keep it smooth with 1000+ clips (virtualise rendering).
6. a simple export path through the media core so a sequence can be rendered to a file (the full render queue comes in phase 13).

write tests for timing maths (frame and sample accuracy), and a stress test project. record playback benchmarks in ARCHITECTURE.md. follow section 5 of PLAN.md exactly. tick the phase 8 boxes and update CHANGELOG.md.
```

---

### Phase 9: Video editing tools

**Goal:** Premiere-level editing on top of the engine.

**Done when:**
- [ ] Ripple, roll, slip, slide, rate stretch, insert/overwrite, 3 and 4 point edits, lift/extract
- [ ] Transitions with timeline handles, effects library, keyframe curve editor
- [ ] Speed ramps, reverse, freeze frame, optical-flow slow motion
- [ ] Titles and templates, auto subtitles with styling, text-based editing
- [ ] Scene detection, silence removal, beat markers, auto-reframe, multicam, chroma key, stabiliser, tracking, face blur

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md (section 8.3) and ARCHITECTURE.md first. the timeline engine exists from phase 8. you are building phase 9 of vellum: the full editing toolset.

1. edit tools: selection, ripple, roll, slip, slide, razor, track select forward and back, rate stretch. insert, overwrite, replace, fit to fill, three and four point editing, lift and extract, ripple delete, close all gaps. magnetic timeline mode as an option. markers with colours and notes.
2. transitions library (dissolves, dips, wipes, slides, zooms, whip pan, glitch, light leaks) with drag handles on the timeline. effects library (section 8.3) as gpu nodes, drag onto multiple clips, copy/paste attributes, presets and favourites.
3. keyframe curve editor with bezier handles and easing presets, shown both inline on clips and in a dedicated panel.
4. speed: constant, speed ramps with a curve, reverse, freeze frame, optical flow slow motion using rife via vellum-ai.
5. titles: editable templates (lower thirds, end cards, captions), text with all phase 6 text features.
6. auto subtitles with whisper: transcript panel, edit text, word level timing, styles (word by word highlight, karaoke, classic), translate, export srt/vtt or burn in. text based editing: deleting words in the transcript cuts the video.
7. smart tools: scene cut detection, silence removal with preview, beat detection markers and cut to beat, auto reframe for vertical/square versions, multicam with audio sync, chroma key with spill suppression, stabiliser, point and planar tracking (attach text/effects to tracked objects), automatic face blur.

tests for every edit operation (especially ripple and roll edge cases). follow section 5 of PLAN.md exactly. tick the phase 9 boxes and update CHANGELOG.md.
```

---

### Phase 10: Colour grading page

**Goal:** a Resolve-style colour page.

**Done when:**
- [ ] Node-based grading (serial, parallel, layer) per clip, grade versions, copy grade
- [ ] Wheels, log wheels, all curve types, HSL qualifier, power windows with tracking
- [ ] Scopes: waveform, parade, vectorscope, histogram, CIE
- [ ] LUT import/export, colour-managed workflow, HDR support
- [ ] Shot match, gallery stills with wipe compare

**Model:** Fable 5.2 · **Effort:** xhigh

```text
read PLAN.md (section 8.3, colour) and ARCHITECTURE.md first. you are building phase 10 of vellum: a davinci resolve style colour page, using the phase 4 gpu engine and the phase 8 timeline.

1. per clip node graph for grading: serial, parallel and layer nodes, node labels, enable/disable, grade versions, copy and paste grades, apply grade to many clips, timeline level grade.
2. primaries: lift, gamma, gain, offset wheels, log wheels, contrast, pivot, saturation, temperature, tint. curves: custom rgb, hue vs hue, hue vs sat, hue vs lum, lum vs sat, sat vs sat.
3. secondaries: hsl qualifier with matte view and refine, power windows (circle, linear, polygon, bezier) with tracking, colour warper.
4. scopes computed on the gpu in real time: waveform, rgb parade, vectorscope, histogram, cie chromaticity, plus false colour and zebras in the viewer.
5. luts: import and export .cube, apply as nodes. colour managed workflow with input transforms per clip, timeline space, and output transform (rec.709, rec.2020, p3, hdr pq/hlg). make sure hdr sources preview and export correctly.
6. shot match between two clips, auto colour, gallery stills with split and wipe compare.
7. the same grading tools should be usable on a still image in the image editor as an adjustment layer.

maths must be accurate: write tests comparing against known reference values. keep everything real time at 4k on a decent gpu. follow section 5 of PLAN.md exactly. tick the phase 10 boxes and update CHANGELOG.md.
```

---

### Phase 11: Audio editor and timeline audio

**Goal:** Audacity/Audition-level audio editing plus a proper mixer in the video editor.

**Done when:**
- [ ] Standalone audio workspace with waveform and spectral views, spectral editing
- [ ] Effects list from section 8.4, real-time preview
- [ ] AI noise removal, voice isolation, stem separation, filler-word removal
- [ ] Timeline mixer with buses, sends, LUFS meters, auto ducking, loudness normalisation
- [ ] Recording (mic and system audio)

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md (section 8.4 and the fairlight part of 8.3) and ARCHITECTURE.md first. you are building phase 11 of vellum: audio, in crates/vellum-audio and ui/src/workspaces/audio.

1. real time audio engine with cpal: low latency playback and recording, device selection, sample rate conversion with rubato, buffer size setting.
2. audio workspace: waveform and spectral views, selection editing (cut, copy, paste, trim, silence, fades, crossfades, envelope), zero crossing snapping, spectral editing (paint/select and attenuate regions), single file and multitrack modes.
3. effects from section 8.4, each with a live preview, presets, and bypass. pitch shift and time stretch must sound good (use a proper algorithm, not naive resampling).
4. ai: noise removal, voice isolation, stem separation with demucs (vocals, drums, bass, other), transcription and filler word removal ("um", "uh"), all through vellum-ai as jobs.
5. loudness: lufs meters (momentary, short term, integrated), true peak, normalise to presets (youtube, spotify, podcast, broadcast).
6. video timeline audio: clip gain, keyframed volume, crossfades, a mixer with pan, sends, buses, meters, auto ducking, voiceover recording with punch in.
7. extras: batch convert and normalise, tag and cover art editor, bpm and key detection, ringtone maker.

tests for dsp correctness (compare against reference outputs) and sync with video. follow section 5 of PLAN.md exactly. tick the phase 11 boxes and update CHANGELOG.md.
```

---

### Phase 12: Motion graphics and compositing

**Goal:** After Effects-lite and Fusion-lite.

**Done when:**
- [ ] Composition mode with shape and text layers, graph editor
- [ ] Text animators and presets, shape animation (trim paths, repeaters, wiggle)
- [ ] Masks, track mattes, AI rotoscoping
- [ ] Node view, particles, 2.5D camera
- [ ] Lottie import/export, templates usable on the video timeline

**Model:** Opus 5.5 · **Effort:** xhigh

```text
read PLAN.md (section 8.5) and ARCHITECTURE.md first. you are building phase 12 of vellum: motion graphics and compositing, reusing the phase 4 render graph and the phase 8 keyframe system.

1. composition documents that can be used standalone or dropped onto the video timeline as a clip (live linked, editing updates the timeline).
2. layers: shape layers with animated paths, trim paths, repeaters, wiggle; text layers with animators (per character position, scale, rotation, opacity, blur, range selectors) and presets (typewriter, bounce in, fade up, glitch); solid, image, video and null layers with parenting.
3. graph editor showing value and speed curves, easy ease, and expressions lite (wiggle, loop, time based maths) in a safe sandbox.
4. masks, track mattes (alpha, luma, inverted), and ai rotoscoping: sam 2 selection propagated across frames with manual correction keyframes.
5. a node view for advanced compositing over the same graph, simple particle emitter, lights and a 2.5d camera with depth of field.
6. lottie import and export, and a template system: mark parameters as editable so templates show a simple form on the video timeline.

golden frame tests for animation output. follow section 5 of PLAN.md exactly. tick the phase 12 boxes and update CHANGELOG.md.
```

---

### Phase 13: Export, render queue and share targets

**Goal:** one great export experience for every workspace.

**Done when:**
- [ ] Unified export dialog with presets, live size estimate and Export to X MB built in
- [ ] Render queue with background rendering, smart render, hardware encoding
- [ ] Platform presets (YouTube, TikTok/Reels/Shorts, Discord, X, Instagram, ProRes, DNxHR, image sequence)
- [ ] Export ranges, individual clips, markers as stills
- [ ] Post-export actions (copy as file, open folder, notify, shut down)

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md (sections 8.1 and 8.3 delivery) and ARCHITECTURE.md first. you are building phase 13 of vellum: a single export system for every workspace in crates/vellum-export.

1. one export dialog component used by the image, video, audio, motion and vector workspaces, with format specific options, a live estimate of file size and render time, and the export to x mb engine from phase 3 built in as a toggle.
2. preset system: youtube (several resolutions), tiktok/reels/shorts vertical, discord (size targeted), x/twitter, instagram feed and story, prores master, dnxhr, lossless, image sequence, audio only. look up current recommended upload specs for each platform. users can save, edit, import and export presets.
3. render queue: background rendering while you keep editing, smart render (copy unchanged gop segments when possible), hardware encoding with a speed vs quality slider, queue reordering, and "shut down when done".
4. export variants: in/out range, every selected clip as its own file, every marker as a still, current frame as image, multiple presets at once from one render.
5. post export actions: copy as file to clipboard, show in folder, open in default app, notification with a thumbnail.
6. file naming templates with tokens and never overwrite by default.

tests for presets and naming. follow section 5 of PLAN.md exactly. tick the phase 13 boxes and update CHANGELOG.md.
```

---

### Phase 14: Capture tools

**Goal:** ShareX/OBS-lite built in.

**Done when:**
- [ ] Screenshots (region, window, full, scrolling, delayed) with instant annotation
- [ ] Screen recorder with system audio, mic, webcam overlay, cursor and click effects
- [ ] Instant replay buffer with hotkey save then one-click Export to 10 MB
- [ ] Record to GIF, screen colour picker, ruler, screen OCR
- [ ] Global hotkeys and tray menu

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md (section 8.7) and ARCHITECTURE.md first. you are building phase 14 of vellum: capture tools in crates/vellum-capture.

1. screenshots: region (with magnifier and pixel coordinates), window, full screen, all screens, scrolling capture, delayed capture. after capture, an instant annotate overlay: arrows, boxes, highlight, blur/pixelate, numbered steps, text, crop. then copy, save, or open in the image editor.
2. screen recording: region, window or screen, system audio and mic (with separate tracks), webcam overlay, cursor highlight, click ripples, keystroke display, pause/resume. use the best native capture api per os (windows graphics capture, screencapturekit, pipewire) and hardware encoding.
3. instant replay buffer: keeps the last n seconds in memory or a ring file, a global hotkey saves it, then a toast offers "export to 10 mb" and "open in editor".
4. record straight to gif, screen colour picker, on screen ruler, ocr any screen region (via vellum-ai).
5. global hotkeys (customisable, conflict checked) and a tray icon menu with all capture actions.

handle permissions prompts on macos gracefully with clear instructions. follow section 5 of PLAN.md exactly. tick the phase 14 boxes and update CHANGELOG.md.
```

---

### Phase 15: Vector, design and PDF tools

**Goal:** Illustrator/Figma-lite plus everyday document tools.

**Done when:**
- [ ] Vector workspace: pen, shapes, booleans, gradients, align, image trace, clean SVG export
- [ ] Template artboards (thumbnails, banners, overlays) and mockup generator
- [ ] PDF tools: merge, split, reorder, compress, convert, sign, annotate
- [ ] Misc: QR, collage, meme maker, sprite sheets, palette extractor, 3D viewer turntable

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md (sections 8.6 and 8.8) and ARCHITECTURE.md first. you are building phase 15 of vellum: vector, design and document tools.

1. vector workspace (crates/vellum-vector): pen, pencil, shapes, boolean operations, gradients, stroke profiles, align and distribute, layers, artboards. svg import and clean, minimal svg export. image trace (bitmap to vector) with presets.
2. design templates: artboard presets for youtube thumbnail, discord banner and avatar, twitch overlays, instagram posts and stories, profile pictures. a mockup generator that places a design onto phone, laptop and t-shirt mockups with perspective.
3. palette extractor from images, gradient generator, icon and favicon pack export.
4. pdf tools: merge, split, reorder pages with thumbnails, compress, pdf to images, images to pdf, add signature, annotate.
5. misc tools: qr generator and reader, collage and contact sheet maker, meme maker, sprite sheet maker, before/after comparison export, a 3d model viewer (glb, obj, fbx) with a turntable render to video or gif.

all of these should appear as quick tool cards and in the command palette. follow section 5 of PLAN.md exactly. tick the phase 15 boxes and update CHANGELOG.md.
```

---

### Phase 16: Automation, batch, CLI and plugins

**Goal:** power-user superpowers.

**Done when:**
- [ ] Macro recorder and editor
- [ ] Batch recipe builder with saved recipes
- [ ] Watch folders
- [ ] CLI covering quick tools and recipes
- [ ] Explorer/Finder context menu entries
- [ ] Plugin API with sandbox, example plugins, docs
- [ ] Asset library with AI tagging and duplicate finder

**Model:** Opus 5.5 · **Effort:** high

```text
read PLAN.md (sections 8.9 and 8.10) and ARCHITECTURE.md first. you are building phase 16 of vellum: automation and extensibility in crates/vellum-automation.

1. macro recorder: record any sequence of vellum-core commands, edit steps, parameterise values, replay on the current file or a batch.
2. batch recipe builder: a visual list of steps (resize, watermark, convert, remove background, compress to x mb, rename...) saved as recipes and shareable as files.
3. watch folders that run a recipe on new files, with a log.
4. cli: `vellum` command with subcommands for every quick tool and for running recipes (eg `vellum compress clip.mp4 --size 10mb`, `vellum gif clip.mp4 --start 3 --end 8`), sharing the same engine as the app.
5. explorer and finder context menu integration for the main quick tools (if phase 3 did not already).
6. plugin api: plugins written in typescript/javascript (sandboxed) or wasm, able to add tools, effects, importers, exporters and panels, with a permission model. write docs and three example plugins.
7. asset library: index of media, fonts, luts, presets, brushes, templates and sounds, tags, collections, ratings, local ai tagging so searching "dog" finds dog photos, and a visually similar duplicate finder.

tests for macros and the cli. follow section 5 of PLAN.md exactly. tick the phase 16 boxes and update CHANGELOG.md.
```

---

### Phase 17: Polish, onboarding, accessibility and performance

**Goal:** turn a big app into a lovely app.

**Done when:**
- [ ] First-run onboarding (pick your background: Photoshop/Premiere/Resolve/new) that sets shortcuts and layout
- [ ] Interactive tool tips with short demo clips, contextual "What can I do with this?"
- [ ] Accessibility pass (keyboard navigation, screen reader labels, contrast)
- [ ] Performance pass: startup under 2 s, memory audit, no UI jank
- [ ] UX copy review, empty states, error messages

**Model:** Sonnet 5.5 · **Effort:** high

```text
read PLAN.md (sections 5, 6 and 7) and ARCHITECTURE.md first. every major feature now exists. you are doing phase 17 of vellum: polish.

1. first run onboarding: ask what the user is coming from (photoshop, photopea, premiere, resolve, audacity, nothing) and set shortcut presets and layouts to match. a skippable 60 second tour.
2. help everywhere: rich tooltips with short looping demo clips for complex tools, a "what can i do with this?" button that suggests tools for the current file, searchable help in the command palette.
3. accessibility: full keyboard navigation, focus rings, screen reader labels, contrast checks for all themes, reduced motion respected.
4. performance: measure and fix startup time (target under 2 seconds), idle cpu and memory usage, panel animation jank, large project load times. record before and after numbers.
5. ux copy review of every button, tooltip, dialog, empty state and error. plain words, consistent terms, errors that say what to do next. no em dashes.
6. go through every workspace looking for inconsistencies (shortcuts, icon styles, spacing, naming) and fix them. list anything you could not fix.

follow section 5 of PLAN.md exactly. tick the phase 17 boxes and update CHANGELOG.md.
```

---

### Phase 18: Packaging and release

**Goal:** installers, auto-update, crash reporting (opt-in), licences, website-ready README.

**Done when:**
- [ ] Signed installers for Windows (MSI/NSIS), macOS (DMG, notarised), Linux (AppImage, deb, Flatpak)
- [ ] Auto-update with release notes and rollback
- [ ] Opt-in crash reports, no telemetry by default
- [ ] `THIRD_PARTY_LICENSES.md` complete (FFmpeg LGPL compliance, AI model licences)
- [ ] README with screenshots and feature list

**Model:** Sonnet 5.5 · **Effort:** medium

```text
read PLAN.md and ARCHITECTURE.md first. you are doing phase 18 of vellum: packaging and release.

1. installers: windows msi and nsis, macos dmg with signing and notarisation steps documented, linux appimage, deb and flatpak. file associations for common media types and .vellum projects.
2. auto update using the tauri updater with release notes shown in app and a way to roll back one version.
3. opt in crash reporting only, nothing sent by default, clear privacy text in settings.
4. licence compliance: check ffmpeg is an lgpl build linked correctly, list every dependency and ai model licence in THIRD_PARTY_LICENSES.md, flag anything that is not compatible with distribution.
5. a github actions release workflow that builds and uploads all installers on a version tag.
6. a README with a feature list, screenshots placeholders, install instructions and build instructions.

follow section 5 of PLAN.md exactly (no em dashes, comment style). tick the phase 18 boxes and update CHANGELOG.md.
```

---

## 10. Reusable prompts

### Phase review prompt (run after every phase)

**Model:** Opus 5.5 · **Effort:** high (use Fable 5.2 at high for phases 0, 4, 5, 8 and 10)

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

**Model:** Fable 5.2 (or Opus 5.5) · **Effort:** max

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
- **Fable access window.** Prioritise phases 0, 4, 5, 8 and 10 while you have it.
