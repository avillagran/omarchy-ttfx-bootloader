#!/bin/bash
set -euo pipefail

archive=https://github.com/avillagran/omarchy-ttfx-bootloader/archive/f17d59642b3dec06eca8edc34c7ea7c72887bb9f.tar.gz
expected_sha256=dab670a6c7bf509fe06d6b75be123e558a0f46d9b2b36339b861fb39d2f47407
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
"$tmp/scripts/install.sh"
