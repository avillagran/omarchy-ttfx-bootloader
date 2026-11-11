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

grep -F 'sudo "$tmp/scripts/install.sh" "$@"' "$root/install.sh" >/dev/null || fail "oneliner drops --build/--prebuilt"
grep -F -- '--build) build_mode=true' "$root/scripts/install.sh" >/dev/null || fail "installer has no build mode"
grep -Fx 'build_mode=false' "$root/scripts/install.sh" >/dev/null || fail "prebuilt is not the default mode"
grep -F 'sha256sum -c --quiet -' "$root/scripts/install.sh" >/dev/null || fail "prebuilt module is not hash-verified"
grep -F 'read_prebuilt_manifest "$root/prebuilt/manifest"' "$root/scripts/install.sh" >/dev/null || fail "prebuilt pin is not read from the archive"
if grep -E '^[[:space:]]*(source|\.)[[:space:]].*manifest' "$root/scripts/install.sh" "$root/scripts/common.sh" >/dev/null; then
  fail "prebuilt manifest is executed"
fi
pass "oneliner defaults to the pinned prebuilt module and keeps --build"

grep -Fx '#define TTFX_PLAYBACK_STEPS_PER_TICK 2U' "$root/native/plymouth-ttfx-plugin/state.h" >/dev/null || fail "native state is not 2x"
grep -Fx '#define ENGINE_STEPS_PER_TICK 2U' "$root/native/plymouth-ttfx-plugin/plugin.c" >/dev/null || fail "handoff contract is not 2x"
grep -Fx 'const BOOT_STEPS_PER_TICK: usize = 2;' "$root/bridge/src/main.rs" >/dev/null || fail "desktop bridge is not 2x"
pass "native and desktop handoff agree on 2x"

for file in omarchy-boot-theme omarchy-boot-theme-apply logo-finals.txt grub.d/09_omarchy-boot-theme grub.d/10_omarchy-boot-theme-end; do
  grep -F "\"\$boot_theme_share/$file\"" "$root/scripts/common.sh" >/dev/null || fail "boot theme file $file is not managed"
  grep -F "$file" "$root/scripts/install.sh" >/dev/null || fail "boot theme file $file is not installed"
done
grep -F 'if ! "$boot_theme_apply"; then' "$root/scripts/install.sh" >/dev/null || fail "installer does not apply the boot theme non-fatally"
grep -F '"$boot_theme_apply" --remove' "$root/scripts/uninstall.sh" >/dev/null || fail "uninstaller does not remove the boot theme"
pass "bootloader theme is installed, applied non-fatally and removed"

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
grep -F "printf '[Daemon]\\nTheme=%s\\n'" "$root/bin/omarchy-plymouth-ttfx-install" >/dev/null || fail "empty Plymouth configuration is not repaired"
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

python3 - "$root/default/plymouth/boot-mark.png" <<'PY'
import struct, sys
data = open(sys.argv[1], "rb").read()
assert data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR"
width, height, depth, color = struct.unpack(">IIBB", data[16:26])
assert (width, height, depth, color) == (300, 300, 8, 6), (width, height, depth, color)
PY
managed=$(bash -c 'source "$1"; managed_files' bash "$root/scripts/common.sh")
for directory in /usr/share/plymouth/themes/omarchy /usr/share/omarchy/default/plymouth; do
  grep -Fx "$directory/boot-mark.png" <<<"$managed" >/dev/null || fail "managed files miss $directory/boot-mark.png"
  grep -Fx "install -m 0644 \"\$root/default/plymouth/boot-mark.png\" $directory/boot-mark.png" "$root/scripts/install.sh" >/dev/null ||
    fail "installer does not install boot-mark.png into $directory"
done
# Branding refreshes publish only their listed assets and leave the rest of the
# theme in place. The mark is theme-independent and absent from Omarchy source
# trees that predate it, so it stays out of those lists.
if grep -F 'boot-mark' "$root/bin/omarchy-plymouth-set" >/dev/null; then
  fail "branding refresh takes over the theme-independent firmware-hold mark"
fi
pass "firmware-hold mark is a managed theme file that branding leaves alone"

grep -F '(( BASHPID == $$ )) || exit "$status"' "$root/scripts/install.sh" >/dev/null ||
  fail "a failing subshell can roll the install back behind the installer's back"
grep -F "find /boot/EFI/Linux -maxdepth 1 -type f -name '*.efi' -print0 2>/dev/null" "$root/scripts/install.sh" >/dev/null &&
  fail "a missing UKI directory fails inside the installer"
pass "only the installer itself rolls back"

# The unprivileged build copies only build_inputs; that copy must pass the
# build's own embedded-logo freshness check.
inputs_line=$(grep -E '^[[:space:]]*build_inputs=\(' "$root/scripts/install.sh") || fail "installer declares no build inputs"
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
