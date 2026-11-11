#!/bin/bash
set -euo pipefail

archive=${OMARCHY_TTFX_ARCHIVE:-https://github.com/avillagran/omarchy-ttfx-bootloader/archive/77f248b66486c972e4db38938e8b62600a47ad3f.tar.gz}
expected_sha256=${OMARCHY_TTFX_SHA256:-def3155efdb05b8dcab5f5a0687500e3c02fd7e5b6cf09cff721018a02f58cb5}
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/omarchy-ttfx-bootloader.XXXXXXXX")
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
sudo "$tmp/scripts/uninstall.sh"
