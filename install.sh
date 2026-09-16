#!/bin/bash
set -euo pipefail

archive=${OMARCHY_TTFX_ARCHIVE:-https://github.com/avillagran/omarchy-ttfx-bootloader/archive/032377e.tar.gz}
expected_sha256=${OMARCHY_TTFX_SHA256:-9b20d9a460c5825d9fc14a8a74d808b89be09a336439885a531efceb79030c91}
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
sudo "$tmp/scripts/install.sh"
