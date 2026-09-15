#!/bin/bash
set -euo pipefail

repo=${OMARCHY_TTFX_REPO:-https://github.com/avillagran/omarchy-ttfx-bootloader}
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$repo/archive/refs/heads/main.tar.gz" | tar -xz -C "$tmp" --strip-components=1
exec "$tmp/scripts/install.sh"
