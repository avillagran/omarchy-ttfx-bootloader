#!/bin/bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
sandbox=$(mktemp -d /tmp/ttfx-plymouth-offline.XXXXXX)
cleanup() {
  rm -rf -- "$sandbox"
}
trap cleanup EXIT INT TERM

mkdir -p "$sandbox/source" "$sandbox/cargo-home" "$sandbox/target"
(
  cd "$root"
  tar --exclude='./target' --exclude='./vendor/ttfx/target' -cf - .
) | tar -xf - -C "$sandbox/source"

(
  cd "$sandbox/source"
  CARGO_HOME="$sandbox/cargo-home" CARGO_TARGET_DIR="$sandbox/target" \
    cargo build --locked --offline --profile plymouth
)

printf 'isolated locked offline build: PASS\n'
