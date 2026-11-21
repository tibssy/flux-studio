# Imagiflume

*Where imagination flows into images.*

A GTK4 / libadwaita desktop app for **local image generation and editing**,
driven by [stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp).
No Python, no PyTorch, no cloud.

Ships with six presets — FLUX.2 klein 4B, FLUX.2 klein 9B, FLUX.3 Image klein,
Fibo 2.0 Edit Turbo, Qwen Image 2.1 Turbo and FLUX.1 schnell — and uses each
model's native reference-image support for editing, so text-to-image and
image-to-image share the same weights.

## Status

Milestone 3 (packaged desktop app):

- **hideable sidebar** with a Setup section and a History section
- **megapixel + aspect-ratio target** (0.01–2.00 MP, 14 landscape/portrait
  presets including 3:1 and 4:1 headers, 128–2048 px sides on a 16 px grid,
  32 px for Qwen) with a live
  aspect-ratio preview
  that overlays the current image's aspect when it differs from the target,
  plus a snapped-size readout
- **style presets** (cinematic, dynamic, 2D, 3D, photographic, anime) applied
  as prompt suffixes
- **model dropdown restricted to installed presets** and seed control in the
  sidebar
- focused composer: prompt, Generate/Cancel and a reference-image button
- **reference-image editing** (`-r`) with up to 10 reference images per model;
  numbered thumbnails above the canvas add, preview and remove references.
  Edits promote their result to the next reference, while text-to-image runs
  stay text-to-image. Loading a reference or crop adopts its aspect ratio at
  the current MP target
- **history** with details and delete actions per row; delete removes the PNG,
  thumbnail and database entry. Images derived from another result (edits,
  effects upscales) are grouped under their source with an indent and a
  collapsible chain counter; chains start collapsed and read newest-first: the
  newest image is the group header, older iterations expand below it, and the
  original sits at the bottom marked **first**. The **Select** button switches
  History to checkboxes for bulk deletion. The search button toggles the
  search field (`Ctrl+F`), which matches prompts, models and seeds; the filter
  button in the History header narrows the list to text-to-image, edits,
  upscales or adjustments; matching chains stay expanded, and a star marks
  **favorites**
  for quick access
- clicking a history image shows it on the canvas with the effects menu,
  upscale, edit and crop actions and restores its model, style, size and seed;
  the prompt picker loads its prompt when needed
- **job queue**: press Generate while a job is running to queue another one.
  Each request snapshots its prompt, model, size, seed and references. The
  composer closes when a job is submitted; use New (`Ctrl+N`, starts with an
  empty prompt) or Edit to compose and queue the next one while earlier jobs
  run. When the sidebar is hidden, a pill in the bottom-left corner shows the
  running job and waiting count; click it to open the sidebar. The sidebar lists waiting and
  running jobs with per-row remove/cancel, **move up/down**, **duplicate** for
  generations, and a **pause/resume** control; the running row shows an
  estimated time left. The progress bar shows how many are
  waiting, and closing the window asks before discarding the queue. Jobs run
  one at a time because the server holds a single model in VRAM. Browsing
  History while a job runs pins the canvas to that image and hides the step
  bar; the eye button in the queue row (or on the canvas) returns to the live
  preview. Upscale joins the same queue: while watching a generation it
  upscales that result when it finishes, while browsing History it upscales the
  image on screen. History rows used by a queued job hide their delete action,
  so a reference or upscale source cannot disappear from under it; other rows
  can still be deleted while jobs run. Browsing with the mouse wheel, the
  history rows or the arrow keys works during generations and upscales
- **live step previews** on the canvas: `sd-server` streams a TAE-decoded frame
  for each sampling step and the canvas crossfades between them, with a
  Preferences toggle and ~11 MB optional preview weights per model family
- **desktop notifications** when a job finishes while the window is in the
  background, with a Preferences toggle; clicking the notification opens that
  result in History
- **optional clipboard copy**: finished generations, upscales and adjusted
  copies can be placed on the clipboard automatically from
  **Preferences → General** (off by default)
- **model caching**: `sd-server` keeps the loaded model for 60 s of idle time
  and releases it immediately when you switch models
- **prompt improvement** through llama.cpp using the active text encoder,
  with undo
- **optional vision helper** (LFM2.5-VL-450M, ~332 MB) for automatic
  day/night detection inside the effects menu; without it, explicit daylight
  and night entries are used
