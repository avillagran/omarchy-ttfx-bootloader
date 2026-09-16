#!/bin/bash
set -euo pipefail

[[ $EUID == 0 ]] || { echo "Run this script through sudo." >&2; exit 1; }
export PATH=/usr/share/omarchy/bin:/usr/local/bin:/usr/bin:/usr/sbin

state_dir=/var/lib/omarchy-ttfx-bootloader
helper=/usr/local/lib/omarchy-plymouth-ttfx-install
[[ -f $state_dir/.installed ]] || { echo "No TTFX installer state found." >&2; exit 1; }
arch=$(uname -m)
user_home=$(<"$state_dir/user-home")
menu_extension="$user_home/.config/omarchy/extensions/omarchy-menu.jsonc"
audio_plugin="$user_home/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background"
bridge_target="$audio_plugin/bin/ttfx-bg-rs-$arch"
runtime_commands=(
  omarchy-plymouth-effect-list
  omarchy-plymouth-effect-set
  omarchy-plymouth-effect-switcher
  omarchy-plymouth-playback-set
)
runtime_dirs=(/usr/share/omarchy/bin /usr/local/bin /usr/bin)

restore_file() {
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

restore_file /usr/lib/plymouth/ttfx-plymouth.so
restore_file /usr/share/plymouth/themes/omarchy/omarchy.plymouth
restore_file /usr/share/omarchy/default/plymouth/omarchy.plymouth
restore_file /etc/plymouth/plymouthd.conf
restore_file "$helper"
restore_file "$menu_extension"
for directory in "${runtime_dirs[@]}"; do
  for command in "${runtime_commands[@]}"; do
    restore_file "$directory/$command"
  done
done
if [[ -e $state_dir/${bridge_target//\//_} || -e $state_dir/${bridge_target//\//_}.absent ]]; then
  restore_file "$bridge_target"
fi

if [[ -f $state_dir/omarchy-static.absent ]]; then
  rm -rf -- /usr/share/plymouth/themes/omarchy-static
elif [[ -d $state_dir/omarchy-static ]]; then
  rm -rf -- /usr/share/plymouth/themes/omarchy-static
  cp -a --no-dereference "$state_dir/omarchy-static" /usr/share/plymouth/themes/omarchy-static
else
  echo "Missing uninstall backup for omarchy-static theme" >&2
  exit 1
fi

previous_theme=$(<"$state_dir/previous-theme")
[[ -n $previous_theme ]] || { echo "Missing previous Plymouth theme." >&2; exit 1; }
if [[ $(plymouth-set-default-theme) != "$previous_theme" ]]; then
  install -d -m 0755 /etc/plymouth
  if [[ ! -s /etc/plymouth/plymouthd.conf ]]; then
    printf '[Daemon]\n' >/etc/plymouth/plymouthd.conf
    chmod 0644 /etc/plymouth/plymouthd.conf
  fi
  plymouth-set-default-theme "$previous_theme"
fi
if [[ -x /usr/bin/limine-mkinitcpio ]]; then
  /usr/bin/limine-mkinitcpio
else
  /usr/bin/mkinitcpio -P
fi
rm -rf "$state_dir"
echo "Uninstalled TTFX Plymouth. Reboot to test the restored boot theme."
