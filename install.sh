#!/usr/bin/env bash
# Install the Duet runtime outside the plugin checkout.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME="${DUET_RUNTIME:-${XDG_DATA_HOME:-$HOME/.local/share}/duet}"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/duet"
BUILD_MODE="auto"
NO_BUILD=0
NO_MODELS=0
LINK_MODELS=0
LINK_VENV=0
BINARY=""

usage() {
  cat <<'EOF'
Install the Duet runtime outside the plugin checkout.

The Omarchy plugin itself is just the QML manifest and components. This
script builds the Rust daemon, installs the runtime helper scripts, creates the
Python environment, and downloads the models under:

  ${XDG_DATA_HOME:-$HOME/.local/share}/duet

Run it from the repository/plugin directory:

  ./install.sh              # auto-select CPU/GPU, build, and download models
  ./install.sh --gpu        # force a Vulkan build
  ./install.sh --cpu        # force a CPU-only build
  ./install.sh --binary target/release/duet --no-models

Options:
  --runtime DIR    Override the runtime directory.
  --binary PATH    Install an existing binary instead of building.
  --no-build       Use target/release/duet without rebuilding.
  --no-models      Do not run the model/Python setup.
  --link-models    Symlink the development models/ directory into the runtime.
  --link-venv      Symlink the development .venv-py/ directory into the runtime.
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --runtime) shift; RUNTIME="${1:?missing value for --runtime}" ;;
    --binary) shift; BINARY="${1:?missing value for --binary}" ;;
    --cpu) BUILD_MODE="cpu" ;;
    --gpu) BUILD_MODE="gpu" ;;
    --no-build) NO_BUILD=1 ;;
    --no-models) NO_MODELS=1 ;;
    --link-models) LINK_MODELS=1 ;;
    --link-venv) LINK_VENV=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "install.sh: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

mkdir -p "$RUNTIME"
RUNTIME="$(cd "$RUNTIME" && pwd)"
mkdir -p "$RUNTIME/bin" "$RUNTIME/scripts"

# ---------------------------------------------------------------------------
# Binary
# ---------------------------------------------------------------------------
if [ -n "$BINARY" ]; then
  case "$BINARY" in
    /*) ;;
    *) BINARY="$ROOT/$BINARY" ;;
  esac
  if [ ! -x "$BINARY" ]; then
    echo "install.sh: binary is not executable: $BINARY" >&2
    exit 1
  fi
elif [ "$NO_BUILD" = 1 ]; then
  BINARY="$ROOT/target/release/duet"
  if [ ! -x "$BINARY" ]; then
    echo "install.sh: no binary at $BINARY; build first or pass --binary" >&2
    exit 1
  fi
else
  command -v cargo >/dev/null 2>&1 || {
    echo "install.sh: cargo not found; install Rust (rustup or mise) first" >&2
    exit 1
  }
  FEATURES=()
  case "$BUILD_MODE" in
    gpu) FEATURES=(--features vulkan) ;;
    cpu) ;;
    auto)
      VULKAN_INCLUDE="${VULKAN_SDK:-/usr}/include/vulkan/vulkan.h"
      if [ -f "$VULKAN_INCLUDE" ] && ldconfig -p 2>/dev/null | grep -q 'libvulkan\.so'; then
        FEATURES=(--features vulkan)
        echo "==> Vulkan headers and loader found; building the GPU backend"
      else
        echo "==> Vulkan headers not found; building the CPU backend"
        echo "    (install vulkan-headers or pass --gpu to force it)"
      fi
      ;;
  esac
  echo "==> Building Duet (${BUILD_MODE})"
  CARGO_TARGET_DIR="$CACHE/build" \
    cargo build --release --manifest-path "$ROOT/Cargo.toml" \
      -p duet "${FEATURES[@]}"
  BINARY="$CACHE/build/release/duet"
fi

install -Dm755 "$BINARY" "$RUNTIME/bin/duet"
install -Dm755 "$ROOT/daemon/scripts/duet-toggle" "$RUNTIME/bin/duet-toggle"
install -Dm755 "$ROOT/daemon/scripts/duet-status" "$RUNTIME/bin/duet-status"
install -Dm644 "$ROOT/daemon/scripts/mt_server.py" "$RUNTIME/scripts/mt_server.py"
install -Dm644 "$ROOT/daemon/scripts/tts_server.py" "$RUNTIME/scripts/tts_server.py"

# ---------------------------------------------------------------------------
# Development shortcuts (optional)
# ---------------------------------------------------------------------------
if [ "$LINK_MODELS" = 1 ]; then
  if [ ! -d "$ROOT/models" ]; then
    echo "install.sh: --link-models requested, but $ROOT/models does not exist" >&2
    exit 1
  fi
  if [ -e "$RUNTIME/models" ] || [ -L "$RUNTIME/models" ]; then
    echo "==> Keeping existing $RUNTIME/models"
  else
    ln -s "$ROOT/models" "$RUNTIME/models"
    echo "==> Linked $RUNTIME/models -> $ROOT/models"
  fi
fi

if [ "$LINK_VENV" = 1 ]; then
  if [ ! -x "$ROOT/.venv-py/bin/python" ]; then
    echo "install.sh: --link-venv requested, but $ROOT/.venv-py/bin/python is missing" >&2
    exit 1
  fi
  if [ -e "$RUNTIME/.venv-py" ] || [ -L "$RUNTIME/.venv-py" ]; then
    echo "==> Keeping existing $RUNTIME/.venv-py"
  else
    ln -s "$ROOT/.venv-py" "$RUNTIME/.venv-py"
    echo "==> Linked $RUNTIME/.venv-py -> $ROOT/.venv-py"
  fi
fi

# ---------------------------------------------------------------------------
# Python environment and models
# ---------------------------------------------------------------------------
if [ "$NO_MODELS" = 0 ]; then
  if [ "$LINK_VENV" = 1 ] || [ "$LINK_MODELS" = 1 ]; then
    echo "==> Skipping setup.sh because development links were requested"
  else
    DUET_RUNTIME="$RUNTIME" bash "$ROOT/daemon/scripts/setup.sh"
  fi
elif [ ! -x "$RUNTIME/.venv-py/bin/python" ]; then
  echo "install.sh: runtime Python is missing; run without --no-models or use --link-venv" >&2
  exit 1
fi

if [ ! -d "$RUNTIME/models" ]; then
  echo "install.sh: warning: $RUNTIME/models is missing; model downloads are required" >&2
fi

echo
echo "Runtime installed at: $RUNTIME"
echo "Start Duet from the Omarchy panel or run:"
echo "  $RUNTIME/bin/duet-toggle start"
echo
REMOTE="$(git -C "$ROOT" remote get-url origin 2>/dev/null || true)"
if [ -n "$REMOTE" ]; then
  echo "To install the plugin itself:"
  echo "  omarchy plugin add $REMOTE"
else
  echo "Publish this repository, then install the plugin with:"
  echo "  omarchy plugin add <git-url>"
fi
