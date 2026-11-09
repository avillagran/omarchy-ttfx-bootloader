#!/bin/bash
set -eEuo pipefail

[[ $EUID == 0 ]] || { echo "Run this script through sudo." >&2; exit 1; }
export PATH=/usr/share/omarchy/bin:/usr/local/bin:/usr/bin:/usr/sbin

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
usage() {
  cat <<'EOF'
Usage: install.sh [--prebuilt | --build] [--prebuilt-base URL]
  --prebuilt         install the release module pinned in prebuilt/manifest (default)
  --build            compile the module on this machine (installs Rust if needed)
  --prebuilt-base    fetch the pinned module from another URL (file:// works); the
                     sha256 pin still applies
EOF
}
build_mode=false
prebuilt_base=
while (( $# )); do
  case "$1" in
    --prebuilt) build_mode=false ;;
    --build) build_mode=true ;;
    --prebuilt-base) [[ -n ${2:-} ]] || { usage >&2; exit 2; }; prebuilt_base=$2; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done
# shellcheck source=scripts/common.sh
source "$root/scripts/common.sh"
acquire_installer_lock
arch=$(uname -m)
new_install=false
boot_changed=false
committed=false

case "$arch" in
  x86_64|aarch64) ;;
  *) echo "Unsupported architecture: $arch (supported: x86_64, aarch64)" >&2; exit 1 ;;
esac

command -v pacman >/dev/null || { echo "This installer supports Arch-based Omarchy systems only." >&2; exit 1; }
[[ -d /usr/share/omarchy/default/plymouth && -f /usr/share/plymouth/themes/omarchy/omarchy.plymouth ]] ||
  { echo "Omarchy's Plymouth theme is not installed." >&2; exit 1; }
invoking_user=${SUDO_USER:-}
[[ -n $invoking_user && $invoking_user != root ]] || { echo "Run with: curl ... | bash (it enters sudo itself)" >&2; exit 1; }
user_home=$(getent passwd "$invoking_user" | cut -d: -f6)
[[ -n $user_home && -d $user_home ]] || { echo "Could not resolve the invoking user's home." >&2; exit 1; }
user_group=$(id -gn "$invoking_user")
menu_extension="$user_home/.config/omarchy/extensions/omarchy-menu.jsonc"

restore_initial_state() {
  if ! $committed; then
    rollback_install_transaction
  fi
}

on_failure() {
  local status=$?
  # errtrace hands this trap to every subshell, process substitutions
  # included; only the installer itself may roll back.
  (( BASHPID == $$ )) || exit "$status"
  trap - ERR HUP INT TERM EXIT
  if ! restore_initial_state; then
    printf 'TTFX rollback was incomplete; inspect %s before rebooting.\n' "$transaction_dir" >&2
  fi
  exit "$status"
}
# EXIT also covers explicit `exit 1` checks after the boot state changed.
trap on_failure ERR HUP INT TERM EXIT

# Prerequisites shared by both modes: the native helper inspects the module
# with file, nm, readelf and ldd, and checks the ABI with pkgconf.
missing=()
required=(plymouth pkgconf python file binutils)
$build_mode && required+=(base-devel)
for package in "${required[@]}"; do
  pacman -Q "$package" >/dev/null 2>&1 || missing+=("$package")
done

# Build mode compiles with root's cargo, else with the invoking user's: rustup
# lives in ~/.cargo/bin, and Arch's /usr/bin/cargo can be a rustup shim with
# no default toolchain for root. Install Arch's rust only when neither works.
user_path="$user_home/.cargo/bin:$user_home/.local/share/mise/shims:$PATH"
user_cargo() {
  runuser -u "$invoking_user" -- env HOME="$user_home" PATH="$user_path" cargo "$@"
}
if $build_mode && ! cargo --version >/dev/null 2>&1 && ! user_cargo --version >/dev/null 2>&1; then
  missing+=(rust)
