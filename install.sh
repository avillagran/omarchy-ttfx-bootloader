#!/bin/bash
set -euo pipefail

archive=${OMARCHY_TTFX_ARCHIVE:-https://github.com/avillagran/omarchy-ttfx-bootloader/archive/2eb89f55a4ea3db1b5629a85f466ed0a6b0755a6.tar.gz}
expected_sha256=${OMARCHY_TTFX_SHA256:-c3ac653cc2612a34002c4c9bf78019afed5ab7ff25ee326d8db907cf106ad831}
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
sudo "$tmp/scripts/install.sh"
