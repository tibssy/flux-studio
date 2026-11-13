# Flux Studio

A GTK4 / libadwaita desktop app for **local image generation and editing**,
driven by [stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp).
No Python, no PyTorch, no cloud.

Ships with two FLUX presets and uses FLUX's native reference-image support for
editing, so text-to-image and image-to-image share the same weights.

## Status

Milestone 3 (packaged desktop app):

- **hideable sidebar** with a Setup section and a History section
- **resolution sliders** (128–1280 px in 64 px steps) with a live aspect-ratio
  preview and megapixel readout
- **style presets** (cinematic, dynamic, 2D, 3D, photographic, anime) applied
  as prompt suffixes
- **model dropdown restricted to installed presets**, seed control and an Advanced group (fixed steps/CFG,
  sampler, CPU offload) in the sidebar
- focused composer: prompt, Generate/Cancel and a reference-image button
- **reference-image editing** (`-r`); edits promote their result to the next
  reference, while text-to-image runs stay text-to-image
- **history** with details and delete actions per row; delete removes the PNG,
  thumbnail and database entry
- clicking a history image restores its prompt, model, style, size and seed and
  loads it as the reference
- **model caching**: `sd-server` keeps the loaded model for 60 s of idle time
  and releases it immediately when you switch models
- **prompt improvement** through llama.cpp using the active text encoder,
  with undo
- **image actions** over the canvas: upscale, edit, crop-to-reference, and undo
- **image upscaling** with five permissive 4× models, 2×/4× output, model
  downloads and tile-size fallback for 8 GB GPUs
- **history navigation** with a crossfade: scroll over the image or use the
  up/down arrows to step through history and compare an upscale with its source
- the composer slides away during generation and returns via Edit/New
- **preferences dialog**: output folder, CPU offload, models folder, auto-hide
- **About page** with repository/issue links plus Credits and Legal subpages with back navigation
- **model downloader** with resume, progress and per-preset install status,
  plus a delete action that keeps files shared with other presets
- first run opens the Models page when the active preset is missing
- PKGBUILD, `.desktop` entry and scalable icon

## Models

| Preset | Files | Steps | Measured @768² | Peak VRAM |
|---|---|---|---|---|
| **FLUX.2 klein 4B** (default) | 5.4 GB | 4 | **78–85 s** | 4.7 GB |
| **FLUX.3 Image klein** | 6.3 GB | 16 | **254 s** | 5.0 GB |

FLUX.3 is the quality option (~3× slower on Maxwell); klein 4B is the fastest
4-step model that fits this GPU. Both support reference-image editing.

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

Upscaled images are saved to the output folder and appear in history with their
source path, upscaler and output size. Deleting an upscale never deletes the
source image.

After a run the upscaled image fades in over the original. Scroll up or down
over the canvas, or use the arrows on the right edge, to step through history
with the same crossfade. Since an upscale is recorded next to its source, one
scroll switches between the two so detail and artifacts can be compared
without leaving the app.

## Usage

| Action | Shortcut / control |
|---|---|
| Generate | `Ctrl+Enter` or **Generate** |
| Cancel | **Cancel** (cancels the running server job) |
| Toggle the sidebar | `Ctrl+B` or the header button |
| Edit the image | pencil button at the top-right of the image |
| Upscale the image | zoom button at the top-right of the image (2× or 4×) |
| Upscale from the keyboard | `Ctrl+U` |
| Older/newer image | scroll over the canvas, use the up/down arrows, or `Ctrl+Up` / `Ctrl+Down` |
| Crop to a reference | selection button, drag a rectangle, release (animated zoom into the crop) |
| Undo a crop | undo button (crops only) |
| New image (no reference) | `Ctrl+N` or **New** at the top of History |
| Edit an image | pick a reference with the image button, or click a history row |
| Delete an image | trash button on the history row (permanent, with confirmation) |
| Image details | info button on the history row |
| Improve the prompt | ✨ button in the prompt field (needs `llama-cli`) |
| New random seed | shuffle button next to the seed field |
| Preferences / downloads | gear button in the header bar |

Editing is iterative: the first edit uses the selected reference, and each
following Generate uses the previous result. Press **New** (or `Ctrl+N`) to
leave edit mode and generate from text alone; the prompt is kept.

After a generation the prompt field slides away. Use the pencil button on the
image (or `New`, or a history entry) to bring it back; the pencil makes the
current image the edit reference. The selection button crops a region of the
image with an animated zoom, uses the crop as the new reference, and opens the
composer; undo plays the same zoom in reverse. The sidebar
hides itself after 3 seconds when neither the keyboard focus nor the pointer is
inside it; `Ctrl+B` brings it back. Turn this off in **Preferences → Interface →
Auto-hide sidebar**.

