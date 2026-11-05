#!/bin/bash
# The menu block edit must keep other tools' comment-marked blocks byte for
# byte, replace the 2026-09 style.bootloader entries, stay idempotent, and
# refuse files it cannot read the way Omarchy's menu reads them.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
editor=$root/scripts/menu-extension.py
block=$root/default/omarchy/ttfx-menu.jsonc
tmp=$(mktemp -d "${TMPDIR:-/var/tmp}/ttfx-test.XXXXXXXX")
printf 'Test fixtures: %s\n' "$tmp"

fail() { printf 'not ok - %s\n' "$1" >&2; exit 1; }
pass() { printf 'ok - %s\n' "$1"; }

# Parse like Omarchy's MenuModel.stripJsonc and print the sorted keys.
keys() {
  node -e '
    const raw = require("fs").readFileSync(process.argv[1], "utf8")
    const stripped = raw.replace(/^\s*\/\/[^\n]*(\n|$)/gm, "").replace(/,(\s*[}\]])/g, "$1")
    console.log(Object.keys(stripped.trim() ? JSON.parse(stripped) : {}).sort().join(" "))
  ' "$1"
}

ours="appearance.bootloader appearance.bootloader.effect appearance.bootloader.full-animated appearance.bootloader.input-only"

# A file other tools manage with their own markers, ending in a trailing comma.
cat >"$tmp/menu.jsonc" <<'EOF'
{
  // ambient-background-submenu:start
  "appearance.background": {"icon": "", "label": "Background", "aliases": ["background", "wallpaper"]},
  "appearance.background.theme": {"label": "Theme", "action": "echo 'a//b'"},
  // ambient-background-submenu:end
  // BEGIN OTHER TOOL
  "other": {"label": "Other", "action": "echo \"}\","},
  // END OTHER TOOL
}
EOF
cp "$tmp/menu.jsonc" "$tmp/original.jsonc"
python3 "$editor" add "$tmp/menu.jsonc" "$block"
[[ $(keys "$tmp/menu.jsonc") == "appearance.background appearance.background.theme $ours other" ]] ||
  fail "add keeps every entry and adds the Bootloader block: $(keys "$tmp/menu.jsonc")"
grep -v 'omarchy-ttfx-bootloader\|"appearance.bootloader' "$tmp/menu.jsonc" | cmp -s - "$tmp/original.jsonc" ||
  fail "add changes nothing outside its own block"
pass "add keeps other tools' marked blocks byte for byte"

cp "$tmp/menu.jsonc" "$tmp/added.jsonc"
python3 "$editor" add "$tmp/menu.jsonc" "$block"
cmp -s "$tmp/menu.jsonc" "$tmp/added.jsonc" || fail "a second add is a no-op"
pass "add is idempotent"

python3 "$editor" remove "$tmp/menu.jsonc"
cmp -s "$tmp/menu.jsonc" "$tmp/original.jsonc" || fail "remove restores the file exactly"
python3 "$editor" remove "$tmp/menu.jsonc"
cmp -s "$tmp/menu.jsonc" "$tmp/original.jsonc" || fail "a second remove is a no-op"
pass "remove restores the original bytes"

# What the 2026-09 installer left: json.dumps output with style.bootloader keys.
cat >"$tmp/legacy.jsonc" <<'EOF'
{
  "keep": {
    "label": "Keep"
  },
  "style.bootloader": {
    "icon": "󰑣",
    "label": "Bootloader"
  },
  "style.bootloader.effect": {
    "icon": "󰑣",
    "label": "Effect",
    "action": "effect=$(omarchy-plymouth-effect-switcher)"
  }
}
EOF
python3 "$editor" add "$tmp/legacy.jsonc" "$block"
[[ $(keys "$tmp/legacy.jsonc") == "$ours keep" ]] || fail "legacy style.bootloader entries are replaced: $(keys "$tmp/legacy.jsonc")"
python3 "$editor" remove "$tmp/legacy.jsonc"
[[ $(keys "$tmp/legacy.jsonc") == "keep" ]] || fail "remove leaves only the user's entries"
pass "legacy style.bootloader entries are replaced and removed"

rm -f "$tmp/missing.jsonc"
python3 "$editor" add "$tmp/missing.jsonc" "$block"
[[ $(keys "$tmp/missing.jsonc") == "$ours" ]] || fail "add creates a missing extension"
python3 "$editor" remove "$tmp/missing.jsonc"
[[ -z $(keys "$tmp/missing.jsonc") ]] || fail "remove empties a file it created"
rm -f "$tmp/absent.jsonc"
python3 "$editor" remove "$tmp/absent.jsonc"
[[ ! -e $tmp/absent.jsonc ]] || fail "remove does not create a missing extension"
pass "a missing extension is created on add and never on remove"

printf '{ "broken": \n' >"$tmp/broken.jsonc"
cp "$tmp/broken.jsonc" "$tmp/broken.orig"
if python3 "$editor" add "$tmp/broken.jsonc" "$block" 2>/dev/null; then
  fail "an unparseable extension is refused"
fi
cmp -s "$tmp/broken.jsonc" "$tmp/broken.orig" || fail "a refused extension is left untouched"

printf '{\n  "appearance.bootloader": {"label": "Mine"}\n}\n' >"$tmp/clash.jsonc"
cp "$tmp/clash.jsonc" "$tmp/clash.orig"
if python3 "$editor" add "$tmp/clash.jsonc" "$block" 2>/dev/null; then
  fail "an entry the user owns under our key is refused"
fi
cmp -s "$tmp/clash.jsonc" "$tmp/clash.orig" || fail "a clashing extension is left untouched"

ln -s "$tmp/original.jsonc" "$tmp/link.jsonc"
if python3 "$editor" add "$tmp/link.jsonc" "$block" 2>/dev/null; then
  fail "a symlinked extension is refused"
fi
pass "unparseable, clashing, and symlinked extensions are refused untouched"

printf 'menu extension: PASS\n'
