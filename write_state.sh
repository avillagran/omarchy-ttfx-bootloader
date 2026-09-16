#!/bin/sh
# Write one setting into the plugin state.json, preserving the rest.
# Usage:
#   write_state.sh running=1|0
#   write_state.sh audio=1|0
#   write_state.sh effect=matrix        (set active effect)
#   write_state.sh effect+fire          (enable effect in the rotation list)
#   write_state.sh effect-fire          (disable it)
#   write_state.sh intensity=0..10
#   write_state.sh speed=1..100        (fifths of normal speed: 5=1x, 100=20x)
#   write_state.sh intro_size=1..3      (intro title ASCII-art scale, default 2)
#   write_state.sh show_fps=1|0         (FPS counter overlay)
#   write_state.sh boot_between=1|0     (show boot splash when rotating backgrounds)
#   write_state.sh rotate_secs=N        (seconds each background stays before rotating)
#   write_state.sh byline=Some text     (intro byline; empty = default)
#   write_state.sh restart              (bump restart counter -> replay intro)
set -e
STATE="${HOME}/.local/state/omarchy/audio-background/state.json"
mkdir -p "$(dirname "$STATE")"

# current values (defaults match the Rust binary)
running="true"; audio="true"; effect="matrix"; intensity="5"; speed="5"; byline=""; restart="0"; intro_size="2"; show_fps="false"
boot_between="true"; rotate_secs="20"; ttfx_text="OMARCHY"; resolution="1"; reactivity="2"; panel_opacity="0.6"
intro_beat_sync="true"; use_theme_colors="true"; transparent_background="false"
char_style="native"
boot_char_style="native"
effects="matrix rain wave bars donut fire starfield life"