- **image actions** over the canvas: an effects menu (colorize, recover,
  enhance, denoise, photoreal, replace background, day/night, expand),
  save-prompt-as-effect, adjust colors, upscale, edit, crop-to-reference,
  undo, back-to-view, copy the image, open its folder, and drag it out as a
  real file
- **self-describing PNGs**: prompt, model, style, seed, steps, CFG, sampler,
  size, references and app version are written into every result as PNG text
  chunks, plus an A1111-compatible `parameters` block for other tools
- **portable effects**: export user effects to a small JSON pack and import
  them on another machine; imports merge without overwriting and rename name
  collisions
- **LoRA support**: the **LoRAs…** button lists `.safetensors` (and `.ckpt`,
  `.gguf`, `.sft`) weights from `models/loras`, with per-weight strength and a
  live sidebar summary; active LoRAs apply to generations and effects and are
  recorded in PNG metadata
- **manual adjustments** with brightness, contrast and saturation sliders and
  a live canvas preview; saving writes a new PNG that joins the history chain
  without re-running the model
- **batch and variations**: **Batch** in the sidebar queues 1–4 images with
  consecutive seeds, and the shuffle button over the canvas re-runs the
  displayed prompt with seed + 1
- **before/after compare**: the split button over the canvas shows the source
  of an edit, upscale or adjustment beside the result; drag the divider or
  click a side to move it
- **prompt history**: the history button in the composer toolbar lists
  saved prompts, and `Alt+Up` / `Alt+Down` recall them while keeping the draft
  you were writing
- **camera capture**: the camera button in the composer grabs one frame from
  the first V4L2 device, adds it as a reference and shows it on the canvas;
  it only appears when a camera and `ffmpeg` are available
- **session restore**: optionally reopen the last image and draft prompt on
  launch (**Preferences → General**, on by default)
- **view-only zoom** with a bottom-right pill: zoom in/out, real-pixel scale
  readout, fit, drag-to-pan and wheel-to-pan while zoomed. It never modifies
  the image
- **image upscaling** with five permissive 4× models, 2×/4× output, model
  downloads and tile-size fallback for 8 GB GPUs
- **history navigation** with a crossfade: scroll over the image or use the
  up/down arrows to step through history and compare an upscale with its source;
  the sidebar highlights the image currently on the canvas
- **open images from disk** (`Ctrl+O` or the folder button next to **New**):
  imports a copy into the output folder and History, ready to edit, upscale or
  use as a reference
- the composer slides away during generation and returns via Edit/New/Back
- **preferences dialog**: output folder, CPU offload, models folder
- **About page** with repository/issue links plus Credits and Legal subpages with back navigation
- **model downloader** with resume, progress and per-preset install status,
  license confirmation for non-commercial weights, and a delete action that
  keeps files shared with other presets
- first run opens the Models page when the active preset is missing
- PKGBUILD, `.desktop` entry and scalable icon

## Models

| Preset | Files | Steps | Measured @768² | Peak VRAM |
|---|---|---|---|---|
| **FLUX.2 klein 4B** (default) | 5.4 GB | 4 | **78–85 s** | 4.7 GB |
| **FLUX.2 klein 9B** | 11.0 GB | 4 | **148–158 s** | 7.8 GB |
| **FLUX.3 Image klein** | 6.3 GB | 16 | **254 s** | 5.0 GB |
| **Fibo 2.0 Edit Turbo** | 9.2 GB | 4 | — | — |
| **Qwen Image 2.1 Turbo** | 10.7 GB | 8 | **192 s** | 6.5 GB |
| **FLUX.1 schnell** | 10.1 GB | 4 | — | — |

FLUX.3 is the quality option (~3× slower on Maxwell); klein 4B is the fastest
4-step model that fits this GPU. Klein 9B is the same 4-step distillation with
a larger transformer and an 8B text encoder, so it needs ~9.5 GB of RAM
alongside the GPU (it peaks at ~7.8 GB of VRAM with CPU offload) and runs about
twice as slowly as klein 4B here. Fibo, klein 9B and FLUX.3 support
reference-image editing; FLUX.1 schnell is text-to-image only. Fibo needs a
vision projector (`--llm_vision`) and is slower than klein 4B.

