# Flux Studio

A GTK4 / libadwaita desktop app for **local image generation and editing**,
driven by [stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp).
No Python, no PyTorch, no cloud.

Ships with two FLUX presets and uses FLUX's native reference-image support for
editing, so text-to-image and image-to-image share the same weights.

## Status

Milestone 3 (packaged desktop app):

- libadwaita window with prompt composer, live progress and cancel
- **model dropdown** with per-preset defaults (steps, CFG, sampler, files)
- **reference-image editing** (`-r`) for models that support it
- **generation history** in a sidebar gallery (SQLite + 128 px thumbnails)
- **seed control**; the seed actually used is stored and shown
- **resolution presets** (512² to 1152×768)
- **preferences dialog**: output folder, CPU offload, models folder
- **model downloader** with resume, progress and per-preset install status
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

## Usage

| Action | Shortcut / control |
|---|---|
| Generate | `Ctrl+Enter` or **Generate** |
| Cancel | **Cancel** (kills the `sd-cli` process) |
| Edit an image | **Reference image…**, then describe the change in the prompt |
| Reproduce an image | click it in the history, then **Generate** |
| New random seed | shuffle button next to the seed field |
| Preferences / downloads | gear button in the header bar |

Editing with klein: pick a reference image, write what should change
("make it winter, snow on the hills"), and generate. It runs at 4 steps and
keeps the composition of the reference.

Data locations:

| Path | Contents |
|---|---|
| `~/.local/share/flux-studio/history.db` | SQLite history (prompt, model, seed, reference) |
| `~/.local/share/flux-studio/models/` | downloaded model files |
| `~/.cache/flux-studio/thumbs/` | 128 px thumbnails |
| `~/.config/flux-studio/config.toml` | output dir, offload, active preset, size |
| `~/Pictures/Flux Studio/` | generated PNGs |

## Setup from source

```bash
# 1. backend binary (~5 min; needs /opt/cuda and gcc-14)
./scripts/build-sdcpp.sh

# 2. model weights (5.4 GB, or pass flux3-klein / all)
./scripts/fetch-models.sh

# 3. run
cargo run --release
```

The app also downloads models itself: **Preferences → Models**, or on first run
when the active preset is missing.

Environment overrides:

| Variable | Meaning |
|---|---|
| `FLUX_SD_CLI` | path to the `sd-cli` binary |
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
    packaging/PKGBUILD packaging/io.github.ltux.FluxStudio.desktop \
    packaging/io.github.ltux.FluxStudio.svg

cd packaging
FLUX_CUDA_ARCH=52 makepkg -si
```

The installed binary looks for the backend at `/usr/lib/flux-studio/sd-cli`.

## Layout

```
crates/core           request types, Backend trait, presets, config, XDG paths
crates/backend-sdcpp  sd-cli process management, arg builder, progress parser
crates/models         resumable downloader driven by the preset registry
crates/app            GTK4 / libadwaita UI, history database
packaging/            PKGBUILD, .desktop, icon
scripts/              reproducible backend + model setup
```

App name, application id and data directory live in
`crates/core/src/identity.rs`, so renaming the app does not mean hunting for
strings.

## Adding a model

Add a `ModelPreset` in `crates/core/src/preset.rs` (files, URLs, steps, CFG,
sampler, edit support) and append it to `PRESETS`. The dropdown, downloader,
history and backend pick it up automatically. Arbitrary user-supplied models
from Hugging Face are planned next.

## Notes for Maxwell / 8 GB cards

- Build CUDA with `-DCMAKE_CUDA_ARCHITECTURES=52`. No tensor cores, so time
  scales with `params × steps`.
- Disable NCCL (`-DGGML_CUDA_NCCL=OFF`) unless your NCCL matches the toolkit.
- `--offload-to-cpu --vae-tiling` keeps peak VRAM around 5 GB.
- Avoid models whose text encoder alone exceeds ~8 GB (FLUX.2 dev, Krea2 Q4,
  Qwen-Image-2.1, MOSS-Image): they run, but swap and take 10+ minutes.

## Roadmap

- ~~M2: history/gallery, seed control, resolution presets, persistence~~
- ~~M3: preferences, first-run downloader, PKGBUILD, desktop file, icon~~
- M4: add user-supplied Hugging Face models, LoRA support, batch queue
