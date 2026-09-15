#!/bin/bash
set -euo pipefail

archive=https://github.com/avillagran/omarchy-ttfx-bootloader/archive/fd8af974e762279fb544cc495bdddd9df1363861.tar.gz
expected_sha256=b02c2e7f2a11e761156b2393c46c068e0fb601af0c2036f70b3cb6d5f0fd54a1
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
"$tmp/scripts/install.sh"