if [ -f "$STATE" ]; then
  # `|| true` on every read: under `set -e`, a grep that finds no key returns
  # 1 and the `v=$(...)` assignment would abort the whole script before any
  # write. state.json may legitimately lack newer keys (show_fps, intro_size).
  v=$(grep -o '"running"[^,}]*' "$STATE" || true);   [ -n "$v" ] && running=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"audio"[^,}]*' "$STATE" || true);     [ -n "$v" ] && audio=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"show_fps"[^,}]*' "$STATE" || true);  [ -n "$v" ] && show_fps=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"boot_between"[^,}]*' "$STATE" || true); [ -n "$v" ] && boot_between=$(echo "$v" | grep -o '\(true\|false\)')
  v=$(grep -o '"effect"[[:space:]]*:[[:space:]]*"[^"]*"' "$STATE" || true); [ -n "$v" ] && effect=$(echo "$v" | sed 's/.*"effect"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
  v=$(grep -o '"intensity"[^,}]*' "$STATE" || true); [ -n "$v" ] && intensity=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"speed"[^,}]*' "$STATE" || true); [ -n "$v" ] && speed=$(echo "$v" | sed 's/^[^:]*:[[:space:]]*//; s/[[:space:]]*$//')
  v=$(grep -o '"restart"[^,}]*' "$STATE" || true);   [ -n "$v" ] && restart=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"intro_size"[^,}]*' "$STATE" || true); [ -n "$v" ] && intro_size=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"rotate_secs"[^,}]*' "$STATE" || true); [ -n "$v" ] && rotate_secs=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"resolution"[^,}]*' "$STATE" || true); [ -n "$v" ] && resolution=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"reactivity"[^,}]*' "$STATE" || true); [ -n "$v" ] && reactivity=$(echo "$v" | grep -o '[0-9]\+')
  v=$(grep -o '"intro_beat_sync"[^,}]*' "$STATE" || true); [ -n "$v" ] && intro_beat_sync=$(echo "$v" | grep -o 'true\|false')
  v=$(grep -o '"ttfx_text":"[^"]*"' "$STATE" || true); [ -n "$v" ] && ttfx_text=$(echo "$v" | sed 's/.*"ttfx_text":"\([^"]*\)".*/\1/')
  v=$(grep -o '"panel_opacity":[^,}]*' "$STATE" || true); [ -n "$v" ] && panel_opacity=$(echo "$v" | grep -o '[0-9.]\+' | head -1)
  v=$(grep -o '"byline"[^,}]*' "$STATE" || true);    [ -n "$v" ] && byline=$(echo "$v" | sed 's/.*"byline":"\([^"]*\)".*/\1/')
  # effects: read the array using grep for the whole bracket block, then
  # extract quoted names. Works for both compact and pretty-printed JSON.
  v=$(grep -o '"effects"[^]]*\]' "$STATE" || true)
  if [ -n "$v" ]; then
    effects=$(echo "$v" | sed 's/.*\[//; s/\].*//' | tr ',' '\n' | grep -o '"[a-z]*"' | tr -d '"' | tr '\n' ' ' | sed 's/ $//')
  fi
  v=$(grep -o '"use_theme_colors"[^,}]*' "$STATE" || true); [ -n "$v" ] && use_theme_colors=$(echo "$v" | grep -o 'true\|false')
  v=$(grep -o '"transparent_background"[^,}]*' "$STATE" || true); [ -n "$v" ] && transparent_background=$(echo "$v" | grep -o 'true\|false')
  v=$(grep -o '"char_style"[[:space:]]*:[[:space:]]*"[^"]*"' "$STATE" || true); [ -n "$v" ] && char_style=$(echo "$v" | sed 's/.*"char_style"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
  v=$(grep -o '"boot_char_style"[[:space:]]*:[[:space:]]*"[^"]*"' "$STATE" || true); [ -n "$v" ] && boot_char_style=$(echo "$v" | sed 's/.*"boot_char_style"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
fi

# Accept multiple key=value args in one invocation (single read-modify-write
# cycle — auto-degrade passes resolution=N and intro_size=M together).
for arg in "$@"; do
case "$arg" in
  running=*)  running=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  audio=*)    audio=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  show_fps=*) show_fps=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  boot_between=*) boot_between=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  use_theme_colors=*) use_theme_colors=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  transparent_background=*) transparent_background=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  intro_beat_sync=*) intro_beat_sync=$([ "${arg#*=}" = "1" ] || [ "${arg#*=}" = "true" ] && echo true || echo false) ;;
  char_style=*) char_style="${arg#*=}" ;;
  boot_char_style=*) boot_char_style="${arg#*=}" ;;
  intensity=*) intensity="${arg#*=}" ;;
  speed=*) speed="${arg#*=}" ;;
  intro_size=*) intro_size="${arg#*=}" ;;
  rotate_secs=*) rotate_secs="${arg#*=}" ;;
  resolution=*) resolution="${arg#*=}" ;;
  reactivity=*) reactivity="${arg#*=}" ;;
  ttfx_text=*) ttfx_text="${arg#*=}" ;;
  panel_opacity=*) panel_opacity="${arg#*=}" ;;
  effect=*)   effect="${arg#*=}" ;;
  byline=*)   byline="${arg#*=}" ;;
  restart)    restart=$((restart + 1)) ;;
  effect+*)
    name="${arg#*+}"
    case " $effects " in *" $name "*) ;; *) effects="$effects $name" ;; esac ;;
  effect-*)
    name="${arg#*-}"
    effects=$(echo " $effects " | sed "s/ $name / /" | xargs) ;;
esac
done

# Bound the persisted speed too, including when preserving an existing value.
# Strip leading zeros before arithmetic and avoid overflow on oversized input.
case "$speed" in
  -*) speed=1 ;;
  ''|*[!0-9]*) speed=5 ;;
  *)
    speed=$(printf '%s' "$speed" | sed 's/^0*//')
    case "$speed" in
      '') speed=1 ;;
      ?|??) : ;;
      *) speed=100 ;;
    esac ;;
esac

# never let the effects list go empty
[ -z "$effects" ] && effects="$effect"

# emit JSON array for effects
arr=$(echo "$effects" | tr ' ' '\n' | sed 's/.*/\"&"/' | paste -sd, -)

printf '{"running":%s,"audio":%s,"show_fps":%s,"boot_between":%s,"effect":"%s","effects":[%s],"intensity":%s,"speed":%s,"byline":"%s","restart":%s,"intro_size":%s,"rotate_secs":%s,"resolution":%s,"reactivity":%s,"ttfx_text":"%s","panel_opacity":%s,"intro_beat_sync":%s,"use_theme_colors":%s,"transparent_background":%s,"char_style":"%s","boot_char_style":"%s"}\n' \
  "$running" "$audio" "$show_fps" "$boot_between" "$effect" "$arr" "$intensity" "$speed" "$byline" "$restart" "$intro_size" "$rotate_secs" "$resolution" "$reactivity" "$ttfx_text" "$panel_opacity" "$intro_beat_sync" "$use_theme_colors" "$transparent_background" "$char_style" "$boot_char_style" > "$STATE.tmp.$$" \
  && mv "$STATE.tmp.$$" "$STATE"
