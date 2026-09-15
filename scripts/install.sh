#!/bin/bash
set -eEuo pipefail

[[ $EUID == 0 ]] || { echo "Run this script through sudo." >&2; exit 1; }
export PATH=/usr/bin:/usr/sbin

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
state_dir=/var/lib/omarchy-ttfx-bootloader
helper=/usr/local/lib/omarchy-plymouth-ttfx-install
arch=$(uname -m)
new_install=false
committed=false

case "$arch" in
  x86_64|aarch64) ;;
  *) echo "Unsupported architecture: $arch (supported: x86_64, aarch64)" >&2; exit 1 ;;
esac

command -v pacman >/dev/null || { echo "This installer supports Arch-based Omarchy systems only." >&2; exit 1; }

backup_file() {
  local path=$1 name=${1//\//_}
  if [[ -e $path || -L $path ]]; then
    cp -a --no-dereference "$path" "$state_dir/$name"
  else
    : > "$state_dir/$name.absent"
  fi
}

restore_file() {
  local path=$1 name=${1//\//_}
  if [[ -f $state_dir/$name.absent ]]; then
    rm -f -- "$path"
  elif [[ -e $state_dir/$name || -L $state_dir/$name ]]; then
    install -d -m 0755 "${path%/*}"
    cp -a --remove-destination "$state_dir/$name" "$path"
  fi
}

restore_initial_state() {
  if $new_install && ! $committed; then
    restore_file /usr/lib/plymouth/ttfx-plymouth.so
    restore_file /usr/share/plymouth/themes/omarchy/omarchy.plymouth
    restore_file /usr/share/omarchy/default/plymouth/omarchy.plymouth
    restore_file "$helper"
    if [[ -f $state_dir/omarchy-static.absent ]]; then
      rm -rf -- /usr/share/plymouth/themes/omarchy-static
    elif [[ -d $state_dir/omarchy-static ]]; then
      rm -rf -- /usr/share/plymouth/themes/omarchy-static
      cp -a --no-dereference "$state_dir/omarchy-static" /usr/share/plymouth/themes/omarchy-static
    fi
  fi
}
on_failure() {
  local status=$?
  trap - ERR HUP INT TERM
  restore_initial_state || true
  exit "$status"
}
trap on_failure ERR HUP INT TERM

missing=()
for package in base-devel rust pkgconf python plymouth; do
  pacman -Q "$package" >/dev/null 2>&1 || missing+=("$package")
done
if (( ${#missing[@]} )); then
  # This installs only missing build prerequisites. It deliberately does not
  # synchronize or upgrade the system; Omarchy owns that through `omarchy update`.
  if ! pacman -S --needed --noconfirm "${missing[@]}"; then
    printf 'Could not install prerequisites: %s\nRun `omarchy update` and retry.\n' "${missing[*]}" >&2
    exit 1
  fi
fi
install -d -m 0700 "$state_dir"

if [[ ! -f $state_dir/.installed ]]; then
  new_install=true
  backup_file /usr/lib/plymouth/ttfx-plymouth.so
  backup_file /usr/share/plymouth/themes/omarchy/omarchy.plymouth
  backup_file /usr/share/omarchy/default/plymouth/omarchy.plymouth
  backup_file "$helper"
  if [[ -e /usr/share/plymouth/themes/omarchy-static || -L /usr/share/plymouth/themes/omarchy-static ]]; then
    cp -a --no-dereference /usr/share/plymouth/themes/omarchy-static "$state_dir/omarchy-static"
  else
    : > "$state_dir/omarchy-static.absent"
  fi
  plymouth-set-default-theme > "$state_dir/previous-theme"
fi

module="$state_dir/ttfx-plymouth-$arch.so"
"$root/bin/omarchy-plymouth-ttfx-build" build --output "$module"
install -m 0755 "$root/bin/omarchy-plymouth-ttfx-install" "$helper"
install -m 0644 "$root/default/plymouth/omarchy.plymouth" /usr/share/omarchy/default/plymouth/omarchy.plymouth
"$helper" --module "$module"
: > "$state_dir/.installed"
committed=true
trap - ERR HUP INT TERM
echo "Installed TTFX Plymouth for $arch. Reboot to test it."
