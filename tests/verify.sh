#!/bin/bash
# Rootless native and installer verification. Run on the target architecture.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
export TMPDIR=${TMPDIR:-/var/tmp}
cd "$root"
for command in cargo gcc python3 node strace pkg-config; do
  command -v "$command" >/dev/null
done
pkg-config --exists ply-splash-core ply-splash-graphics
printf 'Verification host: %s (%s)\n' "$(hostname)" "$(uname -m)"
for test in installer-static install-transaction-test prebuilt-manifest-test kbd-order-test menu-extension-test \
  shell.d/plymouth-effect-switcher-test shell.d/plymouth-native-install-test \
  shell.d/plymouth-native-package-test; do
  bash "tests/$test.sh"
done
(
  cd native/plymouth-ttfx-engine
  cargo test --locked --offline
  cargo test --locked --offline -p ttfx --lib
  cargo clippy --locked --offline --all-targets -- -D warnings
  python3 tests/vendor-provenance.py
  bash tests/self-contained.sh
  bash tests/embedded-smoke.sh
)
bash native/plymouth-ttfx-plugin/tests/run.sh
artifacts=$(mktemp -d "$TMPDIR/ttfx-verified-module.XXXXXXXX")
bash bin/omarchy-plymouth-ttfx-build build --output "$artifacts/ttfx-plymouth.so"
bash bin/omarchy-plymouth-ttfx-build stage --module "$artifacts/ttfx-plymouth.so" --destdir "$artifacts/stage"
cmp "$artifacts/ttfx-plymouth.so" "$artifacts/stage/usr/lib/plymouth/ttfx-plymouth.so"
printf 'Verified module and staging: %s\n' "$artifacts"
printf 'Rootless verification: PASS\n'