fi
if (( ${#missing[@]} )); then
  if ! pacman -S --needed --noconfirm "${missing[@]}"; then
    printf 'Could not install prerequisites: %s\nRun `omarchy update` and retry.\n' "${missing[*]}" >&2
    exit 1
  fi
fi
install -d -m 0700 "$state_dir"
module="$state_dir/ttfx-plymouth-$arch.so"

if $build_mode; then
  build_inputs=(bin native default logo.txt)
  build_user_dir=
  if ! cargo --version >/dev/null 2>&1; then
    if user_cargo --version >/dev/null 2>&1; then
      build_user_dir=$(mktemp -d "${TMPDIR:-/var/tmp}/omarchy-ttfx-build.XXXXXXXX")
      chown "$invoking_user:$user_group" "$build_user_dir"
      install -d -m 0755 "$build_user_dir/source"
      # Every input the build reads, including logo.txt for the embedded-logo check.
      for input in "${build_inputs[@]}"; do
        cp -a -- "$root/$input" "$build_user_dir/source/"
      done
      chown -hR "$invoking_user:$user_group" "$build_user_dir/source"
    else
      printf 'No working cargo for root or %s. Install Rust (pacman -S rust) and retry.\n' "$invoking_user" >&2
      exit 1
    fi
  fi

  if [[ -n $build_user_dir ]]; then
    runuser -u "$invoking_user" -- env HOME="$user_home" PATH="$user_path" "$build_user_dir/source/bin/omarchy-plymouth-ttfx-build" build \
      --output "$build_user_dir/ttfx-plymouth-$arch.so"
    install -o root -g root -m 0755 "$build_user_dir/ttfx-plymouth-$arch.so" "$module"
    printf 'Build source retained at %s\n' "$build_user_dir"
  else
    "$root/bin/omarchy-plymouth-ttfx-build" build --output "$module"
  fi
  module_origin="built on this machine"
else
  # The release module is pinned by sha256 in prebuilt/manifest, which the
  # one-liner already verified as part of the source archive.
  read_prebuilt_manifest "$root/prebuilt/manifest"
  installed_plymouth=$(pacman -Q plymouth)
  installed_plymouth=${installed_plymouth#plymouth }
  if [[ $installed_plymouth != "$prebuilt_abi" && $installed_plymouth != "$prebuilt_abi-"* ]]; then
    printf 'The prebuilt module targets Plymouth %s, this system has %s.\n' "$prebuilt_abi" "$installed_plymouth" >&2
    echo 'Retry with --build to compile it here (curl ... | bash -s -- --build).' >&2
    exit 1
  fi
  [[ -n ${prebuilt_sha256[$arch]:-} ]] || { echo "No prebuilt module for $arch; retry with --build." >&2; exit 1; }
  download=$(mktemp "$state_dir/.ttfx-plymouth-$arch.XXXXXXXX")
  if ! curl -fsSL "${prebuilt_base:-$prebuilt_release}/ttfx-plymouth-$arch.so" -o "$download"; then
    rm -f -- "$download"
    echo "Could not download the prebuilt module; retry later or with --build." >&2
    exit 1
  fi
  if ! printf '%s  %s\n' "${prebuilt_sha256[$arch]}" "$download" | sha256sum -c --quiet -; then
    rm -f -- "$download"
    echo "The prebuilt module does not match its pinned sha256; nothing was installed." >&2
    exit 1
  fi
  chown root:root "$download"
  chmod 0755 "$download"
  mv -fT -- "$download" "$module"
  module_origin="prebuilt for Plymouth $prebuilt_abi"
fi

# Build first; hold the common Plymouth lock only while snapshotting and
# publishing, so rollback cannot overwrite a concurrent successful selection.
acquire_plymouth_lock
begin_install_transaction

# A failed first attempt leaves its backups and previous theme behind; they
# still describe the system before this installer touched it.
if [[ ! -f $state_dir/.installed ]]; then
  new_install=true
  [[ -s $state_dir/previous-theme ]] || plymouth-set-default-theme >"$state_dir/previous-theme"
fi
[[ -f $state_dir/invoking-user ]] || printf '%s\n' "$invoking_user" >"$state_dir/invoking-user"
[[ -f $state_dir/user-home ]] || printf '%s\n' "$user_home" >"$state_dir/user-home"

# Keep the original of each path across reinstalls, unless something other
# than this installer replaced it since, such as an omarchy-settings update:
# that newer file is the one an uninstall must give back.
legacy_state=false
if ! $new_install && [[ ! -f $installed_hashes ]]; then
  legacy_state=true
fi
while IFS= read -r path; do
  if ! has_backup "$path"; then
    backup_file "$path"
  elif $new_install; then
    : # left by a failed first attempt, taken before anything changed
  elif $legacy_state; then
    # The 2026-09 installer recorded no hashes. Its package-file backups are
    # stale once a package update put stock files back.
    if is_descriptor "$path" && ! ours_now "$path"; then
      backup_file "$path"
    fi
  elif ! ours_now "$path"; then
    backup_file "$path"
  fi
done < <(managed_files)
if [[ ! -e $state_dir/omarchy-static && ! -e $state_dir/omarchy-static.absent ]]; then
  if [[ -e /usr/share/plymouth/themes/omarchy-static || -L /usr/share/plymouth/themes/omarchy-static ]]; then
    cp -a --no-dereference /usr/share/plymouth/themes/omarchy-static "$state_dir/omarchy-static"
  else
    : >"$state_dir/omarchy-static.absent"
  fi
fi

install -m 0755 "$root/bin/omarchy-plymouth-ttfx-install" "$helper"
install -m 0644 "$root/default/plymouth/omarchy.plymouth" /usr/share/omarchy/default/plymouth/omarchy.plymouth
install -m 0644 "$root/default/plymouth/omarchy.script" /usr/share/omarchy/default/plymouth/omarchy.script
install -m 0644 "$root/default/plymouth/omarchy.script" /usr/share/plymouth/themes/omarchy/omarchy.script
install -m 0644 "$root/default/plymouth/ttfx-effects.txt" /usr/share/omarchy/default/plymouth/ttfx-effects.txt
install -m 0644 "$root/default/plymouth/ttfx-effects.txt" /usr/share/plymouth/themes/omarchy/ttfx-effects.txt
install -m 0644 "$root/default/mkinitcpio/zz-ttfx-kbd-order.conf" /etc/mkinitcpio.conf.d/zz-ttfx-kbd-order.conf
install -d -m 0755 /etc/systemd/system/plymouth-quit.service.d
install -m 0644 "$root/default/systemd/system/plymouth-quit.service.d/10-omarchy-retain-splash.conf" \
  /etc/systemd/system/plymouth-quit.service.d/10-omarchy-retain-splash.conf
install -m 0644 "$root/default/systemd/system/omarchy-plymouth-reap.service" /etc/systemd/system/omarchy-plymouth-reap.service
install -m 0644 "$root/default/systemd/system/omarchy-plymouth-reap.timer" /etc/systemd/system/omarchy-plymouth-reap.timer
# The 2026-09 drop-in's deactivate now runs inside 10-omarchy-retain-splash.conf.
rm -f -- /etc/systemd/system/plymouth-quit.service.d/20-ttfx-deactivate.conf
systemctl daemon-reload
boot_changed=true
"$helper" --module "$module"

for directory in "${runtime_dirs[@]}"; do
  install -d -m 0755 "$directory"
  for command in "${runtime_commands[@]}"; do
    install -m 0755 "$root/bin/$command" "$directory/$command"
  done
done

current=$(PATH=/usr/share/omarchy/bin:/usr/local/bin:/usr/bin omarchy-plymouth-effect-set current)
[[ $current != off ]] || { echo "Installed effect getter still resolves to Off." >&2; exit 1; }
! grep -F 'add_option "Off"' /usr/share/omarchy/bin/omarchy-plymouth-effect-switcher >/dev/null || {
  echo "Installed selector still contains Off." >&2
  exit 1
}
plymouth-set-default-theme | grep -Fx omarchy >/dev/null
# The native helper verifies exactly the boot images it rebuilt. Other
# distributions and hand-kept UKIs must remain outside this transaction.

record_installed_hashes
: >"$state_dir/.installed"
committed=true
trap - ERR HUP INT TERM EXIT

# The menu extension belongs to the user and to the other tools that mark
# blocks in it, so it is edited as that user and only inside our markers. A
# menu that cannot be edited safely leaves the boot install in place.
menu_status="added to Menu > Appearance > Bootloader"
if ! runuser -u "$invoking_user" -- python3 "$root/scripts/menu-extension.py" add \
  "$menu_extension" "$root/default/omarchy/ttfx-menu.jsonc"; then
  menu_status="not added to the menu (see the message above); the boot animation is installed"
fi

printf 'Pre-attempt backup retained at %s\n' "$transaction_dir"
printf 'Installed TTFX Plymouth for %s (%s, 2x, effect=%s), %s. Reboot to test it.\n' "$arch" "$module_origin" "$current" "$menu_status"
