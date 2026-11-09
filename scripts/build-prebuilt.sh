#!/bin/bash
# Build the release modules for x86_64 and aarch64 inside rootless Arch
# containers (aarch64 runs under qemu-user binfmt), against the Plymouth ABI
# the installer requires. Maintainer tool: the installer never runs it.
#
# Usage: scripts/build-prebuilt.sh [x86_64|aarch64 ...]
# Output: ${TMPDIR:-/var/tmp}/omarchy-ttfx-prebuilt.XXXXXXXX/ with one module
# per architecture, SHA256SUMS and build-info.txt. The directory is retained.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
engine=${CONTAINER_ENGINE:-podman}
abi=$(sed -n 's/^readonly supported_abi=//p' "$root/bin/omarchy-plymouth-ttfx-build")
[[ -n $abi ]] || { echo "Could not read supported_abi from the build helper." >&2; exit 1; }
(( $# )) || set -- x86_64 aarch64

[[ -z $(git -C "$root" status --porcelain) ]] ||
  { echo "Commit or stash local changes first: release modules must match a commit." >&2; exit 1; }
commit=$(git -C "$root" rev-parse HEAD)
out=$(mktemp -d "${TMPDIR:-/var/tmp}/omarchy-ttfx-prebuilt.XXXXXXXX")
git -C "$root" archive --format=tar "$commit" bin native default logo.txt >"$out/source.tar"

for arch in "$@"; do
  case "$arch" in
    x86_64) image=docker.io/library/archlinux:base-devel platform=linux/amd64 ;;
    aarch64) image=docker.io/lopsided/archlinux:latest platform=linux/arm64 ;;
    *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
  esac
  "$engine" run --rm --platform "$platform" --network host \
    -v "$out:/out:Z" -e ARCH="$arch" -e ABI="$abi" "$image" bash -euo pipefail -c '
      [[ $(uname -m) == "$ARCH" ]] || { echo "container is $(uname -m), expected $ARCH" >&2; exit 1; }
      pacman-key --init >/dev/null 2>&1
      [[ $ARCH == x86_64 ]] && pacman-key --populate archlinux >/dev/null 2>&1
      [[ $ARCH == aarch64 ]] && pacman-key --populate archlinuxarm >/dev/null 2>&1
      pacman -Syu --noconfirm --needed base-devel rust plymouth pkgconf python file binutils >/dev/null
      mkdir /work && tar -xf /out/source.tar -C /work
      /work/bin/omarchy-plymouth-ttfx-build build --output "/out/ttfx-plymouth-$ARCH.so"
      # Resolve every symbol against the distribution libply, as plymouthd does.
      python3 -I -c "
import ctypes, os, sys
lib = ctypes.CDLL(sys.argv[1], os.RTLD_NOW)
get = lib.ply_boot_splash_plugin_get_interface
get.restype = ctypes.c_void_p
assert get(), \"null plugin interface\"
" "/out/ttfx-plymouth-$ARCH.so"
      {
        printf "%s: %s\n" "$ARCH" "$(pacman -Q plymouth)"
        printf "%s: %s\n" "$ARCH" "$(rustc --version)"
        printf "%s: %s\n" "$ARCH" "$(gcc --version | head -1)"
      } >"/out/build-info-$ARCH.txt"
    '
done

rm -f -- "$out/source.tar"
{
  printf 'commit: %s\nabi: %s\n' "$commit" "$abi"
  cat "$out"/build-info-*.txt
} >"$out/build-info.txt"
rm -f -- "$out"/build-info-*.txt
(cd "$out" && sha256sum ttfx-plymouth-*.so >SHA256SUMS)
cat "$out/build-info.txt" "$out/SHA256SUMS"
printf 'Release modules in %s\n' "$out"
