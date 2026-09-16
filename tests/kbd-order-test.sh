#!/bin/bash
# The keyboard-order drop-in must place the keyboard hook immediately before
# plymouth, dedupe an existing keyboard entry, stay idempotent when sourced
# twice, and still append keyboard when no plymouth hook exists.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
conf=$root/default/mkinitcpio/zz-ttfx-kbd-order.conf

fail() {
  printf 'not ok - %s\n' "$1" >&2
  exit 1
}

hooks_string() {
  local IFS=' '
  printf '%s' "${HOOKS[*]}"
}

HOOKS=(base udev plymouth keyboard autodetect microcode modconf kms keymap consolefont block encrypt filesystems fsck btrfs-overlayfs resume)
# shellcheck disable=SC1090
source "$conf"
[[ $(hooks_string) == "base udev keyboard plymouth autodetect microcode modconf kms keymap consolefont block encrypt filesystems fsck btrfs-overlayfs resume" ]] ||
  fail "keyboard is not immediately before plymouth: $(hooks_string)"

# shellcheck disable=SC1090
source "$conf"
[[ $(hooks_string) == "base udev keyboard plymouth autodetect microcode modconf kms keymap consolefont block encrypt filesystems fsck btrfs-overlayfs resume" ]] ||
  fail "drop-in is not idempotent: $(hooks_string)"

HOOKS=(base keyboard udev plymouth encrypt filesystems)
# shellcheck disable=SC1090
source "$conf"
[[ $(hooks_string) == "base udev keyboard plymouth encrypt filesystems" ]] ||
  fail "pre-existing keyboard entry is not deduped: $(hooks_string)"

HOOKS=(base udev encrypt filesystems)
# shellcheck disable=SC1090
source "$conf"
[[ $(hooks_string) == "base udev encrypt filesystems keyboard" ]] ||
  fail "missing plymouth must still append keyboard: $(hooks_string)"

grep -q '/etc/mkinitcpio.conf.d/zz-ttfx-kbd-order.conf \\$' "$root/scripts/install.sh" ||
  fail "install.sh does not back up the drop-in"
grep -q 'install -m 0644 "$root/default/mkinitcpio/zz-ttfx-kbd-order.conf" /etc/mkinitcpio.conf.d/zz-ttfx-kbd-order.conf' "$root/scripts/install.sh" ||
  fail "install.sh does not install the drop-in"
grep -q 'restore_file /etc/mkinitcpio.conf.d/zz-ttfx-kbd-order.conf' "$root/scripts/uninstall.sh" ||
  fail "uninstall.sh does not restore the drop-in"
grep -q '/etc/systemd/system/plymouth-quit.service.d/20-ttfx-deactivate.conf \\$' "$root/scripts/install.sh" ||
  fail "install.sh does not back up the deactivate drop-in"
grep -q 'restore_file /etc/systemd/system/plymouth-quit.service.d/20-ttfx-deactivate.conf' "$root/scripts/uninstall.sh" ||
  fail "uninstall.sh does not restore the deactivate drop-in"
grep -q 'ExecStartPre=/usr/bin/timeout -k 1s 3s /usr/bin/plymouth deactivate' \
  "$root/default/systemd/plymouth-quit.service.d/20-ttfx-deactivate.conf" ||
  fail "deactivate drop-in lost its ExecStartPre"

printf 'keyboard hook order: PASS\n'
