# Backend manifests

Imagiflume ships two manifests:

- `manifest.json` for `sd-server` (stable-diffusion.cpp) image-generation backends.
- `llama-manifest.json` for `llama-cli` (llama.cpp) prompt-improvement backends.

Both are embedded in the app and also fetched from `main` when the user presses
**Check for updates** in Preferences → Backends. They are deliberately pinned to
tested builds; updating them is a manual release decision.

## Entries

Each entry describes one self-contained server archive:

| Field | Meaning |
|---|---|
| `id` | Stable install directory name, e.g. `upstream-cpu` |
| `kind` | `cpu`, `vulkan`, `rocm` or `cuda` |
| `version` | Displayed version / commit; also used for update detection |
| `url` | Archive URL |
| `sha256` | Required checksum |
| `size` | Exact archive size in bytes |
| `backend` | `--backend` value passed to `sd-server` |
| `cuda_major` / `rocm_major` | Required runtime major |
| `archs` | CUDA compute capabilities included in the build |
| `requires` | Informational shared-library requirements |
| `min_glibc` | Minimum glibc version |
| `strip_prefix` | Directory to strip from the archive (auto-detected otherwise) |
| `binary` | Executable to install and run (default `sd-server`) |
| `archive` | `zip` (default) or `tar-gz`; inferred from the URL when omitted |

An entry with an empty `url`, zero `size`, or empty `sha256` is shown as
“not published yet” and cannot be downloaded.

## Upstream CPU / Vulkan / ROCm

The current entries pin `leejet/stable-diffusion.cpp` `master-1094-3bad892`:

- CPU: `sd-master-3bad892-bin-Linux-Ubuntu-26.04-x86_64.zip`
- Vulkan: `sd-master-3bad892-bin-Linux-Ubuntu-26.04-x86_64-vulkan.zip`
- ROCm: `sd-master-3bad892-bin-Linux-Ubuntu-24.04-x86_64-rocm-10.1.0.zip`

The archives include `sd-server`, `sd-cli`, their shared libraries and use
`RUNPATH=$ORIGIN`. The app extracts them into
`~/.local/share/imagiflume/backends/<id>/`.

## CUDA builds

Upstream does not publish Linux CUDA archives. Run the manual
`.github/workflows/build-backends.yml` workflow. It builds:

- CUDA 12.8.1 on Ubuntu 22.04, architectures `52;61;70` (Maxwell, Pascal, Volta)
- CUDA 13.0 on Ubuntu 24.04, architectures `75;80;86;89;90;100;120` (Turing and newer)

The resulting archives contain `sd-server`, `sd-cli` and their libraries. They
link against the host's matching `libcudart.so.N`; the CUDA runtime is not
bundled.

The workflow publishes `sd-cuda12-linux-x86_64.zip` and
`sd-cuda13-linux-x86_64.zip` (plus a small JSON file with the SHA-256 and size)
to the release tag passed as `release_tag`. After publishing a release, update
the `cuda12` and `cuda13` entries with the archive URL, SHA-256 and size. Keep
`version` in sync with the pinned upstream commit so update detection works.

CUDA 13 may not support Maxwell (`sm_52`). The app checks `compute_cap` against
the entry's `archs` before offering the build.

## llama.cpp builds

`llama-manifest.json` pins `ggml-org/llama.cpp` release assets:

- CPU: `llama-<tag>-bin-ubuntu-x64.tar.gz`
- Vulkan: `llama-<tag>-bin-ubuntu-vulkan-x64.tar.gz`
- ROCm: `llama-<tag>-bin-ubuntu-rocm-<major>.<minor>-x64.tar.gz`
- CUDA: `llama-<tag>-bin-ubuntu-cuda-<major>.<minor>-x64.tar.gz`

Entries use `binary: "llama-cli"` and `archive: "tar-gz"`. The archive contains
a single top-level `llama-<tag>/` directory, which the installer flattens into
`~/.local/share/imagiflume/backends/llama/<id>/`. Its binaries use
`RUNPATH=$ORIGIN`.

The refresh path queries the releases list rather than `/releases/latest`,
because upstream can publish a release without assets (for example `v1.0.0`).
It picks the newest release with Linux x64 assets and ignores `arm64`, `s390x`,
OpenVINO, SYCL and `cudart-*` archives. CUDA and ROCm builds require the
matching host runtime; the CUDA runtime is not bundled.
