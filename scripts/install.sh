#!/bin/bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
state_dir=/var/lib/omarchy-ttfx-bootloader
arch=$(uname -m)

case "$arch" in
  x86_64|aarch64) ;;
  *) echo "Unsupported architecture: $arch (supported: x86_64, aarch64)" >&2; exit 1 ;;
esac

command -v pacman >/dev/null || { echo "This installer supports Arch-based Omarchy systems only." >&2; exit 1; }

pacman -S --needed --noconfirm base-devel rust pkgconf plymouth
install -d -m 0700 "$state_dir"

# Keep the prior state for uninstall only once. The native installer owns its
# own transaction for module/theme/boot-image publication.
if [[ ! -f $state_dir/.installed ]]; then
  for path in \
    /usr/lib/plymouth/ttfx-plymouth.so \
    /usr/share/plymouth/themes/omarchy/omarchy.plymouth \
    /usr/share/omarchy/default/plymouth/omarchy.plymouth; do
    name=${path//\//_}
    if [[ -e $path || -L $path ]]; then
      cp -a --no-dereference "$path" "$state_dir/$name"
    else
      : > "$state_dir/$name.absent"
    fi
  done
fi

module="$state_dir/ttfx-plymouth-$arch.so"
"$root/bin/omarchy-plymouth-ttfx-build" build --output "$module"
install -m 0755 "$root/bin/omarchy-plymouth-ttfx-install" /usr/local/lib/omarchy-plymouth-ttfx-install
install -m 0644 "$root/default/plymouth/omarchy.plymouth" /usr/share/omarchy/default/plymouth/omarchy.plymouth
/usr/local/lib/omarchy-plymouth-ttfx-install --module "$module"
: > "$state_dir/.installed"
echo "Installed TTFX Plymouth for $arch. Reboot to test it."
