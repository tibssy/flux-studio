#!/usr/bin/env bash
# Fetch model weights into ./models.
#
#   ./scripts/fetch-models.sh                 # FLUX.2 klein 4B (~5.4 GB)
#   ./scripts/fetch-models.sh flux3-klein     # FLUX.3 Image klein (~6.3 GB)
#   ./scripts/fetch-models.sh upscalers       # five upscaler weights (~150 MB)
#   ./scripts/fetch-models.sh all             # all presets and upscalers
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
mkdir -p models/diffusion models/text_encoders models/vae models/upscalers

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

fetch_upscalers() {
  get "https://huggingface.co/Phips/4xNomos2_hq_mosr/resolve/main/4xNomos2_hq_mosr.pth" \
    models/upscalers/4xNomos2_hq_mosr.pth
  get "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.1.0/RealESRGAN_x4plus.pth" \
    models/upscalers/RealESRGAN_x4plus.pth
  get "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.2.4/RealESRGAN_x4plus_anime_6B.pth" \
    models/upscalers/RealESRGAN_x4plus_anime_6B.pth
  get "https://huggingface.co/Phips/4xNomos2_otf_esrgan/resolve/main/4xNomos2_otf_esrgan.safetensors" \
    models/upscalers/4xNomos2_otf_esrgan.safetensors
  get "https://huggingface.co/Phips/4xmssim_mosr_pretrain/resolve/main/4xmssim_mosr_pretrain.pth" \
    models/upscalers/4xmssim_mosr_pretrain.pth
}

case "${1:-flux2-klein-4b}" in
  flux2-klein-4b) fetch_flux2_klein ;;
  flux3-klein) fetch_flux3_klein ;;
  upscalers) fetch_upscalers ;;
  all)
    fetch_flux2_klein
    fetch_flux3_klein
    fetch_upscalers
    ;;
  *)
    echo "unknown preset: $1 (expected flux2-klein-4b, flux3-klein, upscalers or all)" >&2
    exit 1
    ;;
esac

ls -lh models/*/
