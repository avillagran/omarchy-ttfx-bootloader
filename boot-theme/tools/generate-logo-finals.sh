#!/bin/bash
# Build final-cells against the vendored engine and (re)generate or --check
# boot-theme/logo-finals.txt. Needs cargo, gcc and python3; run in a container.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
work=$(mktemp -d "${TMPDIR:-/var/tmp}/omarchy-logo-finals.XXXXXXXX")
(cd "$root/native/plymouth-ttfx-engine" && CARGO_TARGET_DIR="$work/target" cargo build --locked --offline --profile plymouth --quiet)
gcc -O2 -I"$root/native/plymouth-ttfx-engine/include" -I"$root/native/plymouth-ttfx-plugin" \
  "$root/boot-theme/tools/final-cells.c" "$work/target/plymouth/libttfx_plymouth.a" -lm -lpthread -ldl -o "$work/final-cells"
grep -v '^[[:space:]]*$' "$root/default/plymouth/ttfx-effects.txt" | "$work/final-cells" >"$work/cells.txt"
python3 -I "$root/boot-theme/tools/generate-logo-finals.py" "$work/cells.txt" "$root/boot-theme/logo-finals.txt" "$@"
printf 'work files retained in %s\n' "$work"
