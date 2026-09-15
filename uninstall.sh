#!/bin/bash
set -euo pipefail

archive=https://github.com/avillagran/omarchy-ttfx-bootloader/archive/ba586cd14c81dcaf77fc8669ec1635e3a98d2fc7.tar.gz
expected_sha256=f2b177b8450b5ec410c64c443bfd628cd579979976527ad2767a5233664513e5
tmp=$(mktemp -d /tmp/omarchy-ttfx-bootloader.XXXXXXXX)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$archive" -o "$tmp/source.tar.gz"
printf '%s  %s\n' "$expected_sha256" "$tmp/source.tar.gz" | sha256sum -c -
tar -xzf "$tmp/source.tar.gz" -C "$tmp" --strip-components=1
"$tmp/scripts/uninstall.sh"
