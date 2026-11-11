#!/bin/bash
# Boot menu theme: renderer, GRUB entry naming, and apply/remove for Limine and
# GRUB against a fake /boot. Writes to /boot, /etc and /usr/share, so it runs
# only inside a disposable container (Arch, with grub, python and kbd).
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
fail() { printf 'not ok - %s\n' "$1" >&2; exit 1; }
pass() { printf 'ok - %s\n' "$1"; }

[[ -e /run/.containerenv || -e /.dockerenv || ${container:-} != "" ]] ||
  fail "refusing to run outside a container"
(( EUID == 0 )) || fail "needs root inside the container"

work=$(mktemp -d "${TMPDIR:-/var/tmp}/boot-theme-test.XXXXXXXX")
renderer=$root/boot-theme/omarchy-boot-theme

# --- renderer --------------------------------------------------------------
printf '[Plymouth Theme]\nModuleName=ttfx-plymouth\n[ttfx]\nMode=fixed\nEffect=beams\n' >"$work/beams.plymouth"
printf '/+Omarchy\n  //linux\n    protocol: linux\n  //Snapshots\n' >"$work/limine.conf"
"$renderer" limine --width 2560 --height 1600 --limine-conf "$work/limine.conf" --art logo --effect decrypt --out "$work/decrypt" 2>/dev/null
"$renderer" limine --width 2560 --height 1600 --limine-conf "$work/limine.conf" --art logo --descriptor "$work/beams.plymouth" --out "$work/beams" 2>/dev/null
"$renderer" limine --width 2560 --height 1600 --limine-conf "$work/limine.conf" --art logo --effect no-such-effect --out "$work/unknown" 2>/dev/null
cmp -s "$work/decrypt/boot-theme.png" "$work/beams/boot-theme.png" && fail "logo does not follow the configured effect"
cmp -s "$work/decrypt/boot-theme.png" "$work/unknown/boot-theme.png" || fail "unknown effect does not fall back to decrypt"
grep -Fx 'interface_resolution: 2560x1600' "$work/decrypt/limine.conf.theme" >/dev/null || fail "Limine resolution not pinned"
grep -Fx 'term_font_scale: 2x2' "$work/decrypt/limine.conf.theme" >/dev/null || fail "Limine font scale"
[[ $(stat -c %s "$work/decrypt/boot-font-8x16.bin") == 4096 ]] || fail "Limine font is not 256 8x16 glyphs"
pass "renderer (--art logo) follows Effect= and falls back to decrypt"

# --- firmware-hold mark and hint -------------------------------------------
# png_probe MODE PNG [YMIN YMAX] over rows YMIN..YMAX-1 (default: all) of an
# 8-bit RGB PNG with filter 0 on every row, as the renderer writes it:
#   bbox     "x0 y0 x1 y1 colour-at-x0,y0" of pixels differing from 1a1b26
#   colours  the distinct non-background colours, sorted
#   sha      sha256 of the decoded rows
png_probe() {
  python3 -I - "$@" <<'PY'
import hashlib, struct, sys, zlib
mode, path = sys.argv[1:3]
data = open(path, "rb").read()
pos, idat = 8, b""
while pos < len(data):
    n, kind = struct.unpack(">I4s", data[pos:pos + 8])
    if kind == b"IHDR":
        w, h = struct.unpack(">II", data[pos + 8:pos + 16])
    elif kind == b"IDAT":
        idat += data[pos + 8:pos + 8 + n]
    pos += 12 + n
raw, stride, bg = zlib.decompress(idat), w * 3, b"\x1a\x1b\x26"
if mode == "sha":
    sys.exit(print(hashlib.sha256(raw).hexdigest()))
ymin, ymax = (int(sys.argv[3]), int(sys.argv[4])) if len(sys.argv) > 4 else (0, h)
x0 = y0 = 1 << 30
x1 = y1 = -1
colours, corner = set(), "none"
for y in range(max(ymin, 0), min(ymax, h)):
    assert raw[y * (stride + 1)] == 0
    row = raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)]
    if row == bg * w:
        continue
    xs = [x for x in range(w) if row[x * 3:x * 3 + 3] != bg]
    colours.update(row[x * 3:x * 3 + 3].hex() for x in xs)
    x0, x1, y0, y1 = min(x0, xs[0]), max(x1, xs[-1]), min(y0, y), max(y1, y)
if x1 >= 0:
    o = y0 * (stride + 1) + 1 + x0 * 3
    corner = raw[o:o + 3].hex()
