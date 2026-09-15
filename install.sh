#!/bin/bash
set -euo pipefail

archive=https://github.com/avillagran/omarchy-ttfx-bootloader/archive/679ff5bca5e7833b735cbd55022fc4417f7de9af.tar.gz
expected_sha256=ec0e3462c920965fac2c92704b28bc3c706a565453d468f8699e89d75670b1f6
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
"$tmp/scripts/install.sh"
