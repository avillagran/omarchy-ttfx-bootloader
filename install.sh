#!/bin/bash
set -euo pipefail

archive=https://github.com/avillagran/omarchy-ttfx-bootloader/archive/060973a28d29adcbb2a47156bbed707ebfde59e4.tar.gz
expected_sha256=c99fa3737d5786ce97c0534d34c20c2edac1134bec9162486a0099c03815e8ff
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
"$tmp/scripts/install.sh"
