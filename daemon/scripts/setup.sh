#!/usr/bin/env bash
# Reproducible development/runtime setup for Duet.
#
# Installs the toolchain (via mise, no sudo), the Python environment for the
# sidecars, and downloads the models. Assumes Omarchy/Arch with `mise`.
#
# Usage:
#   bash daemon/scripts/setup.sh
#   DUET_RUNTIME="$HOME/.local/share/duet" bash daemon/scripts/setup.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUNTIME="${DUET_RUNTIME:-$REPO_ROOT}"
MODELS="$RUNTIME/models"
VENV="$RUNTIME/.venv-py"
mkdir -p "$MODELS"

verify_sha256() {
  local file="$1" expected="$2" actual
  actual="$(sha256sum "$file" | awk '{print $1}')"
  if [ "$actual" != "$expected" ]; then
    echo "setup.sh: SHA-256 mismatch for $file" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    return 1
  fi
}

fetch_verified() {
  local url="$1" dest="$2" expected="$3" part="$2.part"
  if [ -f "$dest" ]; then
    verify_sha256 "$dest" "$expected"
    return
  fi
  echo "  downloading $(basename "$dest")"
  if ! curl -fL --retry 2 -o "$part" "$url"; then
    rm -f "$part"
    exit 1
  fi
  if ! verify_sha256 "$part" "$expected"; then
    rm -f "$part"
    exit 1
  fi
  mv "$part" "$dest"
}

echo "==> 1/5 Toolchain (rust, cmake, ninja, uv) via mise"
mise use -g rust@stable uv@latest cmake@latest ninja@latest

echo "==> 2/5 Python environment for the sidecars ($VENV)"
uv python install 3.12
uv venv --python 3.12 "$VENV"
uv pip install --python "$VENV/bin/python" ctranslate2 sentencepiece piper-tts

echo "==> 3/5 Whisper models (ggml)"
WHISPER_BASE=https://huggingface.co/ggerganov/whisper.cpp/resolve/main
# base-q5_1 is the default: best quality/latency/RAM trade-off for CPU use.
# base (f16) is downloaded as a quality reference.
fetch_verified "$WHISPER_BASE/ggml-base-q5_1.bin" \
  "$MODELS/ggml-base-q5_1.bin" \
  422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898
fetch_verified "$WHISPER_BASE/ggml-base.bin" \
  "$MODELS/ggml-base.bin" \
  60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe

echo "==> 3b/5 Silero VAD (optional: the default VAD is WebRTC)"
# Only used with --vad silero. Silero is too heavy for streaming on CPU (see
# README), but it is downloaded in case you want to try its quality.
fetch_verified \
  "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin" \
  "$MODELS/ggml-silero-v5.1.2.bin" \
  29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf

echo "==> 4/5 Piper voices (es_MX + en_US)"
PIPER_BASE=https://huggingface.co/rhasspy/piper-voices/resolve/main
fetch_voice() { # $1 = relative path without extension, $2 = onnx hash, $3 = json hash
  local rel="$1" onnx_sha="$2" json_sha="$3" n
  n="$(basename "$rel")"
  fetch_verified "$PIPER_BASE/$rel.onnx" "$MODELS/$n.onnx" "$onnx_sha"
  fetch_verified "$PIPER_BASE/$rel.onnx.json" "$MODELS/$n.onnx.json" "$json_sha"
}
fetch_voice "es/es_MX/claude/high/es_MX-claude-high" \
  3ef40a71ea63852cd8ab7e6fa7d2ecdcfa67a0b47c9c48e3f10e02ee02083ea0 \
  1afc81f703c0e4cb3b4d7c0dca096b8b54a98806807f0170cf5eb5557723c12d
fetch_voice "en/en_US/lessac/medium/en_US-lessac-medium" \
  5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f \
  efe19c417bed055f2d69908248c6ba650fa135bc868b0e6abb3da181dab690a0

echo "==> 5/5 Translation models (Argos/OPUS-MT, CTranslate2 format)"
mkdir -p "$MODELS/mt" && cd "$MODELS/mt"
# The v1.0 packages include the standard binary `sentencepiece.model`.
fetch_argos() { # $1 = package name, $2 = archive SHA-256
  local p="$1" expected="$2" archive="$1.argosmodel"
  if [ -d "$p" ]; then
    return
  fi
  echo "  downloading $archive"
  if ! curl -fL --retry 2 -o "$archive" \
    "https://argos-net.com/v1/translate-$archive"; then
    rm -f "$archive"
    exit 1
  fi
  if ! verify_sha256 "$archive" "$expected"; then
    rm -f "$archive"
    exit 1
  fi
  unzip -oq "$archive"
  rm -f "$archive"
}
fetch_argos "es_en-1_0" \
  1b963aa0e0cb6e5ce874f0aa1a1949a19bc4d762e833532239a8834340f6b378
fetch_argos "en_es-1_0" \
  d698d0ef87ad70d5d184b7fa6965905bf4368f09a2bb9ffb165a79bac96af0c4
cd "$REPO_ROOT"

echo
echo "Runtime ready at: $RUNTIME"
echo "Build from the repository root:"
echo "  cargo build --release                                      # CPU"
echo "  cargo build --release -p duet --features vulkan  # GPU (requires vulkan-headers)"
