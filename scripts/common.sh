# Shared by scripts/install.sh and scripts/uninstall.sh; sourced, never run.

state_dir=/var/lib/omarchy-ttfx-bootloader
helper=/usr/local/lib/omarchy-plymouth-ttfx-install
installed_hashes=$state_dir/installed.sha256
runtime_commands=(
  omarchy-plymouth-effect-list
  omarchy-plymouth-effect-set
  omarchy-plymouth-effect-switcher
  omarchy-plymouth-playback-set
)
runtime_dirs=(/usr/share/omarchy/bin /usr/local/bin /usr/bin)

# Every system path the installer writes. Paths an earlier installer wrote and
# this one removes stay listed, so their original state is restored too.
managed_files() {
  printf '%s\n' \
    /usr/lib/plymouth/ttfx-plymouth.so \
    /usr/share/plymouth/themes/omarchy/omarchy.plymouth \
    /usr/share/plymouth/themes/omarchy/ttfx-effects.txt \
    /usr/share/omarchy/default/plymouth/omarchy.plymouth \
    /usr/share/omarchy/default/plymouth/ttfx-effects.txt \
    /etc/plymouth/plymouthd.conf \
    /etc/mkinitcpio.conf.d/zz-ttfx-kbd-order.conf \
    /etc/systemd/system/plymouth-quit.service.d/10-omarchy-retain-splash.conf \
    /etc/systemd/system/plymouth-quit.service.d/20-ttfx-deactivate.conf \
    /etc/systemd/system/omarchy-plymouth-reap.service \
    /etc/systemd/system/omarchy-plymouth-reap.timer \
    "$helper"
  local directory command
  for directory in "${runtime_dirs[@]}"; do
    for command in "${runtime_commands[@]}"; do
      printf '%s/%s\n' "$directory" "$command"
    done
  done
}

backup_name() {
  printf '%s' "${1//\//_}"
}

has_backup() {
  local name
  name=$(backup_name "$1")
  [[ -e $state_dir/$name || -L $state_dir/$name || -e $state_dir/$name.absent ]]
}

backup_file() {
  local path=$1 name
  name=$(backup_name "$path")
  rm -f -- "$state_dir/$name" "$state_dir/$name.absent"
  if [[ -e $path || -L $path ]]; then
    cp -a --no-dereference "$path" "$state_dir/$name"
  else
    : >"$state_dir/$name.absent"
  fi
}

restore_file() {
  local path=$1 name
  name=$(backup_name "$path")
  if [[ -f $state_dir/$name.absent ]]; then
    rm -f -- "$path"
  elif [[ -e $state_dir/$name || -L $state_dir/$name ]]; then
    install -d -m 0755 "${path%/*}"
    cp -a --remove-destination "$state_dir/$name" "$path"
  fi
}

recorded_hash() {
  local wanted=$1 hash path
  [[ -f $installed_hashes ]] || return 1
  while read -r hash path; do
    if [[ $path == "$wanted" ]]; then
      printf '%s\n' "$hash"
      return 0
    fi
  done <"$installed_hashes"
  return 1
}

# True when the path still holds exactly what the last install wrote. Anything
# else, such as a package update that replaced the file, is no longer ours.
still_ours() {
  local recorded current
  recorded=$(recorded_hash "$1") || return 1
  [[ -f $1 && ! -L $1 ]] || return 1
  current=$(sha256sum -- "$1" | cut -d' ' -f1)
  [[ $current == "$recorded" ]]
}

# The two Plymouth descriptors also change through this project's own helpers
# (omarchy-plymouth-effect-set, omarchy-plymouth-playback-set), so they are
# ours while they hold a [ttfx] section; a package update writes them without
# one.
is_descriptor() {
  [[ $1 == /usr/share/plymouth/themes/omarchy/omarchy.plymouth || $1 == /usr/share/omarchy/default/plymouth/omarchy.plymouth ]]
}

ours_now() {
  if is_descriptor "$1"; then
    [[ -f $1 && ! -L $1 ]] && grep -Fxq '[ttfx]' "$1"
  else
    still_ours "$1"
  fi
}

record_installed_hashes() {
  local path
  while IFS= read -r path; do
    if [[ -f $path && ! -L $path ]]; then
      sha256sum -- "$path"
    fi
  done < <(managed_files) >"$installed_hashes.new"
  mv -f -- "$installed_hashes.new" "$installed_hashes"
}

rebuild_boot_images() {
  if [[ -x /usr/bin/limine-mkinitcpio ]]; then
    /usr/bin/limine-mkinitcpio
  else
    /usr/bin/mkinitcpio -P
  fi
}