Qwen Image 2.1 Turbo is the typography option: it renders long, small text
correctly where the FLUX models drift, costs about 3× klein 4B at the same size,
and peaks at ~10.5 GB of RAM with the 8B text encoder and vision projector. A
1024² edit takes ~11 min here, so 768² is the practical size; the preset snaps
to a 32 px grid because Qwen packs latents in 32 px patches.

Measured on a Quadro M4000 (Maxwell, sm_52, 8 GB), CUDA 12.9,
`GGML_CUDA_NCCL=OFF`, with `--offload-to-cpu --vae-tiling`.

## Backend benchmark (512×512)

Same prompt, seed 42, `euler`, CFG 1.0, `--vae-tiling`. CUDA and Vulkan run
with `--offload-to-cpu` (the app default); CPU has no offload.

| Backend | klein 4B wall / sampling | FLUX.3 wall / sampling | Peak VRAM |
|---|---|---|---|
| **CUDA** | **41 s / 27.9 s** | **115 s / 103.5 s** | 4.4–4.6 GB |
| Vulkan | 109 s / 87.1 s | 355 s / 343.0 s | 4.0–4.3 GB |
| CPU (i5-12400F) | 94 s / 59.1 s | 250 s / 218.8 s | host RAM 4.8–5.8 GB |

Peak VRAM includes ~1.2 GB used by the desktop. Disabling CPU offload changed
wall time by less than 7% and raised VRAM by 0.4–0.8 GB.

CUDA is 2–3× faster than CPU. **Vulkan is slower than CPU on Maxwell**, because
the Vulkan backend has no cooperative-matrix path on sm_52; keep CUDA as the
backend unless you are on a card/driver where Vulkan wins.

## Upscaling

The zoom button over the canvas upscales the current image locally through
stable-diffusion.cpp's ESRGAN/MoSR path. Every model is a native **4×** model;
the dialog also offers a **2×** output, produced by downscaling the 4× result
with Lanczos. Weights are small and downloaded on demand from
**Preferences → Models → Upscaler models**.

| Model | Role | License | Size | 1024²→4096² |
|---|---|---|---|---|
| **Balanced 4×** (default) | Clean/generated images | CC-BY-4.0 | 16 MB | **42 s** |
| **Photo 4× (max detail)** | Maximum photographic detail | BSD-3-Clause | 64 MB | 259 s |
| **Anime 4×** | Line art and illustration | BSD-3-Clause | 17 MB | 84 s |
| **Compressed 4×** | JPEG, noise and blur | CC-BY-4.0 | 32 MB | ≈4 min |
| **Faithful 4× (soft)** | No invented detail | CC-BY-4.0 | 16 MB | 42 s |

Measured on a Quadro M4000 (8 GB) with tile 256. Tile 256 peaks at ~3.3 GB and
falls back to 128 automatically if CUDA runs out of memory.

sd.cpp only supports 4× ESRGAN/MoSR models: the compact `v3` models and 2×
ESRGAN/MoSR weights are rejected. Consequently the largest accepted input is
2048 px, because the native 4× pass must stay within the server's 8192 px
limit. Upscaling unloads the generation model first so both fit in 8 GB.

Upscaling is a queued job. Clicking the button while a generation is running
queues the running result when you are watching its live preview, or the image
you selected in History when you are browsing. The job waits behind earlier
work, shows up in the Queue section with its own cancel button, and unloads the
generation model before the upscaler starts.

Upscaled images are saved to the output folder and appear in history with their
source path, upscaler and output size. Deleting an upscale never deletes the
source image, and deleting any history row works while a job is running.

## LoRAs

Put LoRA weights in `models/loras` (the folder shown by **LoRAs…** in the
sidebar), press **Rescan folder**, then enable the weights you want and set
their strength. Selections are stored in `config.toml` and applied to every
generation, edit and effect through the server's `lora` request field. The
active count is shown next to the button, and the weights are embedded in the
PNG metadata.

LoRAs are model-specific: use weights trained for the active model family
(FLUX, Qwen, …). An incompatible weight is reported when a job runs, and the
image is generated without it.

The dialog validates safetensors headers, so a truncated download is flagged
as **Incomplete file** and cannot be enabled. If the backend still refuses a
weight (for example because it was trained for another architecture), the
finished job shows a warning naming the file.

## Day/night helper

The optional **LFM2.5-VL-450M** vision helper (~332 MB: Q4_K_M weights plus a
Q8 multimodal projector) lets the **Auto day ↔ night** effect decide whether the
current image is night or daylight before applying the opposite prompt. Download
it from **Preferences → Models → Vision helper**. Without it, the **To daylight**
and **To night** effects apply those prompts explicitly.

