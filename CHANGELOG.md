# Changelog

All notable changes to Vellum are listed here. Dates are in YYYY-MM-DD.

## Unreleased

### Phase 6, first part: the engine and document side of the image editor tools (2026-11-22)

Added (in the engine and documents; the panels, dialogs and tools in the window still need building, see PLAN.md phase 6)

- Every adjustment from the plan as a render node with a GPU kernel and a matching CPU path: brightness/contrast, levels, curves, exposure, vibrance, hue/saturation, colour balance, black and white, photo filter, channel mixer, colour lookup (.cube files and built in looks), invert, posterize, threshold, gradient map, selective colour, shadows/highlights, HDR toning, match colour and replace colour, plus a Camera Raw node (white balance, tone, texture, clarity, dehaze, HSL, colour grading, sharpening and noise reduction, lens corrections, vignette, grain).
- Adjustment layers with presets, and auto tone, auto contrast, auto colour, equalize and one click enhance worked out from the picture.
- The filter gallery's filters: motion, radial, lens, tilt-shift, iris, field, path, spin and surface blur, smart sharpen, high pass, reduce noise, add noise, film grain, wave, ripple, twirl, polar coordinates, displace, chromatic aberration, glitch, emboss, find edges, glowing edges, oil paint, mosaic, colour halftone, clouds, lens flare and lighting effects. Golden images for every adjustment and filter in tests/golden/effects.
- Fill layers (colour, gradient, pattern) and smart objects, embedded or linked, with smart filters that stay editable, can be hidden, faded, reordered and masked.
- Retouching: clone stamp (aligned, sample all layers), healing brush, spot healing, patch, content-aware fill and move, red eye, dodge, burn, sponge, blur, sharpen and smudge. Liquify (forward warp, pucker, bloat, reconstruct, smooth, twirl) and puppet warp, both previewed on the GPU.
- Text layers: point, paragraph and on a path, warp styles, OpenType features, variable font axes, system fonts and Google Fonts downloads, and missing fonts reported and replaceable.
- Shape layers (rectangle with live corners, ellipse, polygon and star, line with arrowheads, custom shapes, free paths) with live path operations, vector masks and saved paths. Text, shapes and smart objects stay editable through moves, free transform and canvas changes.
- Layer styles: drop shadow, inner shadow, outer and inner glow, bevel and emboss, satin, colour, gradient and pattern overlays and stroke, with presets and paste onto several layers.
- Saving Photoshop files (PSD and PSB, 8, 16 and 32 bit) with layers intact, checked with psd-tools and ImageMagick. Invert, posterize, threshold and levels go out (and come back in) as real adjustment layers.
- Artboards, export layers to files, export artboards, and save for web previews with the real encoded size.

Fixed

- Free transform on a text or shape layer did nothing, and smart objects could not be transformed.

### Phase 5, second half: transforms, crop, Photoshop files, export and view aids (2026-11-22)

Added

