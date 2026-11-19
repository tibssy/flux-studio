# Changelog

All notable changes to Flux Studio. Alpha builds are test releases; expect
rough edges.

## Unreleased

- **PNG metadata**: every generation, upscale and adjusted copy now carries its
  prompt, model, style, seed, steps, CFG, sampler, size, references and app
  version in PNG text chunks, plus an A1111-compatible `parameters` block.
- **History search, filters and favorites**: search prompts, models and seeds
  with `Ctrl+F`, filter by text-to-image, edits, upscales or adjustments, and
  star entries; matching chains stay expanded.
- **Effects export/import**: save user effects to a JSON pack and merge another
  pack without overwriting existing effects.
- **File actions**: copy the displayed image, open its folder, or drag it out
  of the app as a real PNG.
- Extra-wide **4:1** and **3:1** aspect-ratio presets for LinkedIn and
  Facebook-style header images.
- File-picker references keep the prompt composer open and focused.
- **Drag and drop** images onto the canvas and paste clipboard images or copied
  image files with **Ctrl+V**; both add references and keep the composer open.

## 0.1.0-alpha.2 — 2026-11-19

Changes since `v0.1.0-alpha.1`.

### Models

- **Qwen Image 2.1 Turbo** preset: unified text-to-image and editing in one 7B
  model, 8 steps with the Turbo sigma schedule, Qwen3-VL-8B encoder and vision
  projector, 10 reference images, and a 32 px size grid. Typography is the
  draw; it is the slowest preset and peaks at ~10.5 GB of RAM. The weights use
  the **Qwen Research License** (non-commercial) and require in-app
  confirmation.
- **FLUX.2 klein 9B** preset: the larger 4-step distillation with an 8B text
  encoder. Non-commercial license, confirmation required.
- **Fibo 2.0 Edit Turbo** and **FLUX.1 schnell** presets.
- Up to **10 reference images** per edit, with a numbered thumbnail strip;
  `<image_1>`, `<image_2>`… map to the add order.
- User-saved **effects**: name an edit prompt and reuse it from the effects
  menu; effects are editable and removable.

### Generation queue

- A serial **job queue**: press Generate while a job runs to queue the next
  request. Each job snapshots its prompt, model, style, size, seed and
  references.
- A Queue section in the sidebar with running/waiting rows, step and elapsed
  time, per-row cancel/remove, and clear-waiting.
- The composer closes after a successful submit; **New** (`Ctrl+N`) starts with
  an empty prompt and **Edit** reopens it for the next request.
- Closing the window with jobs pending asks for confirmation.
- **Upscale joins the queue**: while watching a generation it upscales that
  result when it finishes; while browsing history it upscales the displayed
  image. Upscale rows have their own progress and cancel control.
- **Desktop notifications** when a job finishes in the background, with a
  Preferences toggle. Clicking a notification opens that result.
- A floating **queue pill** in the bottom-left when the sidebar is hidden shows
  the running job and waiting count and opens the sidebar.
- History rows used by a running or waiting job as a **reference or upscale
  source** hide their delete action until the job releases them.
- History rows, mouse-wheel navigation and the arrow buttons keep working while
  generations and upscales run.

### Preview and history

- **Live step previews** on the canvas (TAE, with a projection fallback),
  optional per-preset preview weights and a Preferences toggle.
- Browsing history during a generation **pins the canvas** to the selected
  image and hides the step bar; the eye button in the queue row or on the canvas
  returns to the live frame.
- Returning to a job before its first preview shows a **“Waiting for the first
  preview…”** page instead of the previously viewed image.
- **Grouped history chains** for edits, effects and upscales, collapsible,
  starting collapsed, with scroll preservation.
- **View-only zoom** (buttons, `Ctrl+=`/`Ctrl+-`/`Ctrl+0`, wheel, drag to pan)
  at genuine image-to-screen scale, with rounded corners and animated zoom.
- **Manual color adjustments** (brightness, contrast, saturation) with live
  preview; Save creates a new image in the history chain.
- **Optional clipboard copy** for finished generations, upscales and adjusted
  copies (`Preferences → General → Copy results to clipboard`, off by default).

### Fixes

- Preview frames no longer overwrite a history image selected during a
  generation.
- Deleting history entries works during jobs; locked sources stay protected.
- The action toolbar stays available on a real history image during an upscale.
- The seed field is only updated when the queue drains, so queued requests do
  not lose the seed typed for the next one.
- Zoom keeps rounded corners and does not flicker while panning.
- Reduced UI stalls when switching presets and snapping sizes on the Qwen 32 px
  grid.

### Notes

- Non-commercial weights: **Fibo 2.0 Edit Turbo** (CC BY-NC 4.0),
  **FLUX.2 klein 9B** (FLUX Non-Commercial License) and **Qwen Image 2.1
  Turbo** (Qwen Research License). The downloader requires an explicit
  confirmation for these.
- Requirements: GTK 4.16+, libadwaita 1.6+ and glibc 2.39+ (Debian 13,
  Ubuntu 24.04 or newer).