After a run the upscaled image fades in over the original. Scroll up or down
over the canvas, or use the arrows on the right edge, to step through history
with the same crossfade. Since an upscale is recorded next to its source, one
scroll switches between the two so detail and artifacts can be compared
without leaving the app.

## Usage

| Action | Shortcut / control |
|---|---|
| Generate | `Ctrl+Enter` or **Generate** |
| Cancel | stop button (cancels the running server job) |
| Preferences | `Ctrl+,` or the header menu |
| Toggle the sidebar | `Ctrl+B` or the header button |
| Edit the image | pencil button at the top-right of the image |
| Colorize / recover / enhance / denoise / photoreal / replace background | **Effects** menu at the top-right of the image |
| Auto day ↔ night | **Effects → Auto day ↔ night** (needs the optional vision helper) |
| Convert to daylight | **Effects → To daylight** (used when the helper is not installed) |
| Convert to night | **Effects → To night** (used when the helper is not installed) |
| Expand the image | **Effects → Expand** (sidebar MP + ratio target) |
| Save the edit prompt as an effect | bookmark button at the top-right of the image (edit results only) |
| Edit or delete a saved effect | pencil / trash buttons on its row under **Effects → Your effects** |
| Leave edit mode | **Back** button at the top-right of the image |
| Upscale the image | zoom button at the top-right of the image (2× or 4×) |
| Upscale from the keyboard | `Ctrl+U` |
| Older/newer image | scroll/arrow up for newer, down for older, or `Ctrl+Up` / `Ctrl+Down` |
| Crop to a reference | selection button, drag a rectangle, release (animated zoom into the crop) |
| Undo a crop | undo button (crops only) |
| New image (no reference) | `Ctrl+N` or **New** at the top of History |
| Open an image from disk | `Ctrl+O` or the folder button at the top of History |
| Select multiple images | **Select** at the top of History, then check rows and **Delete** |
| Add reference images | image button in the prompt field, or `+` in the reference strip (multi-select) |
| Preview / remove a reference | click its thumbnail / click the × on the thumbnail |
| Edit an image | pick references with the image button, or open a history row and press Edit |
| Delete an image | trash button on the history row (permanent, with confirmation) |
| Image details | info button on the history row |
| Favorite an image | star button on the history row |
| Search history | `Ctrl+F` or the search button in the **History** header |
| Copy the image | copy button over the canvas |
| Show the image in its folder | folder button over the canvas |
| Drag an image out | drag from the canvas into the file manager |
| Export / import effects | **Effects → Export… / Import…** |
| Batch generation | **Batch** spinner in the sidebar (1–4 images) |
| Generate a variation | shuffle button over the canvas (seed + 1) |
| Compare with the source | split button over the canvas |
| Pause / resume the queue | pause button in the **Queue** header |
| Reorder waiting jobs | up/down arrows on a waiting queue row |
| Duplicate a queued prompt | copy button on a waiting queue row |
| Recall a previous prompt | history button in the composer, or `Alt+Up` / `Alt+Down` |
| Take a photo | camera button in the composer (needs a camera and `ffmpeg`) |
| Choose LoRAs | **LoRAs…** in the sidebar |
| Improve the prompt | ✨ button in the prompt field (needs `llama-cli`) |
| New random seed | shuffle button next to the seed field |
| Preferences / downloads | gear button in the header bar |

Editing is iterative: the first edit uses the selected references, and each
following Generate uses the previous result. Press **New** (or `Ctrl+N`) to
leave edit mode and generate from text alone. The composer opens empty; the
history button in its toolbar loads an earlier prompt, and the camera
button takes a photo and uses it as a reference.

After a generation the prompt field slides away and is cleared. Use the pencil
button on the image (or `New`) to bring it back; with no references selected,
the pencil makes the current image the edit reference. The history button in
the prompt field loads a previous prompt if you want to reuse it. **Back**
leaves edit mode without clearing the image. The selection button crops a
region of the image with an animated zoom and uses the crop as the new
reference; undo plays the same zoom in reverse. Selecting references or
cropping keeps the action toolbar visible and leaves the composer closed;
press **Edit** when you want to type a prompt. `Ctrl+B` toggles the sidebar.