Styles are prompt suffixes, e.g. *Cinematic* appends
"cinematic film still, dramatic lighting, shallow depth of field, 35mm". Built-in
options include cinematic, dynamic, photographic, anime, Disney/Pixar, 2D/3D
design, vector, sketch, watercolor, oil painting, comic book, cyberpunk, fantasy,
minimalist and pixel art. The selected style is stored with each history entry.

Prompt improvement reuses the active preset's own text encoder via
[llama.cpp](https://github.com/ggml-org/llama.cpp): it runs `llama-cli` with the
same Qwen GGUF for a few seconds (`-rea off`, 4k context), so no extra model is
needed. The button is hidden when `llama-cli` is not installed. Override the
binary or use a dedicated instruct GGUF in `config.toml`:

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
| `~/.local/share/flux-studio/history.db` | SQLite history (prompt, model, style, seed, reference) |
| `~/.local/share/flux-studio/models/` | downloaded model files |
| `~/.cache/flux-studio/thumbs/` | 128 px thumbnails |
| `~/.cache/flux-studio/sd-server.log` | backend server log |
| `~/.cache/flux-studio/crops/` | cropped reference images |
| `~/.config/flux-studio/config.toml` | output dir, offload, model, style, size, sidebar state, prompt-helper paths |
| `~/Pictures/Flux Studio/` | generated PNGs |

## Licensing and legal

- **Flux Studio** is licensed under **GPL-3.0-or-later** (see `LICENSE`).
- **FLUX.2 klein 4B** and the FLUX.2 VAE from the Apache-2.0
  [Comfy-Org Klein repository](https://huggingface.co/Comfy-Org/Flux-2-Klein-4B)
  are **Apache-2.0**; commercial use is allowed.
- **FLUX.3 Image klein** weights use the **FLUX Community License**, which
  allows commercial use below the revenue threshold defined in that license;
  review the [model card](https://huggingface.co/black-forest-labs/flux-3-image-klein)
  before commercial use. The weights are gated on Hugging Face.
- **Qwen3-4B** and **Qwen3-VL-4B-Instruct** text encoders are **Apache-2.0**.
- **Real-ESRGAN** upscaler weights are **BSD-3-Clause**.
- The **Nomos** and **MoSR** upscalers by Philip Hofmann (Phips) are
  **CC-BY-4.0**; their model cards are linked from Preferences → Credits.
- GGUF quantizations inherit the license of their base model.
- Model weights are downloaded at runtime and are **not** part of this
  repository or package. Their licenses govern the weights and generated
  outputs; check them before commercial use.
- `stable-diffusion.cpp` and `llama.cpp` are MIT-licensed; their bundled
  third-party libraries use BSD/MIT licenses.
- Flux Studio is an independent project and is **not affiliated with Black
  Forest Labs**. FLUX is a trademark of Black Forest Labs; the model licenses
  do not grant trademark rights.

## Setup from source

```bash
# 1. backend binaries, sd-cli + sd-server (~5 min; needs /opt/cuda and gcc-14)
./scripts/build-sdcpp.sh

# 2. model weights (5.4 GB, or pass flux3-klein / all)
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
`cargo run -p flux-backend-sdcpp --example smoke`.

Environment overrides:

| Variable | Meaning |
|---|---|
| `FLUX_SD_SERVER` | path to the `sd-server` binary (falls back to a sibling of `FLUX_SD_CLI`) |
| `FLUX_MODELS_DIR` | model directory (defaults to `./models` when run from the repo) |
| `FLUX_DEVICE` | label shown by the backend info |
| `CUDA_ARCH` | SM architectures for `build-sdcpp.sh` (default `52`) |

## Packaging

`packaging/` contains the PKGBUILD, desktop entry and icon. The package builds
the CUDA backend for Maxwell by default:

```bash
# build a source tarball from this checkout (vendor/, models/ and build
# artifacts are not shipped)
tar --transform 's,^,flux-studio-0.1.0/,' -czf packaging/flux-studio-0.1.0.tar.gz \
    Cargo.toml Cargo.lock LICENSE README.md .gitignore crates scripts \
    packaging/PKGBUILD packaging/io.github.tibssy.FluxStudio.desktop \
    packaging/io.github.tibssy.FluxStudio.svg

cd packaging
FLUX_CUDA_ARCH=52 makepkg -si
```

The installed binary looks for the backend at `/usr/lib/flux-studio/sd-server`.

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
sampler, edit support) and append it to `PRESETS`. The dropdown, downloader,
history and backend pick it up automatically. Styles live in
`crates/core/src/style.rs`; add a `StylePreset` with a prompt suffix and append
it to `STYLES`. Arbitrary user-supplied models from Hugging Face are planned
next.

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
- M4: add user-supplied Hugging Face models, LoRA support, batch queue
