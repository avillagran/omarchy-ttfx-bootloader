# Shared by scripts/install.sh and scripts/uninstall.sh; sourced, never run.

state_dir=/var/lib/omarchy-ttfx-bootloader
helper=/usr/local/lib/omarchy-plymouth-ttfx-install
installed_hashes=$state_dir/installed.sha256
recovery_theme_dir=/usr/share/plymouth/themes/omarchy-static
transaction_dir=
transaction_ready=false
runtime_commands=(
  omarchy-plymouth-effect-list
  omarchy-plymouth-effect-set
  omarchy-plymouth-effect-switcher
  omarchy-plymouth-playback-set
  omarchy-plymouth-set
)
runtime_dirs=(/usr/share/omarchy/bin /usr/local/bin /usr/bin)

# Every system path the installer writes. Paths an earlier installer wrote and
# this one removes stay listed, so their original state is restored too.
managed_files() {
  printf '%s\n' \
    /usr/lib/plymouth/ttfx-plymouth.so \
    /usr/share/plymouth/themes/omarchy/omarchy.plymouth \
    /usr/share/plymouth/themes/omarchy/omarchy.script \
    /usr/share/plymouth/themes/omarchy/ttfx-effects.txt \
    /usr/share/omarchy/default/plymouth/omarchy.plymouth \
    /usr/share/omarchy/default/plymouth/omarchy.script \
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

# Runtime settings and branding edit only the live native descriptor. The
# packaged default template stays ours only while its exact installed hash
# matches; quattro's stock script fallback also contains a [ttfx] section.
is_descriptor() {
  [[ $1 == /usr/share/plymouth/themes/omarchy/omarchy.plymouth || $1 == /usr/share/omarchy/default/plymouth/omarchy.plymouth ]]
}

is_live_descriptor() {
  [[ $1 == /usr/share/plymouth/themes/omarchy/omarchy.plymouth ]]
}

ours_now() {
  still_ours "$1" && return 0
  is_live_descriptor "$1" && [[ -f $1 && ! -L $1 ]] &&
    grep -Fxq '[ttfx]' "$1" && grep -Fxq 'ModuleName=ttfx-plymouth' "$1"
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

# Serialize install and uninstall so transaction snapshots cannot race.
acquire_installer_lock() {
  local lock=${1:-/run/lock/omarchy-ttfx-bootloader.lock}
  exec {installer_lock_fd}>"$lock"
  flock -n "$installer_lock_fd" || {
    echo "Another TTFX install or uninstall is running." >&2
    return 1
  }
}

acquire_plymouth_lock() {
  local lock=${1:-/run/lock/omarchy-plymouth.lock}
  exec {OMARCHY_PLYMOUTH_LOCK_FD}>"$lock"
  flock -n "$OMARCHY_PLYMOUTH_LOCK_FD" || {
    echo "Plymouth settings are being changed by another command." >&2
    return 1
  }
  export OMARCHY_PLYMOUTH_LOCK_FD
}

# Keep a separate pre-attempt snapshot for upgrades as well as first installs.
# The long-lived state_dir backups still describe what uninstall must restore.
begin_install_transaction() {
  transaction_ready=false
  transaction_dir=$(mktemp -d "${TMPDIR:-/var/tmp}/omarchy-ttfx-transaction.XXXXXXXX") || return 1
  chmod 0700 "$transaction_dir" || return 1
  local state_dir=$transaction_dir/files path
  install -d -m 0700 "$state_dir" || return 1
  while IFS= read -r path; do
    backup_file "$path" || return 1
  done < <(managed_files)
  if [[ -e $recovery_theme_dir || -L $recovery_theme_dir ]]; then
    cp -a --no-dereference "$recovery_theme_dir" "$transaction_dir/recovery" || return 1
  else
    : >"$transaction_dir/recovery.absent" || return 1
  fi
  plymouth-set-default-theme >"$transaction_dir/theme" || return 1
  [[ -s $transaction_dir/theme ]] || return 1
  transaction_ready=true
}

rollback_install_transaction() {
  $transaction_ready || return 0
  [[ -n $transaction_dir && -d $transaction_dir/files ]] || return 1
  local state_dir=$transaction_dir/files path failed=0 retired recovery_clear=true
  while IFS= read -r path; do
    restore_file "$path" || failed=1
  done < <(managed_files)
  if [[ -e $recovery_theme_dir || -L $recovery_theme_dir ]]; then
    # Retire the replacement beside itself, without traversing or deleting it.
    retired=$recovery_theme_dir.failed-${transaction_dir##*.}
    if ! mv -T -- "$recovery_theme_dir" "$retired"; then
      failed=1
      recovery_clear=false
    fi
  fi
  if $recovery_clear && [[ -e $transaction_dir/recovery || -L $transaction_dir/recovery ]]; then
    cp -a --no-dereference "$transaction_dir/recovery" "$recovery_theme_dir" || failed=1
  fi
  systemctl daemon-reload || failed=1
  if $boot_changed; then
    plymouth-set-default-theme "$(<"$transaction_dir/theme")" || failed=1
    rebuild_boot_images || failed=1
  fi
  printf 'Pre-attempt backup retained at %s\n' "$transaction_dir" >&2
  return "$failed"
}

# prebuilt/manifest pins the release modules. It is parsed as data, never
# sourced: abi=, release=, source_commit= and one sha256 per architecture.
read_prebuilt_manifest() {
  local manifest=$1 key value
  declare -gA prebuilt_sha256=()
  prebuilt_abi= prebuilt_release= prebuilt_commit=
  [[ -f $manifest ]] || { echo "Missing prebuilt manifest: $manifest" >&2; return 1; }
  while IFS='=' read -r key value; do
    [[ -z $key || $key == \#* ]] && continue
    case "$key" in
      abi) [[ $value =~ ^[0-9]+(\.[0-9]+)+$ ]] && prebuilt_abi=$value ;;
      release) [[ $value == https://github.com/* ]] && prebuilt_release=$value ;;
      source_commit) [[ $value =~ ^[0-9a-f]{40}$ ]] && prebuilt_commit=$value ;;
      x86_64|aarch64) [[ $value =~ ^[0-9a-f]{64}$ ]] && prebuilt_sha256[$key]=$value ;;
    esac
  done <"$manifest"
  [[ -n $prebuilt_abi && -n $prebuilt_release && -n $prebuilt_commit ]] ||
    { echo "Invalid prebuilt manifest: $manifest" >&2; return 1; }
}