The reference strip above the canvas is the multi-image workspace: add images
with the `+` tile or the image button, click a thumbnail to show it on the
canvas, and remove one with its ×. Picking files, dragging images onto the
canvas, or pasting an image or a copied image file with `Ctrl+V` adds it as a
reference and keeps the
prompt composer open and focused, ready for the edit instruction. Add order defines the prompt indices:
refer to the inputs as `<image_1>`, `<image_2>`, and so on (Fibo 2.0, FLUX.2
klein 9B, FLUX.3 klein and Qwen Image 2.1 all accept multi-image
instructions). With several
references the sidebar MP and ratio set the output size; a single reference
still matches its aspect ratio. FLUX.1 schnell cannot edit images, and the
strip caps at the model's reference limit. History chains follow `<image_1>`,
so a result is grouped under the history entry that produced the first
reference.

**Colorize**, **Recover**, **Enhance**, **Denoise**, **Photoreal** and
**Replace background** use built-in prompts and preserve the source aspect at
the current MP target. With the optional vision helper installed,
**Effects → Auto day ↔ night** asks LFM2.5-VL-450M whether the image is night or
daylight and applies the opposite prompt automatically. Without it,
**Effects → To daylight** and **To night** apply those prompts explicitly.
**Effects → Expand** uses the sidebar MP + ratio as the target size and falls
back to a built-in expansion prompt when the prompt field is empty, so it works
directly from the canvas.

After an edit, the bookmark button over the canvas saves the prompt that
produced the image as a user effect. Saved effects appear under **Your
effects** at the bottom of the effects menu, apply with one click, and can be
renamed, reworded or removed with their pencil and trash buttons. They are
stored in `config.toml`.

Styles are prompt suffixes, e.g. *Cinematic* appends
"cinematic film still, dramatic lighting, shallow depth of field, 35mm". Built-in
options include cinematic, dynamic, photographic, anime, Disney/Pixar, 2D/3D
design, vector, sketch, watercolor, oil painting, comic book, cyberpunk, fantasy,
minimalist and pixel art. The selected style is stored with each history entry.

