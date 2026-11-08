#!/bin/bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

fail() { printf 'not ok - %s\n' "$1" >&2; exit 1; }
pass() { printf 'ok - %s\n' "$1"; }

for script in install.sh uninstall.sh scripts/common.sh scripts/install.sh scripts/uninstall.sh bin/omarchy-plymouth-*; do
  bash -n "$root/$script"
done
pass "shell scripts parse"

grep -F 'sudo "$tmp/scripts/install.sh"' "$root/install.sh" >/dev/null || fail "oneliner does not enter the privileged installer"
grep -F 'OMARCHY_TTFX_ARCHIVE' "$root/install.sh" >/dev/null || fail "oneliner cannot use a verified staging archive"
grep -F 'OMARCHY_TTFX_SHA256' "$root/install.sh" >/dev/null || fail "oneliner staging archive cannot be hash-pinned"
pass "oneliner enters sudo and supports hash-pinned staging"

grep -Fx '#define TTFX_PLAYBACK_STEPS_PER_TICK 2U' "$root/native/plymouth-ttfx-plugin/state.h" >/dev/null || fail "native state is not 2x"
grep -Fx '#define ENGINE_STEPS_PER_TICK 2U' "$root/native/plymouth-ttfx-plugin/plugin.c" >/dev/null || fail "handoff contract is not 2x"
grep -Fx 'const BOOT_STEPS_PER_TICK: usize = 2;' "$root/bridge/src/main.rs" >/dev/null || fail "desktop bridge is not 2x"
pass "native and desktop handoff agree on 2x"

if grep -F 'add_option "Off"' "$root/bin/omarchy-plymouth-effect-switcher" >/dev/null; then
  fail "effect selector exposes Off"
fi
grep -F '[[ $current == off ]] && current=decrypt' "$root/bin/omarchy-plymouth-effect-switcher" >/dev/null || fail "legacy Off does not migrate visually to Decrypt"
grep -Fx 'Effect=decrypt' "$root/default/plymouth/omarchy.plymouth" >/dev/null || fail "descriptor default is not Decrypt"
pass "selector omits Off and defaults to Decrypt"

for directory in /usr/share/omarchy/bin /usr/local/bin /usr/bin; do
  grep -F "$directory" "$root/scripts/install.sh" >/dev/null || fail "installer misses runtime command path $directory"
done
grep -F '"$helper" --module "$module"' "$root/scripts/install.sh" >/dev/null || fail "installer does not invoke transactional native installer"
if grep -F -- '--force-enable' "$root/scripts/install.sh" >/dev/null; then
  fail "standalone installer can overwrite a valid effect selection"
fi
grep -F 'legacy Off state all normalize' "$root/bin/omarchy-plymouth-ttfx-install" >/dev/null || fail "legacy Off does not normalize to Decrypt"
grep -F 'chmod 0644 "$module_stage"' "$root/bin/omarchy-plymouth-ttfx-install" >/dev/null && fail "installer makes Plymouth module non-executable"
grep -F "printf '[Daemon]\\n'" "$root/bin/omarchy-plymouth-ttfx-install" >/dev/null || fail "empty Plymouth configuration is not repaired"
pass "installer covers Omarchy runtime paths and Plymouth 26 edge cases"

python3 - "$root/default/omarchy/ttfx-menu.jsonc" <<'PY'
import json, re, sys
text = open(sys.argv[1]).read()
entries = json.loads(re.sub(r",(\s*})", r"\1", "{" + text + "}"))
assert set(entries) == {
    "appearance.bootloader",
    "appearance.bootloader.full-animated",
    "appearance.bootloader.input-only",
    "appearance.bootloader.effect",
}
assert all(line.startswith("  ") and line.endswith(",") for line in text.splitlines())
PY
pass "menu block is valid and complete"

grep -F 'runuser -u "$invoking_user" -- python3 "$root/scripts/menu-extension.py" add' "$root/scripts/install.sh" >/dev/null ||
  fail "installer does not edit the menu extension as the invoking user"
grep -F 'runuser -u "$invoking_user" -- python3 "$root/scripts/menu-extension.py" remove' "$root/scripts/uninstall.sh" >/dev/null ||
  fail "uninstaller does not edit the menu extension as the invoking user"
if grep -F 'menu_extension' "$root/scripts/common.sh" >/dev/null; then
  fail "the menu extension is restored from a backup instead of edited in place"
fi
if grep -F 'audio-background' "$root/scripts/install.sh" "$root/scripts/common.sh" >/dev/null; then
  fail "installer overwrites the audio-background plugin's own renderer"
fi
pass "menu is edited in place as the user and the audio-background plugin is left alone"

grep -F 'ours_now "$path"' "$root/scripts/uninstall.sh" >/dev/null ||
  fail "uninstall restores files that changed since the install"
grep -F 'record_installed_hashes' "$root/scripts/install.sh" >/dev/null || fail "install records what it wrote"
grep -Fx 'systemctl daemon-reload' "$root/scripts/uninstall.sh" >/dev/null || fail "uninstall leaves systemd with removed units"
pass "uninstall gives back only files still as the install left them"

grep -F '(( BASHPID == $$ )) || exit "$status"' "$root/scripts/install.sh" >/dev/null ||
  fail "a failing subshell can roll the install back behind the installer's back"
grep -F "find /boot/EFI/Linux -maxdepth 1 -type f -name '*.efi' -print0 2>/dev/null" "$root/scripts/install.sh" >/dev/null &&
  fail "a missing UKI directory fails inside the installer"
pass "only the installer itself rolls back"

# The unprivileged build copies only build_inputs; that copy must pass the
# build's own embedded-logo freshness check.
inputs_line=$(grep -E '^build_inputs=\(' "$root/scripts/install.sh") || fail "installer declares no build inputs"
eval "$inputs_line"
build_copy=$(mktemp -d "${TMPDIR:-/var/tmp}/ttfx-build-inputs.XXXXXXXX")
for input in "${build_inputs[@]}"; do
  cp -a -- "$root/$input" "$build_copy/"
done
python3 "$build_copy/native/plymouth-ttfx-plugin/generate-embedded-logo.py" --check ||
  fail "user build copy misses inputs of the embedded-logo check (retained at $build_copy)"
grep -F 'trap on_failure ERR HUP INT TERM EXIT' "$root/scripts/install.sh" >/dev/null ||
  fail "explicit exits after boot changes skip rollback"
pass "user build copy is complete and every exit before commit rolls back"
