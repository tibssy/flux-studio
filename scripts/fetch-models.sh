#!/usr/bin/env bash
# Fetch model weights into ./models.
#
#   ./scripts/fetch-models.sh                 # FLUX.2 klein 4B (~5.4 GB)
#   ./scripts/fetch-models.sh flux3-klein     # FLUX.3 Image klein (~6.3 GB)
#   ./scripts/fetch-models.sh all             # both presets (~11.7 GB)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
mkdir -p models/diffusion models/text_encoders models/vae

get() { # url dest
  local url="$1" dest="$2"
  if [ -s "$dest" ]; then
    echo "[skip] $dest"
    return
  fi
  echo "[get ] $dest"
  curl -L --fail -C - --retry 3 -o "$dest" "$url"
}

fetch_flux2_klein() {
  get "https://huggingface.co/unsloth/FLUX.2-klein-4B-GGUF/resolve/main/flux-2-klein-4b-Q4_K_M.gguf" \
    models/diffusion/flux-2-klein-4b-Q4_K_M.gguf
  get "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf" \
    models/text_encoders/Qwen3-4B-Q4_K_M.gguf
  get "https://huggingface.co/Comfy-Org/Flux-2-Klein-4B/resolve/main/split_files/vae/flux2-vae.safetensors" \
    models/vae/flux2-vae.safetensors
}

fetch_flux3_klein() {
  get "https://huggingface.co/leejet/flux-3-image-klein-GGUF/resolve/main/flux3-image-klein-Q4_K.gguf" \
    models/diffusion/flux3-image-klein-Q4_K.gguf
  get "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3VL-4B-Instruct-Q4_K_M.gguf" \
    models/text_encoders/Qwen3VL-4B-Instruct-Q4_K_M.gguf
  get "https://huggingface.co/Comfy-Org/Flux-2-Klein-4B/resolve/main/split_files/vae/flux2-vae.safetensors" \
    models/vae/flux2-vae.safetensors
}

case "${1:-flux2-klein-4b}" in
  flux2-klein-4b) fetch_flux2_klein ;;
  flux3-klein) fetch_flux3_klein ;;
  all)
    fetch_flux2_klein
    fetch_flux3_klein
    ;;
  *)
    echo "unknown preset: $1 (expected flux2-klein-4b, flux3-klein or all)" >&2
    exit 1
    ;;
esac

ls -lh models/*/
