#!/bin/bash
set -euo pipefail

archive=${OMARCHY_TTFX_ARCHIVE:-https://github.com/avillagran/omarchy-ttfx-bootloader/archive/c4d3a8e5ae6039fb4f9acd3712d1e7a3b26e1e25.tar.gz}
expected_sha256=${OMARCHY_TTFX_SHA256:-9b5c7f2668bbb27498a4931ba39b8da914854849085e0cc3b908a3de6bf07972}
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
sudo "$tmp/scripts/install.sh"