- Free transform (Ctrl+T) on the current layer, or only the selected pixels when something is selected. Drag the corners and edges to scale (Shift lets the shape go, Alt scales about the middle), outside the box to turn it (Shift in 15 degree steps), and move the pivot. Ctrl and a corner distorts, Ctrl and an edge skews, Ctrl Alt Shift and a corner adds perspective, or pick Scale, Rotate, Skew, Distort or Perspective in the options bar. Warp bends the picture with sixteen points. Type exact numbers for the position, width, height, angle and skew, flip it, and pick nearest neighbour, bilinear or bicubic. The picture follows live while you drag; Enter applies it as one undoable step and Esc cancels.
- Transform selection changes the selection outline itself.
- Flip and rotate: the whole canvas by a quarter or a half turn, or flipped either way, and the current layer flipped about its middle.
- Canvas size, with an anchor and a colour for the new area, and image size, with width and height in pixels or percent, kept in proportion, and a choice of resampling.
- The crop tool (C): drag the edges, turn outside the box to straighten, draw a line along something level with Straighten, pick a ratio (1:1, 4:3, 16:9 and more), and keep or delete what is cut away. Perspective crop (Shift+C) takes four free corners and makes them a rectangle. Crop to selection too.
- Photoshop files (PSD and PSB) open with their layers: groups, layer masks, clipping, every blend mode, opacity and fill, hidden layers, locks, colour labels, guides, the resolution and the colour profile, at 8, 16 or 32 bits per channel. Text, shape and smart object layers come in as the pixels Photoshop stored for them and say what they were.
- Export as PNG, JPEG, WebP or AVIF (Ctrl+Alt+Shift+W). 16 and 32 bit pictures stay 16 bit in PNG, the colour profile is kept in PNG, JPEG and WebP, and "Fit under a size" uses the same search as Export to X MB, with the same size chips. Exporting runs in the background and shows in the jobs panel.
- Rulers (Ctrl+R), guides dragged out of them and saved with the picture (dragging one back onto a ruler deletes it, and every change undoes), the grid (Ctrl+'), and smart guides: the move tool, marquees, free transform and crop snap to guides, the grid, the canvas edges and middle, and other layers' edges and middles, with a pink line showing what lined up. Hold Ctrl to place something without snapping. Show, hide, lock and clear guides from the View commands, and set the grid size and what snaps from the buttons under the picture.
- The Navigator panel shows the whole picture with the part in view outlined; drag in it to move around and use its slider to zoom.

Changed

- The commands that said "Not here yet" in the image editor now do their job.

- Painting is quicker to show: pictures now keep track of changes in 64 pixel blocks, so a brush stroke redraws about half as much of the screen (on a 4K picture without a graphics card, from about 14 to 8 ms fitted to the window and 22 to 11 ms at 100%).

Fixed

- Layer thumbnails and the navigator now update after painting inside a layer, not only when the layer grows.
- The image editor's dialogs (feather, colour range, new image and the new ones) had no space around their contents.
- The Info panel no longer says a `.vellum` file cannot be read.
- With the navigator and colour picker open, the layer list could be squeezed out of sight; it now always keeps room, and the navigator can be folded away.

Notes

- Found by running the app for real under a virtual display, which phase 5's first half had not been: everything listed under Fixed. Photoshop files in CMYK, Lab, bitmap or indexed colour still open flattened.

### Phase 5, first half: image editor core (2026-11-22)

Added

- The image editor. Pictures open in it with their layers ready to edit, and File, New image makes a blank one at 8, 16 or 32 bits per channel with a white, black, coloured or see through background.
- Layers and groups with every Photoshop blend mode, opacity and fill, layer masks, clipping masks, locks (see through pixels, painting, position, everything), colour labels and a search box that finds layers by name, colour or kind. Drag layers to reorder them or drop them into groups. Merge down, merge visible, flatten, duplicate, new layer via copy and via cut.
- Selections: rectangle and ellipse marquees, the lasso, polygon lasso and magnetic lasso, the magic wand (tolerance, contiguous, sample all layers), the quick selection brush and colour range. Shift adds, Alt takes away, both keep the overlap. Feather, expand, contract, border, smooth, invert, select all, deselect and reselect. Save selections and load them back. Quick mask shows the selection in red and lets brushes paint it. Marching ants stay crisp at any zoom.
- Painting: a brush and pencil with size, hardness, opacity, flow, spacing, smoothing, scatter and texture, pen pressure for size, opacity and flow, and pen tilt. An eraser, a paint bucket, gradients (linear, radial, angle, reflected and diamond) and an eyedropper that can average up to 101 by 101 pixels. [ and ] change the brush size, Shift with them the hardness. Shift click draws a straight line from the last stroke.
- Every change undoes and redoes, including each brush stroke as one step, and shows in the History panel with its snapshots.
- Save as a `.vellum` file, which keeps every layer, mask, saved selection and the colour profile. It is a zip of the project folder, so it is easy to look inside. Autosave and crash recovery keep image documents too, rewriting only the layers that changed.
- Photoshop's shortcuts work in the image editor out of the box: V, M, L, W, I, B, E, G, H, Z, Q, D, X, Ctrl+T, Ctrl+J, Ctrl+G, Ctrl+E, Ctrl+D, Ctrl+Shift+I and more.

Changed

- A workspace's own shortcut now takes over a global one inside that workspace without counting as a conflict, so Ctrl+E merges layers in the image editor and still exports to a size everywhere else.
- `.vellum` projects open in the image editor.
- The command palette lists every command when nothing is typed.

Notes

- Crop, free transform, canvas size, image size, layered PSD import, export to PNG, JPEG, WebP and AVIF, rulers, guides and the grid come in the second half of phase 5. Until then their shortcuts say so, and PSD files open flattened.
- Pictures open at 8 bits per channel for now.

### Phase 4: render engine (2026-11-21)

Added

- Pictures now open in the image workspace and are drawn by Vellum's own render engine, on the graphics card when there is a usable one. Scroll to zoom around the pointer, drag to move, hold R and drag (or Shift and scroll) to turn the view, and use the bar under the picture to zoom in steps, fit, show actual pixels, straighten, mirror the view or show the pixel grid from 800%. With the picture selected, Ctrl+0 fits, Ctrl+1 shows actual pixels and Ctrl+= and Ctrl+- zoom (Cmd on a Mac). Editing arrives in phase 5.
- Moving the view stays smooth however big the picture is: the window keeps what it has already been sent and only asks for the parts that come into view. Even very large pictures show a whole-picture preview straight away while the detail arrives.
- Colour management: photos with an embedded colour profile (PNG, JPEG, WebP and TIFF) show their true colours, and Settings, Graphics and display can use your monitor's colour profile.
- Settings, Graphics and display: turn the graphics card on or off, see which device is drawing and why, and set how much memory rendered results may keep.
- Behind the scenes, the engine every later editor builds on: a graph of effects that caches every result and only redoes what changed, all 27 Photoshop blend modes, Gaussian blur, levels, hue and saturation, sharpen, transforms with perspective, gradients, masks, sRGB, Rec.709, Rec.2020 (with HDR), Display P3, Adobe RGB and ACES colour spaces, 8, 16 and 32 bit pipelines, and pictures far bigger than a graphics card can hold.

Changed

- The image workspace header says it is viewing only until the editor arrives.

Notes

- If the graphics card stops responding, Vellum reconnects to it on its own and draws with the processor in the meantime. Without a usable graphics card everything still works, just slower, and Settings says why.
- The speed numbers in ARCHITECTURE.md were measured on a machine without a graphics card; the graphics card figures still need a run on real hardware.

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