print(" ".join(sorted(colours)) if mode == "colours" else f"{x0} {y0} {x1} {y1} {corner}")
PY
}
mark=$root/default/plymouth/boot-mark.png
limine_hint="Press any key for boot options"
# expect_mark W H SCALE FONT X0 Y0 SIZE [extra renderer args]
expect_mark() {
  local w=$1 h=$2 scale=$3 font=$4 x0=$5 y0=$6 size=$7 out="$work/mark-$1x$2-$3"
  shift 7
  "$renderer" limine --width "$w" --height "$h" --limine-conf "$work/limine.conf" --out "$out" "$@" 2>"$out.log" ||
    { cat "$out.log" >&2; fail "mark render ${w}x${h} failed"; }
  grep -F "mark=${size}px@$x0,$y0 scale=$scale " "$out.log" >/dev/null || fail "${w}x${h}: $(cat "$out.log")"
  # The mark is opaque to its edges, so down to its bottom edge the only
  # non-background pixels form exactly the drawn square.
  local box
  box=$(png_probe bbox "$out/boot-theme.png" 0 $((y0 + size)))
  [[ $box == "$x0 $y0 $((x0 + size - 1)) $((y0 + size - 1)) 9ece6a" ]] ||
    fail "${w}x${h}: mark box is $box, want ${size}px at $x0,$y0"
  # The hint: one line of the menu font, a twenty-fifth of the screen above its
  # bottom edge, centred on the mark, len(text) x 8 x font scale wide; nothing else below.
  local hw=$(( ${#limine_hint} * 8 * font )) hh=$(( 16 * font ))
  local hx=$(( x0 + size / 2 - hw / 2 )) hy=$(( h - h / 25 - hh ))
  grep -F "hint=${hw}x${hh}@$hx,$hy " "$out.log" >/dev/null || fail "${w}x${h}: hint geometry: $(cat "$out.log")"
  local ink ix0 iy0 ix1 iy1
  ink=$(png_probe bbox "$out/boot-theme.png" $((y0 + size)) "$h")
  read -r ix0 iy0 ix1 iy1 _ <<<"$ink"
  (( ix0 >= hx && ix1 < hx + hw && iy0 >= hy && iy1 < hy + hh )) ||
    fail "${w}x${h}: hint ink $ink outside its ${hw}x${hh} box at $hx,$hy"
  (( ${#limine_hint} * 8 * font - (ix1 - ix0 + 1) <= 16 * font )) || fail "${w}x${h}: hint ink $ink too narrow"
  local off=$(( (ix0 + ix1 + 1) - (2 * x0 + size) ))
  (( off >= -8 * font && off <= 8 * font )) || fail "${w}x${h}: hint ink $ink not centred on the mark"
  [[ $(png_probe colours "$out/boot-theme.png" $((y0 + size)) "$h") == 9ece6a ]] || fail "${w}x${h}: hint colour"
}
# 3456x2160: Plymouth picks device scale 2 -> 256px box centred on 1728,1080;
# Limine's font scale there is 2, so the hint is 480x32 at 1488,2042.
expect_mark 3456 2160 2 2 1600 952 256 --mark "$mark"
expect_mark 1920 1080 1 1 896 476 128 --mark "$mark"
expect_mark 1366 768 1 1 619 320 128 --mark "$mark"
expect_mark 3456 2160 1 2 1664 1016 128 --mark "$mark" --scale 1
pass "mark: plugin size and centring at 3456x2160 (256px), 1920x1080, 1366x768 and --scale; hint centred near the bottom"

# --hint '' is the mark alone, pixel for pixel what the renderer drew before
# the hint existed (sha256 of the decoded 3456x2160 rows).
"$renderer" limine --width 3456 --height 2160 --limine-conf "$work/limine.conf" --mark "$mark" --hint '' \
  --out "$work/nohint" 2>"$work/nohint.log"
grep -F "hint=" "$work/nohint.log" >/dev/null && fail "--hint '' still draws a hint"
[[ $(png_probe sha "$work/nohint/boot-theme.png") == 5eb586b8e0a99b985107efdcb42d10b2bfda0992519418df145b0036a1f79367 ]] ||
  fail "--hint '' differs from the mark-only render"
"$renderer" limine --width 1366 --height 768 --limine-conf "$work/limine.conf" --mark "$mark" --hint "Custom hint" \
  --out "$work/customhint" 2>"$work/customhint.log"
grep -F "hint=88x16@639,722 " "$work/customhint.log" >/dev/null || fail "--hint TEXT: $(cat "$work/customhint.log")"
# Behind a hidden notch (74 rows on a 16" MacBook Pro) the mark is lifted
# like the plugin's hold mark: 19 logical pixels at scale 2, 38 rows.
"$renderer" limine --width 3456 --height 2160 --limine-conf "$work/limine.conf" --mark "$mark" --hidden-notch 74 \
  --out "$work/notch" 2>"$work/notch.log"
grep -F "mark=256px@1600,914 scale=2 " "$work/notch.log" >/dev/null || fail "--hidden-notch: $(cat "$work/notch.log")"
pass "hint: --hint '' reproduces the mark-only image, --hint TEXT is honoured"

# The menu image: the same mark without the hint, raised into the gap between
# the help rows and the tree (2560x1600: rows 6..20 of a 46-row terminal at
# 32px, so 256px at 1152,352), and named as the wallpaper while the mark with
# the hint is the quiet_wallpaper.
"$renderer" limine --width 2560 --height 1600 --limine-conf "$work/limine.conf" --mark "$mark" --scale 2 \
  --limine-path "boot():/omarchy-boot-theme" --out "$work/menu" 2>"$work/menu.log"
menu_at=$(sed -n 's/.* menu_mark=256px@\([0-9]*\),\([0-9]*\).*/\1 \2/p' "$work/menu.log")
[[ -n $menu_at ]] || fail "menu mark: $(cat "$work/menu.log")"
read -r mx my <<<"$menu_at"
[[ $(png_probe bbox "$work/menu/boot-menu.png" 0 1600) == "$mx $my $((mx + 255)) $((my + 255)) 9ece6a" ]] ||
  fail "menu image is not the mark alone: $(png_probe bbox "$work/menu/boot-menu.png" 0 1600)"
(( mx == 1152 && my >= 64 + 6 * 32 && my + 256 <= 672 )) || fail "menu mark at $mx,$my is not above the centred mark's rows"
grep -Fx 'wallpaper: boot():/omarchy-boot-theme/boot-menu.png' "$work/menu/limine.conf.theme" >/dev/null || fail "menu wallpaper key"
grep -Fx 'quiet_wallpaper: boot():/omarchy-boot-theme/boot-theme.png' "$work/menu/limine.conf.theme" >/dev/null || fail "quiet_wallpaper key"
pass "menu: boot-menu.png carries the raised mark without the hint; wallpaper and quiet_wallpaper keys"

# Default lookup finds the repository copy next to the script.
"$renderer" limine --width 1920 --height 1080 --limine-conf "$work/limine.conf" --out "$work/default-mark" 2>"$work/default-mark.log"
grep -E "source=[^ ]*/default/plymouth/boot-mark\.png( |$)" "$work/default-mark.log" >/dev/null ||
  fail "default mark lookup: $(cat "$work/default-mark.log")"
cmp -s "$work/default-mark/boot-theme.png" "$work/mark-1920x1080-1/boot-theme.png" || fail "default mark differs from --mark"

# GRUB background carries the same mark (plus the tree text Limine would draw).
printf 'linux\nlinux (fallback)\n' >"$work/grub-entries"
"$renderer" grub --width 1920 --height 1080 --entries "$work/grub-entries" --mark "$mark" --out "$work/grub-mark" 2>"$work/grub-mark.log"
grep -F "mark=128px@896,476 scale=1 hint=208x16@856,1021 " "$work/grub-mark.log" >/dev/null ||
  fail "GRUB mark or Esc hint: $(cat "$work/grub-mark.log")"

# Fallback: missing, non-PNG and unsupported (16-bit, interlaced) marks all
# render the logo, exactly as --art logo does, with a warning.
"$renderer" limine --width 1920 --height 1080 --limine-conf "$work/limine.conf" --art logo --effect decrypt --out "$work/logo-1080" 2>/dev/null
printf 'not a png\n' >"$work/bad.png"
python3 -I - "$work" <<'PY'
import struct, sys, zlib
def png(path, depth, colour, interlace, raw):
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, depth, colour, 0, 0, interlace))
                           + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
png(sys.argv[1] + "/16bit.png", 16, 6, 0, (b"\0" + b"\xff" * 16) * 2)
png(sys.argv[1] + "/interlaced.png", 8, 6, 1, b"\0" * 64)
png(sys.argv[1] + "/gray.png", 8, 0, 0, b"\0\0\0" * 2)
PY
for bad in "$work/missing.png" "$work/bad.png" "$work/16bit.png" "$work/interlaced.png" "$work/gray.png"; do
  name=$(basename "$bad" .png)
  "$renderer" limine --width 1920 --height 1080 --limine-conf "$work/limine.conf" --effect decrypt --mark "$bad" \
    --out "$work/fallback-$name" 2>"$work/fallback-$name.log" || fail "fallback render for $name failed"
  grep -F "using the logo instead" "$work/fallback-$name.log" >/dev/null || fail "no fallback warning for $name"
  grep -F "effect=decrypt" "$work/fallback-$name.log" >/dev/null || fail "$name did not fall back to the logo"
  cmp -s "$work/fallback-$name/boot-theme.png" "$work/logo-1080/boot-theme.png" || fail "$name fallback differs from --art logo"
done
pass "mark: default lookup, GRUB background, and logo fallback for missing or unsupported PNGs"

# --- GRUB entry naming -----------------------------------------------------
cat >"$work/linux.cfg" <<'EOF'
function gfxmode { set gfxpayload="${1}"; }
menuentry 'Omarchy Linux' --class os $menuentry_id_option 'gnulinux-simple-UUID' {
	linux /vmlinuz-linux
}
menuentry 'Omarchy Linux, with Linux linux' --class os $menuentry_id_option 'gnulinux-linux-advanced-UUID' {
	linux /vmlinuz-linux
}
menuentry 'Omarchy Linux, with Linux linux (fallback initramfs)' --class os $menuentry_id_option 'gnulinux-linux-fallback-UUID' {
	linux /vmlinuz-linux
}
EOF
printf "if [ x\$grub_platform = xefi ]; then\n\tmenuentry 'UEFI Firmware Settings' \$menuentry_id_option 'uefi-firmware' {\n\t\tfwsetup\n\t}\nfi\nsubmenu 'It'\\\\''s mine' {\n\tmenuentry 'inner' { true; }\n}\n" >"$work/rest.cfg"
"$renderer" grub-linux --linux-cfg "$work/linux.cfg" --rest-cfg "$work/rest.cfg" --entries "$work/entries" >"$work/menu.cfg"
[[ $(paste -sd '|' "$work/entries") == "linux|linux (fallback)|UEFI Firmware Settings|It's mine" ]] ||
  fail "unexpected entry names: $(paste -sd '|' "$work/entries")"
grep -F "gnulinux-simple-" "$work/menu.cfg" >/dev/null && fail "simple entry duplicates the newest kernel"
grep -F "menuentry 'linux' --class os \$menuentry_id_option 'gnulinux-linux-advanced-UUID' {" "$work/menu.cfg" >/dev/null ||
  fail "entry ids are not preserved"
grep -Fx 'function gfxmode { set gfxpayload="${1}"; }' "$work/menu.cfg" >/dev/null || fail "10_linux helpers dropped"
pass "GRUB entries are named like Limine's tree and keep their ids"

# --- install the shared files as the installer does ------------------------
share=/usr/share/omarchy/boot-theme
install -d "$share/grub.d" /usr/share/plymouth/themes/omarchy
install -m 0755 "$renderer" "$root/boot-theme/omarchy-boot-theme-apply" "$share/"
install -m 0644 "$root/boot-theme/logo-finals.txt" "$share/"
install -m 0755 "$root/boot-theme/grub.d/"* "$share/grub.d/"
cp "$work/beams.plymouth" /usr/share/plymouth/themes/omarchy/omarchy.plymouth
install -m 0644 "$mark" /usr/share/plymouth/themes/omarchy/boot-mark.png
snapshot() { find /boot /etc -xdev \( -path /etc/pacman.d/gnupg -prune \) -o -print 2>/dev/null | sort; }

# --- Limine apply / reapply / remove ---------------------------------------
install -d /boot
{ cat "$root/tests/fixtures/omarchy-limine.conf"; printf '\n/+Omarchy\n  //linux\n    protocol: linux\n'; } >/boot/limine.conf
cp /boot/limine.conf "$work/limine.original"
snapshot >"$work/before-limine"
"$share/omarchy-boot-theme-apply" >/dev/null 2>&1
[[ $(head -n 1 /boot/limine.conf) == "# >>> omarchy-boot-theme (managed by omarchy-boot-theme-apply)" ]] ||
  fail "theme block is not first in limine.conf"
"$share/omarchy-boot-theme-apply" >/dev/null 2>&1
[[ $(grep -c '^# >>> omarchy-boot-theme' /boot/limine.conf) == 1 ]] || fail "reapply duplicated the theme block"
tail -n "$(wc -l <"$work/limine.original")" /boot/limine.conf | cmp -s - "$work/limine.original" ||
  fail "the rest of limine.conf changed"
[[ -s /boot/omarchy-boot-theme/boot-theme.png && -s /boot/omarchy-boot-theme/boot-menu.png &&
  -s /boot/omarchy-boot-theme/boot-font-8x16.bin ]] ||
  fail "Limine assets missing from the ESP"
# The detected resolution depends on the container's view of /sys: compare
# with a direct render at that resolution from the theme's mark.
res=$(sed -n 's/^interface_resolution: //p' /boot/limine.conf | head -n 1)
"$renderer" limine --width "${res%x*}" --height "${res#*x}" --limine-conf "$work/limine.conf" \
  --mark /usr/share/plymouth/themes/omarchy/boot-mark.png --out "$work/applied-ref" 2>"$work/applied-ref.log"
grep -F " mark=" "$work/applied-ref.log" >/dev/null || fail "reference render at $res has no mark"
cmp -s /boot/omarchy-boot-theme/boot-theme.png "$work/applied-ref/boot-theme.png" ||
  fail "applied Limine wallpaper at $res does not carry the theme's mark: $(png_probe bbox /boot/omarchy-boot-theme/boot-theme.png)"
"$share/omarchy-boot-theme-apply" --remove >/dev/null 2>&1
cmp -s /boot/limine.conf "$work/limine.original" || fail "remove did not restore limine.conf byte for byte"
snapshot >"$work/after-limine"
comm -23 "$work/before-limine" "$work/after-limine" | grep -v '^/var' >"$work/lost" || true
[[ ! -s $work/lost ]] || fail "Limine apply/remove deleted: $(paste -sd ' ' "$work/lost")"
rm -f /boot/limine.conf
pass "Limine: block first, idempotent, remove restores limine.conf exactly, nothing else deleted"

# --- GRUB apply / remove with a real grub-mkconfig -------------------------
printf '#!/bin/sh\ncase "$*" in *fs_uuid*) echo 0000-TEST ;; *--target=fs*) echo ext2 ;; *partmap*) echo gpt ;; *abstraction*) ;; *) echo /dev/vda2 ;; esac\n' >/usr/bin/grub-probe
chmod +x /usr/bin/grub-probe
for k in linux linux-lts; do : >"/boot/vmlinuz-$k"; : >"/boot/initramfs-$k.img"; done
install -d /boot/grub
grub-mkconfig -o /boot/grub/grub.cfg >/dev/null 2>&1
cp /boot/grub/grub.cfg "$work/grub.original"
snapshot >"$work/before-grub"
"$share/omarchy-boot-theme-apply" >"$work/grub-apply.log" 2>&1 || { cat "$work/grub-apply.log" >&2; fail "GRUB apply failed"; }
grub-script-check /boot/grub/grub.cfg || fail "generated grub.cfg does not parse"
grep -F 'set theme=' /boot/grub/grub.cfg >/dev/null || fail "grub.cfg does not load the theme"
grep -E "^menuentry 'linux' " /boot/grub/grub.cfg >/dev/null || fail "kernel entries are not renamed"
[[ $(grep -cx 'if false; then' /boot/grub/grub.cfg) == 1 ]] || fail "stock 10_linux output is not disabled exactly once"
for f in theme.txt background.png select_c.png omarchy-boot.pf2; do
  [[ -s /boot/grub/themes/omarchy/$f ]] || fail "theme file $f missing"
done
compgen -G '/run/omarchy-boot-theme.*' >/dev/null && fail "grub.d marker left behind"
"$share/omarchy-boot-theme-apply" --remove >/dev/null 2>&1
grep -F 'omarchy-boot-theme' /boot/grub/grub.cfg >/dev/null && fail "theme still referenced after remove"
[[ ! -e /etc/default/grub.d/omarchy-boot-theme.cfg && ! -e /etc/grub.d/09_omarchy-boot-theme ]] ||
  fail "remove left managed GRUB files"
snapshot >"$work/after-grub"
comm -23 "$work/before-grub" "$work/after-grub" >"$work/lost" || true
[[ ! -s $work/lost ]] || fail "GRUB apply/remove deleted: $(paste -sd ' ' "$work/lost")"
pass "GRUB: themed tree with real grub-mkconfig, remove restores the stock menu, nothing else deleted"
printf 'fixtures retained in %s\n' "$work"
