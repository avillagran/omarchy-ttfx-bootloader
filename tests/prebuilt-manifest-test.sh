#!/bin/bash
# prebuilt/manifest is data: pins parse, malformed values are dropped, and
# nothing in it is executed.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
source "$root/scripts/common.sh"
fixture=$(mktemp -d "${TMPDIR:-/var/tmp}/ttfx-manifest-test.XXXXXXXX")
printf 'Test fixtures: %s\n' "$fixture"
sha=$(printf '%064d' 0 | tr 0 a)

cat >"$fixture/valid" <<MANIFEST
# comment
abi=26.134.222
release=https://github.com/example/repo/releases/download/tag
source_commit=$(printf '%040d' 0)
x86_64=$sha
aarch64=\$(touch $fixture/executed)
MANIFEST
read_prebuilt_manifest "$fixture/valid"
[[ $prebuilt_abi == 26.134.222 ]]
[[ ${prebuilt_sha256[x86_64]} == "$sha" ]]
[[ -z ${prebuilt_sha256[aarch64]:-} ]]
[[ ! -e $fixture/executed ]]
printf 'manifest pins parse and malformed values are dropped unexecuted: PASS\n'

printf 'abi=26.134.222\nrelease=http://insecure.example/x\nsource_commit=%040d\n' 0 >"$fixture/insecure"
if read_prebuilt_manifest "$fixture/insecure" 2>/dev/null; then
  echo 'manifest with a non-GitHub release URL was accepted' >&2
  exit 1
fi
printf 'manifest refuses release URLs outside GitHub: PASS\n'

read_prebuilt_manifest "$root/prebuilt/manifest"
[[ $prebuilt_abi == "$(sed -n 's/^readonly supported_abi=//p' "$root/bin/omarchy-plymouth-ttfx-build")" ]]
[[ -n ${prebuilt_sha256[x86_64]:-} && -n ${prebuilt_sha256[aarch64]:-} ]]
printf 'shipped manifest matches the build ABI and pins both architectures: PASS\n'
