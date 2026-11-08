#!/bin/bash
# Exercise upgrade rollback with independent pre-attempt and uninstall backups.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
source "$root/scripts/common.sh"
fixture=$(mktemp -d "${TMPDIR:-/var/tmp}/ttfx-upgrade-test.XXXXXXXX")
printf 'Test fixtures: %s\n' "$fixture"
state_dir=$fixture/uninstall-backups
recovery_theme_dir=$fixture/recovery-theme
mkdir -p "$state_dir" "$recovery_theme_dir" "$fixture/current"
printf stock >"$state_dir/stock-sentinel"
printf installed-v1 >"$fixture/current/module"
printf custom >"$recovery_theme_dir/keep"
ln -s keep "$recovery_theme_dir/keep-link"
managed_files() { printf '%s\n' "$fixture/current/module" "$fixture/current/new-file"; }
plymouth-set-default-theme() {
  if (( $# )); then printf '%s\n' "$1" >"$fixture/theme"; else cat "$fixture/theme"; fi
}
systemctl() { [[ $1 == daemon-reload ]]; }
rebuild_boot_images() { printf '%s\n' "$(cat "$fixture/current/module")" >"$fixture/rebuilt"; }
printf omarchy >"$fixture/theme"
boot_changed=true
begin_install_transaction
printf installed-v2 >"$fixture/current/module"
printf new >"$fixture/current/new-file"
printf changed >"$recovery_theme_dir/keep"
printf other >"$fixture/theme"
rollback_install_transaction
[[ $(cat "$fixture/current/module") == installed-v1 ]]
[[ ! -e $fixture/current/new-file ]]
[[ $(cat "$recovery_theme_dir/keep") == custom ]]
[[ -L $recovery_theme_dir/keep-link ]]
[[ $(readlink "$recovery_theme_dir/keep-link") == keep ]]
[[ $(cat "$fixture/rebuilt") == installed-v1 ]]
[[ $(cat "$fixture/theme") == omarchy ]]
[[ $(cat "$state_dir/stock-sentinel") == stock ]]
printf 'upgrade transaction restores the pre-attempt state and preserves uninstall backups: PASS\n'

# A failed restore must not suppress the remaining recovery operations.
begin_install_transaction
restore_file() { [[ $1 != "$fixture/current/module" ]]; }
if rollback_install_transaction; then
  echo 'failed restore unexpectedly reported success' >&2
  exit 1
fi
[[ $(cat "$fixture/rebuilt") == installed-v1 ]]
printf 'rollback reports failure after attempting boot recovery: PASS\n'

(
  acquire_installer_lock "$fixture/install.lock"
  if bash -c 'source "$1"; acquire_installer_lock "$2"' bash "$root/scripts/common.sh" "$fixture/install.lock"; then
    echo 'concurrent installer unexpectedly acquired the lock' >&2
    exit 1
  fi
)
# Releasing the first lock allows a later invocation to proceed.
(bash -c 'source "$1"; acquire_installer_lock "$2"' bash "$root/scripts/common.sh" "$fixture/install.lock")
printf 'install and uninstall are serialized: PASS\n'

# Quattro ships [ttfx] defaults even when the stock script module is active.
# A package refresh must therefore no longer be mistaken for our descriptor.
stock=$fixture/stock.plymouth
printf '[Plymouth Theme]\nModuleName=script\n[ttfx]\nEffect=decrypt\n' >"$stock"
is_live_descriptor() { [[ $1 == "$stock" ]]; }
if ours_now "$stock"; then
  echo 'stock quattro descriptor incorrectly claimed as installed native state' >&2
  exit 1
fi
sed -i 's/ModuleName=script/ModuleName=ttfx-plymouth/' "$stock"
ours_now "$stock"
printf 'package script fallback is distinguished from installed native state: PASS\n'

# An incomplete snapshot cannot retire or restore any live recovery theme.
transaction_ready=false
plymouth-set-default-theme() { return 1; }
if begin_install_transaction; then
  echo 'failed snapshot unexpectedly reported success' >&2
  exit 1
fi
rollback_install_transaction
[[ $(cat "$recovery_theme_dir/keep") == custom ]]
printf 'incomplete snapshot leaves live state untouched: PASS\n'

# The untouched source template is owned by its installed hash, even though
# its safe default uses ModuleName=script.
installed_hashes=$fixture/installed.sha256
default_template=$fixture/default.plymouth
printf '[Plymouth Theme]\nModuleName=script\n[ttfx]\nEffect=decrypt\n' >"$default_template"
sha256sum "$default_template" >"$installed_hashes"
ours_now "$default_template"
printf '# package update\n' >>"$default_template"
if ours_now "$default_template"; then
  echo 'changed package default template incorrectly claimed as ours' >&2
  exit 1
fi
printf 'default template ownership uses its exact installed hash: PASS\n'