Prompt improvement reuses the active preset's own text encoder via
[llama.cpp](https://github.com/ggml-org/llama.cpp): it runs `llama-cli` with the
same Qwen GGUF for a few seconds (`-rea off`, 4k context), so no extra model is
needed. The button is hidden when `llama-cli` is not installed; install one from
**Preferences → Backends → llama.cpp backends**. Override the binary or use a
dedicated instruct GGUF in `config.toml`:

```toml
prompt_helper_cli = "/usr/bin/llama-cli"
prompt_helper_model = "/path/to/Qwen3-4B-Instruct.gguf"
```

The improved text replaces the prompt and a toast offers **Undo**. Note that
`stable-diffusion.cpp` has no `--upsample_prompt_mode`; this is the local
equivalent, done outside the diffusion process.

Data locations:

| Path | Contents |
|---|---|
| `~/.local/share/imagiflume/history.db` | SQLite history (prompt, model, style, seed, reference) |
| `~/.local/share/imagiflume/models/` | downloaded model files |
| `<models folder>/loras/` | LoRA weights, shown by **LoRAs…** |
| `~/.local/share/imagiflume/backends/` | downloaded `sd-server` backends |
| `~/.local/share/imagiflume/backends/llama/` | downloaded `llama-cli` backends |
| `~/.cache/imagiflume/thumbs/` | 128 px thumbnails |
| `~/.cache/imagiflume/sd-server.log` | backend server log |
| `~/.cache/imagiflume/crops/` | cropped reference images |
| `~/.config/imagiflume/config.toml` | output dir, offload, model, style, size, sidebar state, backend, prompt-helper paths, user effects |
| `~/Pictures/Imagiflume/` | generated PNGs |

## Licensing and legal

- **Imagiflume** is licensed under **GPL-3.0-or-later** (see `LICENSE`).
- **FLUX.2 klein 4B** and the FLUX.2 VAE from the Apache-2.0
  [Comfy-Org Klein repository](https://huggingface.co/Comfy-Org/Flux-2-Klein-4B)
  are **Apache-2.0**; commercial use is allowed.
- **FLUX.2 klein 9B** weights use the **FLUX Non-Commercial License** from Black
  Forest Labs and are for **non-commercial use only**; commercial use needs a
  license from BFL ([license text](https://huggingface.co/black-forest-labs/FLUX.2-klein-9B/blob/main/LICENSE.txt)).
  The preset requires an explicit in-app confirmation before downloading. The
  GGUF is [unsloth's quantization](https://huggingface.co/unsloth/FLUX.2-klein-9B-GGUF),
  which inherits the same license.
- **FLUX.3 Image klein** weights use the **FLUX Community License**, which
  allows commercial use below the revenue threshold defined in that license;
  review the [model card](https://huggingface.co/black-forest-labs/flux-3-image-klein)
  before commercial use. The weights are gated on Hugging Face.
- **Fibo 2.0 Edit Turbo** is **CC BY-NC 4.0** (© BRIA AI) and is usable for
  **non-commercial purposes only**; commercial use needs a separate agreement
  with BRIA. The preset shows the license in **Models** and requires an explicit
  in-app confirmation before downloading. The GGUF is
  [molbal's quantization](https://huggingface.co/molbal/fibo-2.0-GGUF), which
  inherits the same license.
- **FLUX.1 schnell** is **Apache-2.0**. The preset uses the
  [unsloth GGUF](https://huggingface.co/unsloth/FLUX.1-schnell-GGUF) for the
  transformer, a Q4_K_M T5-XXL encoder and the Apache-2.0 FLUX VAE.
- **Qwen Image 2.1 Turbo** weights use the **Qwen Research License** and are
  for **non-commercial use only**; commercial use needs a separate licence from
  Alibaba Qwen ([license text](https://huggingface.co/Qwen/Qwen-Image-2.1/blob/main/LICENSE)).
  The preset requires an explicit in-app confirmation before downloading. The
  GGUF is [unsloth's quantization](https://huggingface.co/unsloth/Qwen-Image-2.1-Turbo-GGUF),
  which inherits the same license.
- **Qwen3-4B**, **Qwen3-8B**, **Qwen3-VL-4B-Instruct**, **Qwen3-VL-8B-Instruct**,
  **T5-XXL** and **CLIP-L** text encoders are **Apache-2.0** (CLIP-L is MIT). The FLUX VAE is
  Apache-2.0, repacked by Meituan LongCat.
- **LFM2.5-VL-450M** is released under the **LFM Open License v1.0**; commercial
  use is allowed below the revenue threshold defined in that license. The
  weights are downloaded at runtime from Liquid AI's Hugging Face repository.
- **Real-ESRGAN** upscaler weights are **BSD-3-Clause**.
- The **Nomos** and **MoSR** upscalers by Philip Hofmann (Phips) are
  **CC-BY-4.0**; their model cards are linked from Preferences → Credits.
- GGUF quantizations inherit the license of their base model.
- Model weights are downloaded at runtime and are **not** part of this
  repository or package. Their licenses govern the weights and generated
  outputs; check them before commercial use.
- `stable-diffusion.cpp` and `llama.cpp` are MIT-licensed; their bundled
  third-party libraries use BSD/MIT licenses.
- Imagiflume is an independent project and is **not affiliated with Black
  Forest Labs**. FLUX is a trademark of Black Forest Labs; the model licenses
  do not grant trademark rights.

## Setup from source

```bash
# 1. backend binaries, sd-cli + sd-server (~5 min; needs /opt/cuda and gcc-14)
./scripts/build-sdcpp.sh

# 2. model weights (5.4 GB, or pass flux2-klein-9b / flux3-klein / all)
./scripts/fetch-models.sh

# 2b. optional upscaler weights (~150 MB)
./scripts/fetch-models.sh upscalers

# 3. run
cargo run --release
```

The app also downloads models itself: **Preferences → Models**, or on first run
when no model is installed. Only installed presets appear in the sidebar
selector; missing ones can be downloaded or removed from the Models tab.

`sd-server` keeps the model loaded between generations and releases it after
60 seconds without activity. Switching the model in the sidebar releases the
previous context immediately; changing CPU offload does the same. A headless
check of the backend is available with
`cargo run -p imagiflume-backend-sdcpp --example smoke`.

Environment overrides:

| Variable | Meaning |
|---|---|
| `IMAGIFLUME_SD_SERVER` | path to the `sd-server` binary (falls back to a sibling of `IMAGIFLUME_SD_CLI`) |
| `IMAGIFLUME_MODELS_DIR` | model directory (defaults to `./models` when run from the repo) |
| `IMAGIFLUME_DEVICE` | label shown by the backend info |
| `CUDA_ARCH` | SM architectures for `build-sdcpp.sh` (default `52`) |

## Backends

The app no longer assumes CUDA. On first run, if no usable `sd-server` is
found, it downloads the pinned **CPU backend (29 MB)** and makes it active.
The **Preferences → Backends** page detects NVIDIA, AMD, Vulkan and CPU,
shows which downloadable builds are compatible, and lets you download, update,
remove or activate one. It also accepts a custom `sd-server` path.

The same page has a **llama.cpp backends** section for the prompt improver. It
installs `llama-cli` from upstream `ggml-org/llama.cpp` releases (CPU, Vulkan,
ROCm or CUDA, depending on the detected hardware), detects a system `llama-cli`
as **System**, and supports a custom path. llama.cpp is optional, so its CPU
build is only downloaded when you ask for it.

| Backend | Source | Notes |
|---|---|---|
| CPU | upstream `leejet/stable-diffusion.cpp` | Universal fallback; slow |
| Vulkan | upstream | Cross-vendor; may be slow or buggy on old Intel GPUs |
| ROCm 10.1 | upstream | AMD only; ~299 MB |
| CUDA 12 | this repo's workflow | Maxwell and newer; needs `libcudart.so.12` |
| CUDA 13 | this repo's workflow | Turing and newer; needs `libcudart.so.13` |

Upstream does not publish Linux CUDA builds, so the CUDA entries are built by
`.github/workflows/build-backends.yml` (manual dispatch) and pinned in
`backends/manifest.json`. Builds are published as release assets; the app only
uses entries whose URL and SHA-256 are present in the manifest. The manifest is
deliberately pinned to tested commits; the refresh button checks it manually.

CUDA major versions are not interchangeable. A CUDA 12 build will not start on
a CUDA 13-only system, and CUDA 13 may not support Maxwell (`sm_52`). The app
checks the installed runtime and compute capability before offering a build.

The backend is launched with an explicit `--backend` value (`cpu`, `vulkan0`,
`cuda0`). Changing it in Preferences restarts the server on the next request.
The self-contained upstream zips use `RUNPATH=$ORIGIN`, so no
`LD_LIBRARY_PATH` is needed.

llama.cpp builds are pinned in `backends/llama-manifest.json`. Upstream ships
them as `.tar.gz` archives whose single top-level directory is flattened during
install. The refresh button checks the curated manifest and then the newest
upstream release with Linux x64 assets; releases without assets (such as
`v1.0.0`) are skipped. CUDA and ROCm builds expect the matching host runtime;
the separate `cudart-*` archives are not bundled.

## Packaging

`packaging/` contains the PKGBUILD, desktop entry, icon, AppStream metadata and
Flatpak manifests.

### Flatpak

`packaging/flatpak/` is the recommended beta channel: the app needs GTK 4.16+
and libadwaita 1.6+, which most current distros do not ship. It targets the
GNOME 51 runtime.

Both the CPU `sd-server` and `llama-cli` are built from source in the manifest,
so the Flatpak works out of the box without downloading an executable backend.
The x86_64 build uses AVX2/FMA (x86-64-v3, Intel Haswell 2013+ or AMD
Excavator/Zen+). Models are still downloaded on first use.

```bash
# working tree (uses the local manifest)
flatpak-builder --user --install --force-clean \
  --state-dir=.flatpak-builder \
  build-dir packaging/flatpak/io.github.tibssy.Imagiflume.local.yml

flatpak run io.github.tibssy.Imagiflume
```

`io.github.tibssy.Imagiflume.yml` is the release manifest: it clones `main` and
is what a Flathub submission would pin to a tag. Both use
`cargo-sources.json`, generated from `Cargo.lock` with Flathub's
`flatpak-cargo-generator.py`; regenerate it whenever dependencies change:

```bash
python3 flatpak-cargo-generator.py Cargo.lock \
  -o packaging/flatpak/cargo-sources.json
```

Models, history and config live under
`~/.var/app/io.github.tibssy.Imagiflume/`; the bundled backends are under
`/app/lib/imagiflume`. The sandbox can read and write `Pictures` and
`Downloads`; everything else goes through the file chooser portal.

Custom backend binaries are hidden in the Flatpak: they link against host
CUDA/ROCm libraries that the sandbox does not expose. Use the bundled CPU
backends or install the Vulkan backend from Preferences.

### Arch

The package builds the CUDA backend for Maxwell by default:

```bash
# build a source tarball from this checkout (vendor/, models/ and build
# artifacts are not shipped)
tar --transform 's,^,imagiflume-0.1.0/,' -czf packaging/imagiflume-0.1.0.tar.gz \
    Cargo.toml Cargo.lock LICENSE README.md .gitignore crates scripts \
    packaging/PKGBUILD packaging/io.github.tibssy.Imagiflume.desktop \
    packaging/io.github.tibssy.Imagiflume.svg \
    packaging/io.github.tibssy.Imagiflume.metainfo.xml

cd packaging
IMAGIFLUME_CUDA_ARCH=52 makepkg -si
```

The installed binary discovers a backend in this order: `IMAGIFLUME_SD_SERVER`,
`sd_server` in the config, a sibling of `sd_cli`, a server next to the app
binary, `/usr/lib/imagiflume`, `/usr/local`, `~/.local`, then `PATH`. If none
is found it downloads the CPU backend automatically.

## Layout

```
crates/core           request types, Backend trait, presets, styles, size rules
crates/backend-sdcpp  sd-server process/HTTP backend, sd-cli fallback, progress parsing
crates/models         resumable downloader driven by the preset registry
crates/app            GTK4 / libadwaita UI, history database
  main.rs             window, layout, generation flow
  sidebar.rs          setup controls, aspect preview, history list container
  main_view.rs        image area and composer
  history_view.rs     rows, details dialog, delete flow
  preferences.rs      preferences + model downloader
  state.rs            text-to-image vs edit state transitions
packaging/            PKGBUILD, .desktop, icon
scripts/              reproducible backend + model setup
```

App name, application id and data directory live in
`crates/core/src/identity.rs`, so renaming the app does not mean hunting for
strings.

## Adding a model

Add a `ModelPreset` in `crates/core/src/preset.rs` (files, URLs, steps, CFG,
sampler, edit support, size grid and optional fixed sigma schedule) and append
it to `PRESETS`. The dropdown, downloader,
history and backend pick it up automatically. Styles live in
`crates/core/src/style.rs`; add a `StylePreset` with a prompt suffix and append
it to `STYLES`. Arbitrary user-supplied models from Hugging Face are planned
next.

## Portability

The GitHub alpha binaries are built on Debian 13 (trixie) in CI with GTK 4.18
and libadwaita 1.7. The app binary is not bundled with its toolkit: it needs a
host with **GTK 4.16+, libadwaita 1.6+ and glibc 2.39+** (Debian 13, Ubuntu
24.04 or newer). A local `cargo build --release` instead matches your own
distro's GTK stack, so it will not run on older releases than that distro.

The backend archives are self-contained; the app binary is not. If a distro
reports a missing library, check:

```bash
ldd ./imagiflume | grep "not found"
```

On Fedora/Bazzite, `libbz2.so.1.0` comes from `bzip2-libs` (pulled in by
gdk-pixbuf), not from Imagiflume. Install it with
`rpm-ostree install bzip2-libs` and reboot, then check for further missing
sonames such as `liblzma`, `libzstd` or `libxml2`. For a distro-independent
install, a Flatpak/AppImage build is the correct long-term solution; a raw
binary will always depend on the host GTK stack.

## Notes for Maxwell / 8 GB cards

- Build CUDA with `-DCMAKE_CUDA_ARCHITECTURES=52`. No tensor cores, so time
  scales with `params × steps`.
- Disable NCCL (`-DGGML_CUDA_NCCL=OFF`) unless your NCCL matches the toolkit.
- `--offload-to-cpu --vae-tiling` keeps peak VRAM around 5 GB.
- Generation keeps roughly one CPU core busy: CUDA work is dispatched from a
  single thread while the GPU computes. This is expected and `--threads` does
  not change GPU sampling time.
- Avoid models whose text encoder alone exceeds ~8 GB (FLUX.2 dev, Krea2 Q4,
  Qwen-Image-2.1, MOSS-Image): they run, but swap and take 10+ minutes.

## Roadmap

- ~~M2: history/gallery, seed control, resolution presets, persistence~~
- ~~M3: preferences, first-run downloader, PKGBUILD, desktop file, icon~~
- ~~M4: LoRA support, batch generation, queue controls~~
- Post-beta: user-supplied Hugging Face models and mask inpainting
