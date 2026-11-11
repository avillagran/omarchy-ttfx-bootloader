#!/bin/bash
set -euo pipefail

archive=${OMARCHY_TTFX_ARCHIVE:-https://github.com/avillagran/omarchy-ttfx-bootloader/archive/2c047b68ff4699371c7a6048ef6e28eb8f556bd5.tar.gz}
expected_sha256=${OMARCHY_TTFX_SHA256:-e45e10fccdc1353f44e229397d0a471c59b3da57c33a8f4a2c503df0d9fa8c21}
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/omarchy-ttfx-bootloader.XXXXXXXX")
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
sudo "$tmp/scripts/uninstall.sh"
