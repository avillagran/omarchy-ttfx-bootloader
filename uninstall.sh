#!/bin/bash
set -euo pipefail

archive=${OMARCHY_TTFX_ARCHIVE:-https://github.com/avillagran/omarchy-ttfx-bootloader/archive/0041528e216861f291e1b47c8f4a55a7d56113c6.tar.gz}
expected_sha256=${OMARCHY_TTFX_SHA256:-611e30e3dc1624a3d1954e04110cb270c2ced7e0bc8a9882c984952f9ccb089d}
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/omarchy-ttfx-bootloader.XXXXXXXX")
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
sudo "$tmp/scripts/uninstall.sh"
