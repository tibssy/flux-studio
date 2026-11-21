# Changelog

All notable changes to Imagiflume. Alpha builds are test releases; expect
rough edges.

## Unreleased

- **Renamed to Imagiflume**: the app ID is now
  `io.github.tibssy.Imagiflume` and the binary is `imagiflume`. Config,
  history and caches now live under `~/.config/imagiflume`,
  `~/.local/share/imagiflume` and `~/.cache/imagiflume`; the Flatpak data
  directory is `~/.var/app/io.github.tibssy.Imagiflume/`. Existing native
  installs can move the old directories, and the Flatpak gets a new data
  directory. PNGs written by older builds are still readable.
- **Flatpak**: a Flathub-ready manifest on the GNOME 51 runtime with AppStream
  metadata and screenshots. The CPU `sd-server` and `llama-cli` backends are
  built inside the runtime with AVX2/FMA and OpenMP, so the sandboxed app works
  without downloading binaries that need a newer glibc.
- **Flatpak prompt helper**: the bundled `llama-cli` is discovered next to the
  app, so **Improve** works in the sandbox without a separate download.
- **Startup model check**: restoring the last image no longer reports its
  installed model as missing.
- **Upscale dialog**: only installed upscaler weights are listed; additional
  ones can be downloaded from Preferences.
- **GPU detection in the Flatpak**: NVIDIA GPUs are found through NVML or
  sysfs when `nvidia-smi` is unavailable, so the detected-hardware summary
  includes the model, compute capability and the driver's CUDA version.
- **Backend requirements**: ROCm, CUDA and Vulkan entries now check the
  libraries named in the manifest before being offered, including toolkits
  installed under `/opt`.
- **Custom backend errors**: when a backend cannot start because a shared
  library is missing, the toast names that library instead of only pointing
  at the log.
- **Flatpak backends**: custom `sd-server`/`llama-cli` pickers are hidden in
  the sandbox, and a previously selected custom path is ignored and cleared.
- **Backend rows**: the custom-binary chooser is right-aligned when there is
  no activate button next to it.
- **LoRA validation**: a weight that cannot load is flagged in the **LoRAs…**
  dialog (with the reason, e.g. an incomplete download) and cannot be enabled.
  A job that would use a broken weight is stopped with a clear message instead
  of generating without it.
- **Failed LoRA warning**: if the backend rejects a weight at load time, the
  result now says so instead of silently looking like a LoRA with no effect.
- **LoRA support**: a **LoRAs…** dialog lists weights from `models/loras`,
  with per-weight strength and a sidebar summary; active LoRAs apply to
  generations and effects and are recorded in PNG metadata.
- **Composer toolbar**: image, camera, prompt-history and improve buttons sit
  in one row under the prompt. The field is cleared after each generation
  request.
- **Camera capture**: take a photo from the first V4L2 camera and use it as an
  edit reference; the button is hidden when no camera or `ffmpeg` is available.
- **Search toggle**: a search button in the History header shows or hides the
  search field (`Ctrl+F` still opens it).
- **History filter button**: the funnel icon in the History header opens a
  compact filter list for favorites, text-to-image, edits, upscales and
  adjustments.
- **Stable History scroll**: deleting an image no longer jumps the sidebar back
  to the top.
- **Bulk delete**: a **Select** button turns History rows into checkboxes with
  Select all, a selection count and one confirmation for the whole batch.
  Images used by queued jobs are skipped.
- **History chains read newest-first**: a collapsed chain shows its newest
  image and opens it when clicked; expanding reveals older iterations below,
  with the original image at the bottom marked **first**.
- **Open images from disk**: `Ctrl+O` (or the folder button next to **New**)
  imports a copy into the output folder and History, ready to edit, upscale or
  use as a reference.

- **Batch and variations**: queue 1–4 images per Generate with consecutive
  seeds, and re-run a result with seed + 1 from the new shuffle button.
- **Before/after compare**: split the canvas against the source of an edit,
  upscale or adjustment with a draggable divider.
- **Queue control**: pause/resume, reorder waiting jobs, duplicate a generation
  job, and an estimated time remaining on the running row.
- **Prompt history**: `Alt+Up` / `Alt+Down` recall submitted prompts without
  losing the current draft.
- **Session restore**: optionally reopen the last image and draft prompt on
  launch (**Preferences → General**, on by default).
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
