#!/usr/bin/env bash
# Ubuntu / Debian / WSL setup. Run from any directory.
set -euo pipefail
cd "$(dirname "$0")/.."
if ! command -v apt-get >/dev/null; then
  echo 'This setup script supports Debian/Ubuntu and Ubuntu on WSL.' >&2
  exit 1
fi
sudo apt-get update
sudo apt-get install -y --no-install-recommends build-essential ca-certificates curl git qemu-system-x86 xorriso ovmf
if ! command -v rustup >/dev/null && [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  installer=$(mktemp)
  trap 'rm -f "$installer"' EXIT
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$installer"
  sh "$installer" -y --profile minimal --default-toolchain none
fi
export PATH="$HOME/.cargo/bin:$PATH"
cargo --version
cargo xtask test
