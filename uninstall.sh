#!/bin/bash
set -euo pipefail

repo=https://github.com/avillagran/omarchy-ttfx-bootloader
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$repo/archive/refs/heads/main.tar.gz" | tar -xz -C "$tmp" --strip-components=1
"$tmp/scripts/uninstall.sh"
