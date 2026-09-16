#!/bin/bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

fail() { printf 'not ok - %s\n' "$1" >&2; exit 1; }
pass() { printf 'ok - %s\n' "$1"; }

for script in install.sh uninstall.sh scripts/install.sh scripts/uninstall.sh bin/omarchy-plymouth-*; do
  bash -n "$root/$script"
done
pass "shell scripts parse"

grep -F 'sudo "$tmp/scripts/install.sh"' "$root/install.sh" >/dev/null || fail "oneliner does not enter the privileged installer"
grep -F 'OMARCHY_TTFX_ARCHIVE' "$root/install.sh" >/dev/null || fail "oneliner cannot use a verified staging archive"
grep -F 'OMARCHY_TTFX_SHA256' "$root/install.sh" >/dev/null || fail "oneliner staging archive cannot be hash-pinned"
pass "oneliner enters sudo and supports hash-pinned staging"

grep -Fx '#define TTFX_PLAYBACK_STEPS_PER_TICK 2U' "$root/native/plymouth-ttfx-plugin/state.h" >/dev/null || fail "native state is not 2x"
grep -Fx '#define ENGINE_STEPS_PER_TICK 2U' "$root/native/plymouth-ttfx-plugin/plugin.c" >/dev/null || fail "handoff contract is not 2x"
grep -Fx 'const BOOT_STEPS_PER_TICK: usize = 2;' "$root/bridge/src/main.rs" >/dev/null || fail "desktop bridge is not 2x"
pass "native and desktop handoff agree on 2x"

if grep -F 'add_option "Off"' "$root/bin/omarchy-plymouth-effect-switcher" >/dev/null; then
  fail "effect selector exposes Off"
fi
grep -F '[[ $current == off ]] && current=decrypt' "$root/bin/omarchy-plymouth-effect-switcher" >/dev/null || fail "legacy Off does not migrate visually to Decrypt"
grep -Fx 'Effect=decrypt' "$root/default/plymouth/omarchy.plymouth" >/dev/null || fail "descriptor default is not Decrypt"
pass "selector omits Off and defaults to Decrypt"

for directory in /usr/share/omarchy/bin /usr/local/bin /usr/bin; do
  grep -F "$directory" "$root/scripts/install.sh" >/dev/null || fail "installer misses runtime command path $directory"
done
grep -F '"$helper" --module "$module"' "$root/scripts/install.sh" >/dev/null || fail "installer does not invoke transactional native installer"
if grep -F -- '--force-enable' "$root/scripts/install.sh" >/dev/null; then
  fail "standalone installer can overwrite a valid effect selection"
fi
grep -F 'legacy Off state all normalize' "$root/bin/omarchy-plymouth-ttfx-install" >/dev/null || fail "legacy Off does not normalize to Decrypt"
grep -F 'chmod 0644 "$module_stage"' "$root/bin/omarchy-plymouth-ttfx-install" >/dev/null && fail "installer makes Plymouth module non-executable"
grep -F "printf '[Daemon]\\n'" "$root/bin/omarchy-plymouth-ttfx-install" >/dev/null || fail "empty Plymouth configuration is not repaired"
pass "installer covers Omarchy runtime paths and Plymouth 26 edge cases"

python3 -m json.tool "$root/default/omarchy/ttfx-menu.json" >/dev/null
python3 - "$root/default/omarchy/ttfx-menu.json" <<'PY'
import json, sys
obj=json.load(open(sys.argv[1]))
assert set(obj) == {
    "style.bootloader",
    "style.bootloader.full-animated",
    "style.bootloader.input-only",
    "style.bootloader.effect",
}
PY
pass "menu extension is valid and complete"

python3 - "$root/scripts/install.sh" "$root/default/omarchy/ttfx-menu.json" <<'PY'
import json, sys, tempfile
from pathlib import Path
script, additions = map(Path, sys.argv[1:])
text = script.read_text()
marker = "python3 - \"$menu_extension\" \"$root/default/omarchy/ttfx-menu.json\" <<'PY'\n"
program = text.split(marker, 1)[1].split("\nPY\n", 1)[0]
with tempfile.TemporaryDirectory() as directory:
    extension = Path(directory) / "menu.jsonc"
    extension.write_text('''{
      // keep existing entries
      "existing": {"url": "https://example.test/a//b",},
      /* and block comments */
    }''')
    old_argv = sys.argv
    sys.argv = ["merge-menu", str(extension), str(additions)]
    try:
        exec(compile(program, str(script), "exec"), {})
    finally:
        sys.argv = old_argv
    merged = json.loads(extension.read_text())
    assert merged["existing"]["url"] == "https://example.test/a//b"
    assert "style.bootloader.effect" in merged
PY
pass "JSONC menu merge preserves quoted comment markers and existing entries"
