#!/bin/bash
set -euo pipefail

archive=${OMARCHY_TTFX_ARCHIVE:-https://github.com/avillagran/omarchy-ttfx-bootloader/archive/430fafeedbc5f2c9504e7fbcfdb568eed901a484.tar.gz}
expected_sha256=${OMARCHY_TTFX_SHA256:-d534608f074f623c5240071db387367d7f542e569d78f0372282452452f01f77}
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
sudo "$tmp/scripts/uninstall.sh"
