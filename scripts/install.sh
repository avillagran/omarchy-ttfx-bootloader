#!/bin/bash
set -eEuo pipefail

[[ $EUID == 0 ]] || { echo "Run this script through sudo." >&2; exit 1; }
export PATH=/usr/share/omarchy/bin:/usr/local/bin:/usr/bin:/usr/sbin

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
state_dir=/var/lib/omarchy-ttfx-bootloader
helper=/usr/local/lib/omarchy-plymouth-ttfx-install
arch=$(uname -m)
new_install=false
committed=false
runtime_commands=(
  omarchy-plymouth-effect-list
  omarchy-plymouth-effect-set
  omarchy-plymouth-effect-switcher
  omarchy-plymouth-playback-set
)
runtime_dirs=(/usr/share/omarchy/bin /usr/local/bin /usr/bin)

case "$arch" in
  x86_64|aarch64) ;;
  *) echo "Unsupported architecture: $arch (supported: x86_64, aarch64)" >&2; exit 1 ;;
esac

command -v pacman >/dev/null || { echo "This installer supports Arch-based Omarchy systems only." >&2; exit 1; }
invoking_user=${SUDO_USER:-}
[[ -n $invoking_user && $invoking_user != root ]] || { echo "Run with: curl ... | sudo bash" >&2; exit 1; }
user_home=$(getent passwd "$invoking_user" | cut -d: -f6)
[[ -n $user_home && -d $user_home ]] || { echo "Could not resolve the invoking user's home." >&2; exit 1; }
user_group=$(id -gn "$invoking_user")
menu_extension="$user_home/.config/omarchy/extensions/omarchy-menu.jsonc"
audio_plugin="$user_home/.config/omarchy/plugins/io.github.avillagran.omarchy-audio-background"
bridge_target="$audio_plugin/bin/ttfx-bg-rs-$arch"

backup_name() {
  printf '%s' "${1//\//_}"
}

