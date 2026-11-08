#!/bin/bash
set -euo pipefail

[[ $EUID == 0 ]] || { echo "Run this script through sudo." >&2; exit 1; }
export PATH=/usr/share/omarchy/bin:/usr/local/bin:/usr/bin:/usr/sbin

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck source=scripts/common.sh
source "$root/scripts/common.sh"
acquire_installer_lock
acquire_plymouth_lock

[[ -f $state_dir/.installed ]] || { echo "No TTFX installer state found." >&2; exit 1; }
if [[ ! -f $installed_hashes ]]; then
  echo "This TTFX install comes from the 2026-09 installer, whose backups may predate package updates." >&2
  echo "Run the current installer once, then uninstall." >&2
  exit 1
fi
invoking_user=$(<"$state_dir/invoking-user")
user_home=$(<"$state_dir/user-home")
menu_extension="$user_home/.config/omarchy/extensions/omarchy-menu.jsonc"

# Give back only what is still exactly as the install left it. A file changed
# since, for example by an omarchy-settings update, already is someone
# else's and stays as it is.
left=()
while IFS= read -r path; do
  if ours_now "$path"; then
    restore_file "$path"
  elif recorded_hash "$path" >/dev/null && [[ -e $path ]]; then
    left+=("$path")
  fi
done < <(managed_files)

if [[ -f $state_dir/omarchy-static.absent ]]; then
  rm -rf -- /usr/share/plymouth/themes/omarchy-static
elif [[ -d $state_dir/omarchy-static ]]; then
  rm -rf -- /usr/share/plymouth/themes/omarchy-static
  cp -a --no-dereference "$state_dir/omarchy-static" /usr/share/plymouth/themes/omarchy-static
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
rmdir --ignore-fail-on-non-empty /etc/systemd/system/plymouth-quit.service.d 2>/dev/null || true
systemctl daemon-reload
rebuild_boot_images

if [[ -n $invoking_user && -d $user_home ]]; then
  runuser -u "$invoking_user" -- python3 "$root/scripts/menu-extension.py" remove "$menu_extension" ||
    echo "The Bootloader entries stay in $menu_extension; remove them by hand." >&2
fi

rm -rf -- "$state_dir"
for path in "${left[@]}"; do
  printf 'Left as it is (changed since the install): %s\n' "$path"
done
echo "Uninstalled TTFX Plymouth. Reboot to test the restored boot theme."
