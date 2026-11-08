#!/bin/bash
set -euo pipefail

archive=${OMARCHY_TTFX_ARCHIVE:-https://github.com/avillagran/omarchy-ttfx-bootloader/archive/2c07c737af293933ca7c1c2088e287a3d9e86e3e.tar.gz}
expected_sha256=${OMARCHY_TTFX_SHA256:-9b3c3cae73987d9a6e3737eb1fad0ba6065a07543c745e13b94e0f1f1899f42b}
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/omarchy-ttfx-bootloader.XXXXXXXX")
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
sudo "$tmp/scripts/uninstall.sh"