backup_file() {
  local path=$1 name
  name=$(backup_name "$path")
  [[ ! -e $state_dir/$name && ! -e $state_dir/$name.absent ]] || return 0
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

managed_files() {
  printf '%s\n' \
    /usr/lib/plymouth/ttfx-plymouth.so \
    /usr/share/plymouth/themes/omarchy/omarchy.plymouth \
    /usr/share/omarchy/default/plymouth/omarchy.plymouth \
    /etc/plymouth/plymouthd.conf \
    "$helper" \
    "$menu_extension"
  local directory command
  for directory in "${runtime_dirs[@]}"; do
    for command in "${runtime_commands[@]}"; do
      printf '%s/%s\n' "$directory" "$command"
    done
  done
  [[ -d $audio_plugin ]] && printf '%s\n' "$bridge_target"
}

restore_initial_state() {
  if $new_install && ! $committed; then
    while IFS= read -r path; do restore_file "$path"; done < <(managed_files)
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
  if ! pacman -S --needed --noconfirm "${missing[@]}"; then
    printf 'Could not install prerequisites: %s\nRun `omarchy update` and retry.\n' "${missing[*]}" >&2
    exit 1
  fi
fi
install -d -m 0700 "$state_dir"

if [[ ! -f $state_dir/.installed ]]; then
  new_install=true
  plymouth-set-default-theme >"$state_dir/previous-theme"
fi
[[ -f $state_dir/invoking-user ]] || printf '%s\n' "$invoking_user" >"$state_dir/invoking-user"
[[ -f $state_dir/user-home ]] || printf '%s\n' "$user_home" >"$state_dir/user-home"
while IFS= read -r path; do backup_file "$path"; done < <(managed_files)
if [[ ! -e $state_dir/omarchy-static && ! -e $state_dir/omarchy-static.absent ]]; then
  if [[ -e /usr/share/plymouth/themes/omarchy-static || -L /usr/share/plymouth/themes/omarchy-static ]]; then
    cp -a --no-dereference /usr/share/plymouth/themes/omarchy-static "$state_dir/omarchy-static"
  else
    : >"$state_dir/omarchy-static.absent"
  fi
fi

module="$state_dir/ttfx-plymouth-$arch.so"
"$root/bin/omarchy-plymouth-ttfx-build" build --output "$module"

bridge_binary=
if [[ -d $audio_plugin ]]; then
  bridge_target_dir="$state_dir/bridge-target-$arch"
  CARGO_TARGET_DIR="$bridge_target_dir" cargo build --release --locked --manifest-path "$root/bridge/Cargo.toml"
  bridge_binary="$bridge_target_dir/release/ttfx-bg-rs"
  [[ -x $bridge_binary ]] || { echo "Bridge build did not produce an executable." >&2; exit 1; }
fi

install -m 0755 "$root/bin/omarchy-plymouth-ttfx-install" "$helper"
install -m 0644 "$root/default/plymouth/omarchy.plymouth" /usr/share/omarchy/default/plymouth/omarchy.plymouth
"$helper" --module "$module"

for directory in "${runtime_dirs[@]}"; do
  install -d -m 0755 "$directory"
  for command in "${runtime_commands[@]}"; do
    install -m 0755 "$root/bin/$command" "$directory/$command"
  done
done

install -d -m 0755 "${menu_extension%/*}"
python3 - "$menu_extension" "$root/default/omarchy/ttfx-menu.json" <<'PY'
import json, sys
from pathlib import Path

def jsonc_to_json(text):
    out = []
    i = 0
    quoted = False
    escaped = False
    while i < len(text):
        c = text[i]
        if quoted:
            out.append(c)
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == '"':
                quoted = False
            i += 1
            continue
        if c == '"':
            quoted = True
            out.append(c)
            i += 1
        elif text.startswith("//", i):
            i = text.find("\n", i)
            if i < 0:
                break
            out.append("\n")
            i += 1
        elif text.startswith("/*", i):
            end = text.find("*/", i + 2)
            if end < 0:
                raise ValueError("unterminated JSONC block comment")
            out.extend("\n" * text[i:end + 2].count("\n"))
            i = end + 2
        else:
            out.append(c)
            i += 1
    text = "".join(out)
    out = []
    i = 0
    quoted = escaped = False
    while i < len(text):
        c = text[i]
        if quoted:
            out.append(c)
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == '"':
                quoted = False
        elif c == '"':
            quoted = True
            out.append(c)
        elif c == ',':
            j = i + 1
            while j < len(text) and text[j].isspace():
                j += 1
            if j < len(text) and text[j] in '}]':
                i += 1
                continue
            out.append(c)
        else:
            out.append(c)
        i += 1
    return "".join(out)

extension, additions = map(Path, sys.argv[1:])
current = json.loads(jsonc_to_json(extension.read_text()) or "{}") if extension.exists() else {}
current.update(json.loads(additions.read_text()))
extension.write_text(json.dumps(current, indent=2, ensure_ascii=False) + "\n")
PY
chown "$invoking_user:$user_group" "$menu_extension"
chmod 0644 "$menu_extension"

if [[ -n $bridge_binary ]]; then
  install -m 0755 "$bridge_binary" "$bridge_target"
  chown "$invoking_user:$user_group" "$bridge_target"
fi

current=$(PATH=/usr/share/omarchy/bin:/usr/local/bin:/usr/bin omarchy-plymouth-effect-set current)
[[ $current != off ]] || { echo "Installed effect getter still resolves to Off." >&2; exit 1; }
! grep -F 'add_option "Off"' /usr/share/omarchy/bin/omarchy-plymouth-effect-switcher >/dev/null || {
  echo "Installed selector still contains Off." >&2
  exit 1
}
plymouth-set-default-theme | grep -Fx omarchy >/dev/null
mapfile -d '' -t uki_images < <(find /boot/EFI/Linux -maxdepth 1 -type f -name '*.efi' -print0 2>/dev/null)
for image in "${uki_images[@]}"; do
  "$helper" --verify-initramfs "$image"
done

: >"$state_dir/.installed"
committed=true
trap - ERR HUP INT TERM
printf 'Installed TTFX Plymouth for %s (2x, effect=%s). Reboot to test it.\n' "$arch" "$current"
