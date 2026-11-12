#!/usr/bin/env bash
# Build the sd-cli backend for this machine.
#
# The Quadro M4000 is Maxwell (sm_52) and the distro's NCCL is built against
# CUDA 13 while /opt/cuda is 12.9, so NCCL is disabled to avoid a link error.
# Adjust CUDA_ARCH for other GPUs (e.g. 86 for Ampere, 89 for Ada).
#
# Environment:
#   SDCPP_SRC   existing stable-diffusion.cpp checkout (skips the clone)
#   CUDA_HOME   CUDA toolkit root (default /opt/cuda)
#   CUDA_ARCH   comma-separated SM architectures (default 52)
#   HOST_CC/HOST_CXX  host compilers (default gcc-14/g++-14)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${SDCPP_SRC:-$ROOT/vendor/stable-diffusion.cpp}"
CUDA_HOME="${CUDA_HOME:-/opt/cuda}"
CUDA_ARCH="${CUDA_ARCH:-52}"
HOST_CC="${HOST_CC:-gcc-14}"
HOST_CXX="${HOST_CXX:-g++-14}"

if [ ! -d "$SRC/.git" ] && [ ! -f "$SRC/CMakeLists.txt" ]; then
  git clone --recursive --depth 1 \
    https://github.com/leejet/stable-diffusion.cpp "$SRC"
elif [ -f "$SRC/.gitmodules" ] && [ ! -f "$SRC/ggml/CMakeLists.txt" ]; then
  # Packaged checkouts (e.g. makepkg) do not fetch submodules.
  git -C "$SRC" submodule update --init --recursive
fi

export PATH="$CUDA_HOME/bin:$PATH"

cmake -S "$SRC" -B "$SRC/build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DSD_CUDA=ON \
  -DGGML_CUDA_NCCL=OFF \
  -DCMAKE_CUDA_ARCHITECTURES="$CUDA_ARCH" \
  -DCMAKE_C_COMPILER="$HOST_CC" \
  -DCMAKE_CXX_COMPILER="$HOST_CXX" \
  -DCMAKE_CUDA_HOST_COMPILER="$HOST_CXX" \
  -DSD_SERVER_BUILD_FRONTEND=OFF

cmake --build "$SRC/build" --target sd-cli sd-server -j"$(nproc)"

echo "built: $SRC/build/bin/sd-cli and $SRC/build/bin/sd-server"
