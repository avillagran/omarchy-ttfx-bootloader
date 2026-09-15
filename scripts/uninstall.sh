#!/bin/bash
set -euo pipefail

state_dir=/var/lib/omarchy-ttfx-bootloader
command -v pacman >/dev/null || { echo "This uninstaller supports Arch-based Omarchy systems only." >&2; exit 1; }
[[ -d $state_dir ]] || { echo "No TTFX installer state found." >&2; exit 1; }

restore() {
  local path=$1 name=${1//\//_}
  if [[ -f $state_dir/$name.absent ]]; then
    rm -f -- "$path"
  elif [[ -e $state_dir/$name || -L $state_dir/$name ]]; then
    install -d -m 0755 "${path%/*}"
    cp -a --remove-destination "$state_dir/$name" "$path"
  else
    echo "Missing uninstall backup for $path" >&2
    exit 1
  fi
}

restore /usr/lib/plymouth/ttfx-plymouth.so
restore /usr/share/plymouth/themes/omarchy/omarchy.plymouth
restore /usr/share/omarchy/default/plymouth/omarchy.plymouth
plymouth-set-default-theme omarchy
if [[ -x /usr/bin/limine-mkinitcpio ]]; then
  /usr/bin/limine-mkinitcpio
else
  /usr/bin/mkinitcpio -P
fi
rm -f /usr/local/lib/omarchy-plymouth-ttfx-install
rm -rf "$state_dir"
echo "Uninstalled TTFX Plymouth. Reboot to test the restored boot theme."
