// ttfx-bg-rs: 100% Rust audio-reactive desktop background.
//
// Omarchy paints its desktop background as a gtk4-layer-shell bottom layer.
// We do the same, ONE layer per connected monitor. Inside each layer we embed
// a Vte terminal and draw the effect directly. The renderer is a child process
// (`ttfx-bg-rs --render <effect> <cols> <rows> ...`) per monitor.
//
// Config lives in the plugin's state.json (written by the panel / bar widget):
//   running, effect, effects[], intensity, audio, byline, restart
// The parent polls it every 700ms and rebuilds layers on change. A `restart`
// counter bump forces a rebuild (replays the intro). When `effects` has more
// than one entry, the parent rotates the active effect every 20s.
//
// Roadmap:
//   - Step 1: vendor ttfx's effect engine as a lib and call it here instead.
//   - Step 2: (done here) audio level from parec modulates flow speed/density.

mod wordmark;

use anyhow::{anyhow, bail, Result};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use vte4::{TerminalExt, TerminalExtManual};

// Thread-local palette shared between the watcher (run_render) and the effects
// (fx_matrix, etc.). The watcher writes the current palette here whenever the
// theme changes; effects read it every frame so color changes apply live
// without restarting the plugin.
thread_local! {
    static THEME_PALETTE: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

fn set_theme_palette(palette: Vec<String>) {
    THEME_PALETTE.with(|p| *p.borrow_mut() = palette);
}

fn get_theme_palette() -> Vec<String> {
    THEME_PALETTE.with(|p| p.borrow().clone())
}

// Throttled theme-color watcher: reads the theme files at most every `interval`
// and reports only actual changes. Without this, the frame loop did 2 file reads
// + TOML parse + string building at 60fps (~120 file opens/sec) — visible stutter
// under disk load. Theme changes are user-initiated and rare; 500ms detection
// latency is invisible.
struct ThemeWatcher {
    last_check: std::time::Instant,
    interval: Duration,
    cached: Vec<(String, String)>,
    key: String,
}

impl ThemeWatcher {
    fn new() -> Self {
        Self {
            last_check: std::time::Instant::now() - Duration::from_secs(60), // force first read
            interval: Duration::from_millis(500),
            cached: Vec::new(),
            key: String::new(),
        }
    }
    /// Returns the theme colors only when they changed since the last call
    /// (and at most once per `interval`). Cheap no-op otherwise.
    fn changed(&mut self) -> Option<&[(String, String)]> {
        if self.last_check.elapsed() < self.interval {
            return None;
        }
        self.last_check = std::time::Instant::now();
        let new = read_theme_colors();
        let new_key: String = new
            .iter()
            .map(|(k, v)| format!("{}:{}", k, v))
            .collect::<Vec<_>>()
            .join("|");
        if new_key != self.key {
            self.key = new_key;
            self.cached = new;
            Some(&self.cached)
        } else {
            None
        }
    }
}

// Slider updates are sampled at a bounded rate by the renderer. This avoids
// killing the PTY and replaying an effect whenever the user drags intensity.
struct IntensityWatcher {
    last_check: std::time::Instant,
    intensity: i64,
}

impl IntensityWatcher {
    fn current(&mut self, fallback: i64) -> i64 {
        if self.last_check.elapsed() >= Duration::from_millis(250) {
            self.last_check = std::time::Instant::now();
            self.intensity = read_config().intensity.clamp(0, 10);
        }
        if self.intensity == -1 {
            fallback.clamp(0, 10)
        } else {
            self.intensity
        }
    }
}

thread_local! {
    static INTENSITY_WATCHER: RefCell<IntensityWatcher> = RefCell::new(IntensityWatcher {
        last_check: std::time::Instant::now() - Duration::from_secs(60),
        intensity: -1,
    });
}

fn live_intensity(fallback: i64) -> i64 {
    INTENSITY_WATCHER.with(|watcher| watcher.borrow_mut().current(fallback))
}

// Speed is a separate live setting: visual intensity and audio reactivity must
// not be the only ways to change animation pace.
struct SpeedWatcher {
    last_check: std::time::Instant,
    speed: i64,
}

impl SpeedWatcher {
    fn current(&mut self, fallback: i64) -> i64 {
        if self.last_check.elapsed() >= Duration::from_millis(250) {
            self.last_check = std::time::Instant::now();
            self.speed = read_config().speed.clamp(1, 100);
        }
        if self.speed == -1 {
            fallback.clamp(1, 100)
        } else {
            self.speed
        }
    }
}

thread_local! {
    static SPEED_WATCHER: RefCell<SpeedWatcher> = RefCell::new(SpeedWatcher {
        last_check: std::time::Instant::now() - Duration::from_secs(60),
        speed: -1,
    });
}

fn live_speed(fallback: i64) -> i64 {
    SPEED_WATCHER.with(|watcher| watcher.borrow_mut().current(fallback))
}

// Stored in fifths of normal speed: existing default 5 = 1x, 100 = 20x.
// Both backends divide their ordinary (intensity/audio-adjusted) delay by this
// multiplier. Apply after native minimum-delay clamping so it cannot cap SPEED.
fn speed_multiplier(speed: i64) -> f64 {
    speed.clamp(1, 100) as f64 / 5.0
}

fn speed_delay(baseline: Duration, speed: i64) -> Duration {
    baseline.div_f64(speed_multiplier(speed))
}

// --- Auto-degrade: detect sustained FPS drops and coarsen the grid -------------
const AUTO_DEGRADE_WINDOW_SECS: f64 = 15.0;
const AUTO_DEGRADE_MIN_FPS: f64 = 10.0;

fn should_auto_degrade(frames: u32, elapsed_secs: f64) -> bool {
    elapsed_secs >= AUTO_DEGRADE_WINDOW_SECS
        && frames as f64 / elapsed_secs.max(0.001) < AUTO_DEGRADE_MIN_FPS
}

thread_local! {
    static AUTO_DEGRADE: RefCell<AutoDegrade> = RefCell::new(AutoDegrade::new());
    static AUTO_DEGRADE_ENABLED: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

fn set_auto_degrade_enabled(on: bool) {
    AUTO_DEGRADE_ENABLED.with(|e| e.set(on));
    if on {
        // The intro is intentionally unmeasured. Start a fresh 15-second window
        // when real background rendering begins, otherwise intro time makes the
        // first sample look artificially slow.
        AUTO_DEGRADE.with(|a| *a.borrow_mut() = AutoDegrade::new());
    }
}

struct AutoDegrade {
    last: Option<std::time::Instant>,
    win_frames: u32,
    win_since: std::time::Instant,
    escalated: bool,
}

impl AutoDegrade {
    fn new() -> Self {
        Self {
            last: None,
            win_frames: 0,
            win_since: std::time::Instant::now(),
            escalated: false,
        }
    }

    /// Call once per rendered frame. Degrade only after a full, deliberately
    /// slow 15-second observation window averaging below 10 real FPS.
    fn tick(&mut self, _intended_ms: f64) {
        let enabled = AUTO_DEGRADE_ENABLED.with(|e| e.get());
        let now = std::time::Instant::now();
        if enabled && self.last.is_some() {
            self.win_frames += 1;
        }
        self.last = Some(now);
        let win_elapsed = self.win_since.elapsed().as_secs_f64();
        if win_elapsed >= AUTO_DEGRADE_WINDOW_SECS {
            let fps = self.win_frames as f64 / win_elapsed.max(0.001);
            log_dbg(&format!(
                "auto-degrade: {:.1}s window, {} frames, average={:.1} FPS",
                win_elapsed, self.win_frames, fps
            ));
            if should_auto_degrade(self.win_frames, win_elapsed) && !self.escalated {
                self.escalated = true;
                self.win_frames = 0;
                self.win_since = std::time::Instant::now();
                escalate_resolution();
                return;
            }
            self.win_frames = 0;
            self.win_since = std::time::Instant::now();
        }
    }
}

fn claim_auto_degrade_step() -> bool {
    use std::io::{Read, Seek, Write};
    use std::os::fd::AsRawFd;
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let path = format!(
        "{base}/omarchy-audio-background-autodegrade-{}.lock",
        unsafe { libc::getuid() }
    );
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
    else {
        return false;
    };
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut previous = String::new();
    let _ = file.read_to_string(&mut previous);
    if now.saturating_sub(previous.trim().parse::<u64>().unwrap_or(0))
        < AUTO_DEGRADE_WINDOW_SECS as u64
    {
        log_dbg("auto-degrade: another monitor already escalated this 15s window");
        return false;
    }
    let _ = file.set_len(0);
    let _ = file.seek(std::io::SeekFrom::Start(0));
    write!(file, "{now}").is_ok()
}

/// One-step escalation: resolution+1 (cap 8) and intro_size -> 1 (smaller boot).
/// Patches state.json IN-PROCESS (no child process, no PATH lookup, no script
/// execution) with an atomic tmp+rename; the controller's 700ms poll sees the
/// change and rebuilds. Once per process: the rebuild respawns us.
fn escalate_resolution() {
    let cfg = read_config();
    if cfg.resolution >= 8 {
        log_dbg("auto-degrade: resolution already at max (8), not escalating");
        return;
    }
    if !claim_auto_degrade_step() {
        return;
    }
    let new_res = (cfg.resolution + 1).min(8);
    let new_intro = if cfg.intro_size > 1 {
        1
    } else {
        cfg.intro_size
    };
    log_dbg(&format!("auto-degrade: FPS drop detected -> resolution {res} -> {new_res}, intro_size {intro} -> {new_intro}",
        res = cfg.resolution, intro = cfg.intro_size));
    patch_state_nums(&[("resolution", new_res), ("intro_size", new_intro)]);
}

/// Replace `"key":<int>` in the state file, preserving every other byte
/// (including fields this binary doesn't know about, like panel_opacity).
/// Writes tmp+rename so a concurrent reader never sees a torn file.
fn patch_state_nums(patches: &[(&str, i64)]) {
    use std::io::Write;
    let path = config_path();
    let text =
        std::fs::read_to_string(&path).or_else(|_| std::fs::read_to_string(legacy_config_path()));
    let Ok(text) = text else {
        log_dbg("auto-degrade: state file unreadable, skipping patch");
        return;
    };
    let mut out = text;
    for (key, value) in patches {
        let needle = format!("\"{key}\":");
        if let Some(pos) = out.find(&needle) {
            let num_start = pos + needle.len();
            let rest = &out[num_start..];
            let num_len = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '-'))
                .unwrap_or(rest.len());
            out = format!("{}{}{}", &out[..num_start], value, &rest[num_len..]);
        }
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(mut f) = std::fs::File::create(&tmp) {
        if f.write_all(out.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

// Convert a tint index (0=default, 1..=palette.len()) to an ANSI color string.
fn tint_to_color(tint: u8, palette: &[String]) -> String {
    if tint == 0 || palette.is_empty() {
        String::new()
    } else {
        let idx = ((tint - 1) as usize).min(palette.len() - 1);
        palette[idx].clone()
    }
}
const DEFAULT_EFFECTS: [&str; 8] = [
    "matrix",
    "rain",
    "wave",
    "bars",
    "donut",
    "fire",
    "starfield",
    "life",
];
const DEFAULT_BYLINE: &str = "By x.com/avillagran";

#[derive(Clone, PartialEq)]
struct Config {
    running: bool,
    effect: String,
    effects: Vec<String>,
    intensity: i64,
    // Overall animation pace, separate from visual intensity and audio reactivity.
    speed: i64,
    audio: bool,
    byline: String,
    restart: i64,
    intro_size: i64,
    show_fps: bool,
    // Show the boot splash when ROTATING between backgrounds (manual restart always
    // shows it). Time each background stays on screen before rotating.
    boot_between: bool,
    rotate_secs: i64,
    // Canvas text for ttfx effects (they animate text), rendered large as ASCII art. And a
    // resolution scale: bigger cells => fewer cols/rows => less CPU on old machines.
    ttfx_text: String,
    resolution: i64,
    // How strongly ttfx effects react to audio: slider 0..5 (0 = off, 2 = normal, 5 =
    // strong). Scales the frame-pacing speed boost driven by the live volume/beat.
    reactivity: i64,
    // When true, the boot intro types one letter per audio beat (with a timeout
    // fallback if no music is playing), so the letters appear synced to the rhythm.
    intro_beat_sync: bool,
    // When true, use the active Omarchy theme colors for the effect palettes instead
    // of the built-in hardcoded colors. Requires omarchy-theme-current to be installed.
    use_theme_colors: bool,
    // When true, the background is transparent so the user's wallpaper shows through.
    // Off by default (opaque black background).
    transparent_background: bool,
    // Glyph treatment for the input text animated by ttfx; effect symbols stay native.
    char_style: String,
    // Glyph treatment for the boot intro's ASCII-art letters. It is deliberately
    // independent from the TTFX input text and hand-rolled effects.
    boot_char_style: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            running: true,
            effect: "matrix".into(),
            effects: DEFAULT_EFFECTS.iter().map(|s| s.to_string()).collect(),
            intensity: 5,
            speed: 5,
            audio: true,
            byline: String::new(),
            restart: 0,
            intro_size: 2,
            show_fps: false,
            boot_between: true,
            rotate_secs: 20,
            ttfx_text: "OMARCHY".into(),
            resolution: 1,
            reactivity: 2,
            intro_beat_sync: true,
            use_theme_colors: true,
            transparent_background: false,
            char_style: "native".into(),
            boot_char_style: "native".into(),
        }
    }
}

// state.json lives in ~/.local/state/omarchy/... — NOT in the plugin source dir.
// The shell runs `inotifywait -r` on ~/.config/omarchy/plugins and RELOADS the
// plugin on any file change there, which closed the open panel on every write
// (and respawned everything). Runtime state belongs in the XDG state dir, like
// the other plugins (control-panel-prefs.json etc.).
fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/state/omarchy/audio-background/state.json")
}

// Legacy location (inside the plugin dir) — read once for migration if the new
// path doesn't exist yet.
fn legacy_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json")
}

// Best-effort read of the panel's state.json. Missing/invalid => defaults.
// Tolerant of compact or pretty JSON. Independent `if`s on purpose: a compact
// line carries every key at once.
// Last config that parsed successfully. read_config() returns this instead of
// defaults when the state file is momentarily unreadable/empty (a non-atomic
// writer truncates-then-writes; even with an atomic mv, reading mid-flight used
// to return Config::default() = "matrix", and the controller interpreted that as
// the user switching effects -> visible rebuild/restart every write).
thread_local! {
    static LAST_GOOD_CONFIG: RefCell<Option<Config>> = RefCell::new(None);
}

fn read_config() -> Config {
    let text = std::fs::read_to_string(config_path())
        .or_else(|_| std::fs::read_to_string(legacy_config_path()));
    // Empty or brace-less content = torn write window (or a corrupt file); never
    // treat it as "user wants defaults".
    let text = match text {
        Ok(t) if t.contains('{') => t,
        Ok(t) => {
            log_dbg(&format!(
                "read_config: state file empty/partial ({} bytes) — keeping last good",
                t.len()
            ));
            return LAST_GOOD_CONFIG.with(|c| c.borrow().clone().unwrap_or_default());
        }
        Err(e) => {
            log_dbg(&format!(
                "read_config: no state file ({e}) — keeping last good"
            ));
            return LAST_GOOD_CONFIG.with(|c| c.borrow().clone().unwrap_or_default());
        }
    };
    let mut cfg = Config::default();
    if let Some(v) = json_bool(&text, "running") {
        cfg.running = v;
    }
    if let Some(v) = json_bool(&text, "audio") {
        cfg.audio = v;
    }
    if let Some(v) = json_str(&text, "effect") {
        cfg.effect = v;
    }
    if let Some(v) = json_str(&text, "byline") {
        cfg.byline = v;
    }
    if let Some(v) = json_num(&text, "intensity") {
        cfg.intensity = v;
    }
    if let Some(v) = json_num(&text, "speed") {
        cfg.speed = v.clamp(1, 100);
    }
    if let Some(v) = json_num(&text, "restart") {
        cfg.restart = v;
    }
    if let Some(v) = json_num(&text, "intro_size") {
        cfg.intro_size = v;
    }
    if let Some(v) = json_bool(&text, "show_fps") {
        cfg.show_fps = v;
    }
    if let Some(v) = json_bool(&text, "boot_between") {
        cfg.boot_between = v;
    }
    if let Some(v) = json_num(&text, "rotate_secs") {
        cfg.rotate_secs = v;
    }
    if let Some(v) = json_str(&text, "ttfx_text") {
        if !v.trim().is_empty() {
            cfg.ttfx_text = v;
        }
    }
    if let Some(v) = json_num(&text, "resolution") {
        cfg.resolution = v;
    }
    if let Some(v) = json_num(&text, "reactivity") {
        cfg.reactivity = v;
    }
    if let Some(v) = json_bool(&text, "intro_beat_sync") {
        cfg.intro_beat_sync = v;
    }
    if let Some(v) = json_bool(&text, "use_theme_colors") {
        cfg.use_theme_colors = v;
    }
    if let Some(v) = json_bool(&text, "transparent_background") {
        cfg.transparent_background = v;
    }
    if let Some(v) = json_str(&text, "char_style") {
        cfg.char_style = sanitize_char_style(&v).into();
    }
    if let Some(v) = json_str(&text, "boot_char_style") {
        cfg.boot_char_style = sanitize_char_style(&v).into();
    }
    if let Some(v) = json_str_list(&text, "effects") {
        if !v.is_empty() {
            cfg.effects = v;
        }
    }
    LAST_GOOD_CONFIG.with(|c| *c.borrow_mut() = Some(cfg.clone()));
    cfg
}

// --- tiny tolerant JSON field extractors (no serde dependency) ---
fn json_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let pos = text.find(&pat)?;
    let rest = &text[pos + pat.len()..];
    let colon = rest.find(':')?;
    Some(rest[colon + 1..].trim_start())
}

fn json_bool(text: &str, key: &str) -> Option<bool> {
    let v = json_value(text, key)?;
    if v.starts_with("true") {
        Some(true)
    } else if v.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn json_str(text: &str, key: &str) -> Option<String> {
    let v = json_value(text, key)?;
    let inner = v.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

fn json_num(text: &str, key: &str) -> Option<i64> {
    let v = json_value(text, key)?;
    let end = v.find(|c: char| !(c.is_ascii_digit() || c == '-'))?;
    v[..end].parse().ok()
}

fn json_str_list(text: &str, key: &str) -> Option<Vec<String>> {
    let v = json_value(text, key)?;
    let inner = v.strip_prefix('[')?;
    let end = inner.find(']')?;
    let body = &inner[..end];
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        match after.find('"') {
            Some(stop) => {
                out.push(after[..stop].to_string());
                rest = &after[stop + 1..];
            }
            None => break,
        }
    }
    Some(out)
}

fn sanitize_char_style(style: &str) -> &'static str {
    match style {
        "block" => "block",
        "dark" => "dark",
        "medium" => "medium",
        "light" => "light",
        "hash" => "hash",
        "dot" => "dot",
        "lower_o" => "lower_o",
        "upper_o" => "upper_o",
        _ => "native",
    }
}

fn styled_glyph(style: &str, ch: char) -> char {
    if ch == ' ' {
        return ch;
    }
    match sanitize_char_style(style) {
        "block" => '█',
        "dark" => '▓',
        "medium" => '▒',
        "light" => '░',
        "hash" => '#',
        "dot" => '·',
        "lower_o" => 'o',
        "upper_o" => 'O',
        _ => ch,
    }
}

// Parse a simple TOML colors.toml file from Omarchy themes.
// Only handles `key = "value"` lines (no sections, no tables). Returns a map
// of color name -> hex string (without the leading #).
fn parse_theme_colors(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_string();
            let val = line[eq + 1..].trim();
            let val = val.strip_prefix('"').unwrap_or(val);
            let val = val.strip_suffix('"').unwrap_or(val);
            if !key.is_empty() && !val.is_empty() {
                out.push((key, val.to_string()));
            }
        }
    }
    out
}

// Convert a hex color string (with or without leading #) to an ANSI 24-bit SGR code.
// Supports both 6-digit (#RRGGBB) and 3-digit (#RGB) forms.
fn hex_to_ansi(hex: &str) -> String {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    let bytes = if hex.len() == 6 {
        [
            u8::from_str_radix(&hex[..2], 16).unwrap_or(0),
            u8::from_str_radix(&hex[2..4], 16).unwrap_or(0),
            u8::from_str_radix(&hex[4..6], 16).unwrap_or(0),
        ]
    } else if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[..1], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[1..2], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[2..3], 16).unwrap_or(0);
        [r * 17, g * 17, b * 17]
    } else {
        [0, 0, 0]
    };
    format!("\x1b[38;2;{};{};{}m", bytes[0], bytes[1], bytes[2])
}

// Parse a hex color string (#RRGGBB) into (r, g, b) bytes. Returns None if invalid.
fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some((r, g, b))
    } else {
        None
    }
}

// Read the active Omarchy theme's colors.toml and return a map of color name -> ANSI code.
// Returns empty map if the theme file can't be read (graceful degradation to hardcoded colors).
fn read_theme_colors() -> Vec<(String, String)> {
    let theme_name_path =
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join(".local/state/omarchy/current/theme.name");
    let theme_name = match std::fs::read_to_string(&theme_name_path) {
        Ok(t) => t.trim().to_string(),
        Err(_) => return Vec::new(),
    };
    if theme_name.is_empty() {
        return Vec::new();
    }
    let omarchy_path = std::env::var("OMARCHY_PATH").unwrap_or_else(|_| {
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join(".local/share/omarchy")
            .to_string_lossy()
            .to_string()
    });
    let colors_path = format!("{}/themes/{}/colors.toml", omarchy_path, theme_name);
    match std::fs::read_to_string(&colors_path) {
        Ok(c) => parse_theme_colors(&c),
        Err(_) => Vec::new(),
    }
}

// Build an effect palette from the active theme colors.
fn theme_palette(theme: &[(String, String)], effect: &str) -> Vec<String> {
    let get = |name: &str, fallback: &str| -> String {
        for (k, v) in theme {
            if k == name {
                return hex_to_ansi(v);
            }
        }
        fallback.to_string()
    };
    let accent = get("accent", "\x1b[96m");
    let foreground = get("foreground", "\x1b[37m");
    let background = get("background", "\x1b[40m");
    let red = get("red", "\x1b[31m");
    let green = get("green", "\x1b[32m");
    let blue = get("blue", "\x1b[34m");
    let cyan = get("cyan", "\x1b[36m");
    let yellow = get("yellow", "\x1b[33m");
    let magenta = get("magenta", "\x1b[35m");
    let bright_red = get("bright_red", "\x1b[91m");
    let bright_green = get("bright_green", "\x1b[92m");
    let bright_cyan = get("bright_cyan", "\x1b[96m");
    let bright_blue = get("bright_blue", "\x1b[94m");
    let bright_yellow = get("bright_yellow", "\x1b[93m");
    let bright_magenta = get("bright_magenta", "\x1b[95m");

    match effect {
        "rain" => vec![accent.clone(), cyan.clone(), foreground.clone()],
        "matrix" => vec![accent.clone(), green.clone(), blue.clone()],
        "wave" => vec![accent.clone(), magenta.clone(), foreground.clone()],
        "bars" => vec![yellow.clone(), bright_yellow.clone(), foreground.clone()],
        "fire" => vec![red.clone(), bright_red.clone(), yellow.clone()],
        "life" => vec![green.clone(), bright_green.clone(), foreground.clone()],
        "starfield" => vec![foreground.clone(), bright_cyan.clone(), background.clone()],
        "donut" => vec![accent.clone(), cyan.clone(), yellow.clone()],
        _ => vec![foreground.clone(), green.clone(), blue.clone()],
    }
}

// Hardcoded fallback palettes (used when use_theme_colors is false OR theme unavailable).
fn hardcoded_palette(effect: &str) -> Vec<String> {
    match effect {
        "rain" => vec![
            "\x1b[96m".to_string(),
            "\x1b[36m".to_string(),
            "\x1b[37m".to_string(),
        ],
        "wave" => vec![
            "\x1b[95m".to_string(),
            "\x1b[35m".to_string(),
            "\x1b[37m".to_string(),
        ],
        "bars" => vec![
            "\x1b[93m".to_string(),
            "\x1b[33m".to_string(),
            "\x1b[37m".to_string(),
        ],
        "fire" => vec![
            "\x1b[91m".to_string(),
            "\x1b[93m".to_string(),
            "\x1b[31m".to_string(),
        ],
        "life" => vec![
            "\x1b[92m".to_string(),
            "\x1b[32m".to_string(),
            "\x1b[90m".to_string(),
        ],
        "starfield" => vec![
            "\x1b[97m".to_string(),
            "\x1b[37m".to_string(),
            "\x1b[90m".to_string(),
        ],
        "donut" => vec![
            "\x1b[96m".to_string(),
            "\x1b[95m".to_string(),
            "\x1b[93m".to_string(),
        ],
        _ => vec![
            "\x1b[97m".to_string(),
            "\x1b[92m".to_string(),
            "\x1b[32m".to_string(),
        ],
    }
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == key {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{key}=")) {
            return Some(v.to_string());
        }
    }
    None
}

#[cfg(test)]
#[test]
fn boot_scene_uses_native_input_and_embedding_settings() {
    let settings =
        BootSceneSettings::from_descriptor("[ttfx]\nBackgroundColor=112233\nTextColor=aabbcc\n")
            .unwrap();
    let scene = BootScene::new("vhstape", 42, settings).unwrap();
    assert_eq!(BOOT_LOGO.lines().count(), 20);
    assert!(BOOT_LOGO.lines().all(|line| line.chars().count() == 162));
    assert_eq!(
        BOOT_LOGO
            .chars()
            .filter(|glyph| *glyph != '\n')
            .collect::<std::collections::BTreeSet<_>>(),
        [' ', '█'].into_iter().collect()
    );
    let mut checksum = glib::Checksum::new(glib::ChecksumType::Sha256).unwrap();
    checksum.update(BOOT_LOGO.as_bytes());
    assert_eq!(
        checksum.string().as_deref(),
        Some("bea042e266960464d65309dde406e408838bf5a92a887b047416c601f9db387f")
    );
    assert_eq!(scene.ctx.terminal.config.canvas_width, 162);
    assert_eq!(scene.ctx.terminal.config.canvas_height, 20);
    assert_eq!(scene.ctx.terminal.config.frame_rate, 240);
    assert_eq!(boot_tick_contract(), (240, 2));
    assert!(scene.ctx.terminal.config.ignore_terminal_dimensions);
    assert!(!scene.ctx.final_text_bands);
    assert_eq!(scene.settings.background, 0x112233);
    assert_eq!(scene.cells.len(), 3240);
    assert!(BootSceneSettings::from_descriptor(
        "[ttfx]\nBackgroundColor=\nBackgroundColor=112233\n"
    )
    .is_err());
}

// Generated from omarchy/logo.txt by generate-embedded-logo.py and checked
// byte-for-byte against Plymouth's embedded-logo-v2.txt.
const BOOT_LOGO: &str = include_str!("../boot-logo-v2.txt");
const BOOT_COLUMNS: usize = 162;
const BOOT_ROWS: usize = 20;
const BOOT_FRAME_RATE: u32 = 240;
const BOOT_STEPS_PER_TICK: usize = 2;
const BOOT_GLYPH_WIDTH: u32 = 5;
const BOOT_GLYPH_HEIGHT: u32 = 10;
const BOOT_GLYPH_ATLAS: &[u8; 5230] = include_bytes!("../glyph-atlas.bin");

fn boot_glyph_index(symbol: char) -> usize {
    match symbol as u32 {
        codepoint @ 33..=126 => (codepoint - 33) as usize,
        codepoint @ 174..=451 => 94 + (codepoint - 174) as usize,
        codepoint @ 9472..=9598 => 372 + (codepoint - 9472) as usize,
        codepoint @ 9608..=9631 => 499 + (codepoint - 9608) as usize,
        _ => ('?' as usize) - 33,
    }
}

#[cfg(test)]
fn boot_tick_contract() -> (u32, usize) {
    (BOOT_FRAME_RATE, BOOT_STEPS_PER_TICK)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BootSceneSettings {
    background: u32,
    text: u32,
}

impl Default for BootSceneSettings {
    fn default() -> Self {
        // Explicit native plugin BACKGROUND_COLOR / LOGO_COLOR defaults. These
        // are not read from the desktop theme, which may have changed at login.
        Self {
            background: 0x0b0d10,
            text: 0xf4f4f5,
        }
    }
}

impl BootSceneSettings {
    fn load_for_handoff(handoff: &BootHandoff) -> Result<Self> {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let path = std::env::var_os("OMARCHY_PLYMOUTH_DESCRIPTOR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from("/usr/share/plymouth/themes/omarchy/omarchy.plymouth")
            });
        if !path.is_absolute() {
            bail!("boot descriptor must be absolute");
        }
        // Inspect every component without following symlinks. Root-owned and
        // non-writable ancestry makes the subsequent open stable to session users.
        for component in path.ancestors() {
            let metadata = std::fs::symlink_metadata(component)?;
            if metadata.file_type().is_symlink()
                || metadata.uid() != 0
                || metadata.mode() & 0o022 != 0
            {
                bail!("untrusted boot descriptor ancestry");
            }
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
            || metadata.len() > 16384
        {
            bail!("untrusted boot descriptor");
        }
        let mut input = String::new();
        file.take(16385).read_to_string(&mut input)?;
        if input.len() > 16384 {
            bail!("oversized boot descriptor");
        }
        let mut section = "";
        let mut values = std::collections::HashMap::new();
        for line in input.lines() {
            if line.starts_with('[') && line.ends_with(']') {
                section = &line[1..line.len() - 1];
            } else if let Some((key, value)) = line.split_once('=') {
                if matches!(
                    (section, key),
                    ("Plymouth Theme", "ModuleName")
                        | ("ttfx", "Effect" | "Seed" | "Mode" | "Enabled")
                ) && values.insert(key, value).is_some()
                {
                    bail!("duplicate boot descriptor selector");
                }
            }
        }
        if values.get("ModuleName") != Some(&"ttfx-plymouth")
            || values.get("Effect").copied() != Some(handoff.effect.as_str())
            || values.get("Seed").and_then(|v| v.parse::<u64>().ok()) != Some(handoff.seed)
            || values.get("Enabled") != Some(&"true")
            || !matches!(values.get("Mode"), Some(&"fixed" | &"random"))
        {
            bail!("boot scene descriptor does not match handoff");
        }
        Self::from_descriptor(&input)
    }

    fn from_descriptor(input: &str) -> Result<Self> {
        let mut settings = Self::default();
        let mut section = "";
        let mut seen = std::collections::HashSet::new();
        for line in input.lines() {
            if line.starts_with('[') && line.ends_with(']') {
                section = &line[1..line.len() - 1];
            } else if section == "ttfx" {
                if let Some((key, value)) = line.split_once('=') {
                    if !matches!(key, "BackgroundColor" | "TextColor") {
                        continue;
                    }
                    if !seen.insert(key) {
                        bail!("duplicate boot scene color");
                    }
                    // Match native parse_color: six hex digits, no # or 0x.
                    if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                        bail!("invalid boot scene color");
                    }
                    let color = u32::from_str_radix(value, 16)?;
                    if key == "BackgroundColor" {
                        settings.background = color;
                    } else {
                        settings.text = color;
                    }
                }
            }
        }
        Ok(settings)
    }
}

// Native state.h: cycle is informational; step zero includes first next_frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BootPhase {
    cycle: u64,
    step: u64,
    hold_final: bool,
}

impl BootPhase {
    fn parse(input: &str, effect: &str, seed: u64, settings: BootSceneSettings) -> Result<Self> {
        if input.len() > 512 || !input.ends_with('\n') {
            bail!("invalid phase size/termination");
        }
        let mut fields = std::collections::HashMap::new();
        for line in input.split_terminator('\n') {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| anyhow!("invalid phase record"))?;
            if !matches!(
                key,
                "version"
                    | "effect"
                    | "seed"
                    | "cycle"
                    | "step"
                    | "width"
                    | "height"
                    | "fps"
                    | "speed"
                    | "background"
                    | "foreground"
                    | "input"
                    | "playback"
            ) || fields.insert(key, value).is_some()
            {
                bail!("unknown/duplicate phase field");
            }
        }
        if fields.len() != 13 {
            bail!("incomplete phase");
        }
        let number = |key| -> Result<u64> {
            let value = fields[key];
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                bail!("invalid phase number");
            }
            Ok(value.parse()?)
        };
        let color = |key| -> Result<u32> {
            let value = fields[key];
            if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("invalid phase color");
            }
            Ok(u32::from_str_radix(value, 16)?)
        };
        if fields["version"] != "2"
            || fields["effect"] != effect
            || number("seed")? != seed
            || number("width")? != BOOT_COLUMNS as u64
            || number("height")? != BOOT_ROWS as u64
            || number("fps")? != u64::from(BOOT_FRAME_RATE)
            || number("speed")? != BOOT_STEPS_PER_TICK as u64
            || fields["input"] != "embedded-logo-v2"
            || !matches!(fields["playback"], "hold-final" | "continuous")
            || color("background")? != settings.background
            || color("foreground")? != settings.text
        {
            bail!("phase configuration mismatch");
        }
        let phase = Self {
            cycle: number("cycle")?,
            step: number("step")?,
            hold_final: fields["playback"] == "hold-final",
        };
        if phase.step > 10000 {
            bail!("phase replay step limit");
        }
        Ok(phase)
    }

    fn read(handoff: &BootHandoff) -> Result<Self> {
        use std::io::Read;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        // Fixed trusted tmpfs directory, opened once; openat pins its ancestry.
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open("/run")?;
        let metadata = directory.metadata()?;
        let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
        if metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
            || unsafe { libc::fstatfs(directory.as_raw_fd(), &mut fs) } != 0
            || fs.f_type != libc::TMPFS_MAGIC
        {
            bail!("untrusted phase directory");
        }
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c"omarchy-plymouth-handoff.state".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
            || metadata.len() > 512
            || unsafe { libc::fstatfs(file.as_raw_fd(), &mut fs) } != 0
            || fs.f_type != libc::TMPFS_MAGIC
        {
            bail!("untrusted phase file");
        }
        let mut input = String::new();
        file.take(513).read_to_string(&mut input)?;
        Self::parse(
            &input,
            &handoff.effect,
            handoff.seed,
            handoff.scene_settings,
        )
    }
}

#[cfg(test)]
#[test]
fn boot_phase_parser_matches_native_contract() {
    let text = "version=2\neffect=vhstape\nseed=42\ncycle=2\nstep=17\nwidth=162\nheight=20\nfps=240\nspeed=2\nbackground=0b0d10\nforeground=f4f4f5\ninput=embedded-logo-v2\nplayback=hold-final\n";
    let parse = |s: &str| BootPhase::parse(s, "vhstape", 42, BootSceneSettings::default());
    assert_eq!(
        parse(text).unwrap(),
        BootPhase {
            cycle: 2,
            step: 17,
            hold_final: true,
        }
    );
    for (old, new) in [
        ("version=2", "version=1"),
        ("seed=42", "seed=43"),
        ("step=17", "step=10001"),
        ("width=162", "width=161"),
        ("fps=240", "fps=30"),
        ("speed=2", "speed=3"),
        ("background=0b0d10", "background=ffffff"),
        ("input=embedded-logo-v2", "input=other"),
        ("step=17", "step=+17"),
        ("effect=vhstape", "effect=matrix"),
        ("foreground=f4f4f5", "foreground=000000"),
        ("playback=hold-final", "playback=invalid"),
    ] {
        assert!(parse(&text.replace(old, new)).is_err(), "{new}");
    }
    assert!(parse(&format!("{text}step=17\n")).is_err());
    assert!(parse(&format!("{text}unknown=1\n")).is_err());
    assert!(parse(&text.replace("cycle=2\n", "")).is_err());
    assert!(parse(&text.replace("playback=hold-final\n", "")).is_err());
    assert!(parse(&"x".repeat(513)).is_err());
}

// Bounded boot adapter over the existing TTFX engine, not ANSI parsing or font
// stretching. Its parameters match Plymouth's embedding; the ordinary desktop
// backend and all of its audio/theme/config behavior remain separate.
struct BootScene {
    effect_name: String,
    seed: u64,
    settings: BootSceneSettings,
    effect: Box<dyn ttfx::engine::effect::Effect>,
    ctx: ttfx::engine::ctx::EngineCtx,
    cells: Vec<Option<Rc<ttfx::engine::animation::CharacterVisual>>>,
    hold_final: bool,
}

impl BootScene {
    fn new(name: &str, seed: u64, settings: BootSceneSettings) -> Result<Self> {
        use clap::Parser;
        use ttfx::engine::ctx::{Clock, EngineCtx};
        let cli = ttfx::cli::Cli::try_parse_from(["ttfx", name])?;
        let mut config = cli.terminal_config();
        let mut effect = cli
            .effect
            .ok_or_else(|| anyhow!("missing boot effect"))?
            .build_effect();
        config.canvas_width = BOOT_COLUMNS as i64;
        config.canvas_height = BOOT_ROWS as i64;
        config.frame_rate = i64::from(BOOT_FRAME_RATE);
        config.ignore_terminal_dimensions = true;
        config.terminal_background_color =
            ttfx::utils::graphics::Color::from_hex(&format!("{:06x}", settings.background))
                .map_err(|e| anyhow!(e))?;
        let mut ctx = EngineCtx::new(
            BOOT_LOGO,
            config,
            ttfx_rng(Some(seed)),
            Clock::virtual_with_frame_rate(240),
        )?;
        // In particular, do not enable the desktop-only final_text_bands.
        effect.build(&mut ctx)?;
        effect
            .next_frame(&mut ctx)
            .ok_or_else(|| anyhow!("boot effect has no initial frame"))?;
        let mut scene = Self {
            effect_name: name.into(),
            seed,
            settings,
            effect,
            ctx,
            cells: vec![None; BOOT_COLUMNS * BOOT_ROWS],
            hold_final: false,
        };
        scene.snapshot();
        Ok(scene)
    }

    fn replay(&mut self, phase: BootPhase, deadline: std::time::Instant) -> Result<()> {
        if phase.step > 10000 {
            bail!("phase replay step limit");
        }
        // Creation already consumed next_frame once. Do not replay cycles or
        // advance for time spent between freeze and login. Never loop on EOF.
        for _ in 0..phase.step {
            if std::time::Instant::now() >= deadline {
                bail!("phase replay time limit");
            }
            if self.effect.next_frame(&mut self.ctx).is_none() {
                bail!("phase past end of cycle");
            }
        }
        if std::time::Instant::now() >= deadline {
            bail!("phase replay time limit");
        }
        self.snapshot();
        Ok(())
    }

    fn for_handoff(handoff: &BootHandoff) -> Result<Self> {
        let start = std::time::Instant::now();
        let mut scene = Self::new(&handoff.effect, handoff.seed, handoff.scene_settings)?;
        if let Some(phase) = handoff.phase {
            match scene.replay(phase, start + Duration::from_millis(750)) {
                Ok(()) => log_dbg(&format!("boot phase accepted effect={} seed={} cycle={} step={} width=162 height=20 fps=240 speed=2 background={:06x} foreground={:06x} input=embedded-logo-v2 replay_ms={}",
                    handoff.effect, handoff.seed, phase.cycle, phase.step, handoff.scene_settings.background,
                    handoff.scene_settings.text, start.elapsed().as_millis())),
                Err(error) => {
                    log_dbg(&format!("boot phase fallback: {error}; restarting initial frame"));
                    scene = Self::new(&handoff.effect, handoff.seed, handoff.scene_settings)?;
                }
            }
        }
        scene.hold_final = handoff.phase.is_some_and(|phase| phase.hold_final);
        Ok(scene)
    }

    fn draw(&self, cr: &gtk4::cairo::Context, width: i32, height: i32, scale: i32) -> Result<()> {
        let scale = scale.max(1) as u32;
        let geometry = BootGeometry::new(
            (width.max(0) as u32).saturating_mul(scale),
            (height.max(0) as u32).saturating_mul(scale),
            scale,
        );
        cr.save()?;
        cr.scale(1.0 / f64::from(scale), 1.0 / f64::from(scale));
        cr.set_antialias(gtk4::cairo::Antialias::None);
        let source = |rgb: u32| {
            cr.set_source_rgb(
                f64::from((rgb >> 16) & 255) / 255.0,
                f64::from((rgb >> 8) & 255) / 255.0,
                f64::from(rgb & 255) / 255.0,
            )
        };
        let rgb = |color: ttfx::utils::graphics::Color| {
            let (r, g, b) = color.rgb_ints();
            (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
        };
        source(self.settings.background);
        cr.paint()?;
        for row in 0..geometry.rows {
            for column in 0..geometry.columns {
                let Some(cell) = &self.cells[row as usize * BOOT_COLUMNS + column as usize] else {
                    continue;
                };
                let left = geometry.x_edge(column);
                let cell_width = geometry.x_edge(column + 1) - left;
                let mut y = geometry.y + row * geometry.cell_height;
                let mut h = geometry.cell_height;
                if cell_width == 0 || h == 0 {
                    continue;
                }
                let fg = cell.colors.and_then(|c| c.fg_color).map(rgb);
                let bg = cell.colors.and_then(|c| c.bg_color).map(rgb);
                let mut foreground = fg.unwrap_or(self.settings.text);
                let mut background = bg.unwrap_or(self.settings.background);
                if cell.reverse {
                    std::mem::swap(&mut foreground, &mut background);
                }
                if bg.is_some() || cell.reverse {
                    source(background);
                    cr.rectangle(
                        f64::from(geometry.x + left),
                        f64::from(y),
                        f64::from(cell_width),
                        f64::from(h),
                    );
                    cr.fill()?;
                }
                let symbol = cell.symbol.chars().next().unwrap_or('\0');
                if !cell.hidden && (fg.is_some() || cell.reverse) && symbol != ' ' && symbol != '\0'
                {
                    if symbol == '▀' {
                        h /= 2;
                    } else if symbol == '▄' {
                        y += h / 2;
                        h -= h / 2;
                    } else if symbol != '█' {
                        source(foreground);
                        let rows = &BOOT_GLYPH_ATLAS
                            [boot_glyph_index(symbol) * BOOT_GLYPH_HEIGHT as usize..]
                            [..BOOT_GLYPH_HEIGHT as usize];
                        for (glyph_y, bits) in rows.iter().copied().enumerate() {
                            let top = glyph_y as u32 * h / BOOT_GLYPH_HEIGHT;
                            let bottom = (glyph_y as u32 + 1) * h / BOOT_GLYPH_HEIGHT;
                            if bottom <= top {
                                continue;
                            }
                            let mut glyph_x = 0;
                            while glyph_x < BOOT_GLYPH_WIDTH {
                                while glyph_x < BOOT_GLYPH_WIDTH && bits & (1 << glyph_x) == 0 {
                                    glyph_x += 1;
                                }
                                if glyph_x == BOOT_GLYPH_WIDTH {
                                    break;
                                }
                                let run_start = glyph_x;
                                while glyph_x < BOOT_GLYPH_WIDTH && bits & (1 << glyph_x) != 0 {
                                    glyph_x += 1;
                                }
                                let run_left = run_start * cell_width / BOOT_GLYPH_WIDTH;
                                let run_right = glyph_x * cell_width / BOOT_GLYPH_WIDTH;
                                if run_right <= run_left {
                                    continue;
                                }
                                cr.rectangle(
                                    f64::from(geometry.x + left + run_left),
                                    f64::from(y + top),
                                    f64::from(run_right - run_left),
                                    f64::from(bottom - top),
                                );
                                cr.fill()?;
                            }
                        }
                        continue;
                    }
                    source(foreground);
                    cr.rectangle(
                        f64::from(geometry.x + left),
                        f64::from(y),
                        f64::from(cell_width),
                        f64::from(h),
                    );
                    cr.fill()?;
                }
            }
        }
        cr.restore()?;
        Ok(())
    }

    fn snapshot(&mut self) {
        // Same maximum (layer, character_id) winner as Terminal's cell buffer.
        // The desktop engine does not expose visit_render_cells yet. Read its
        // public structured arena; never parse/transform the generated ANSI.
        let mut winners = [None::<usize>; BOOT_COLUMNS * BOOT_ROWS];
        let arena = &self.ctx.terminal.arena;
        for (id, ch) in arena.iter().enumerate().filter(|(_, ch)| ch.is_visible) {
            let c = ch.motion.current_coord;
            if !(1..=BOOT_COLUMNS as i64).contains(&c.column)
                || !(1..=BOOT_ROWS as i64).contains(&c.row)
            {
                continue;
            }
            let slot = ((BOOT_ROWS as i64 - c.row) * BOOT_COLUMNS as i64 + c.column - 1) as usize;
            if winners[slot].is_none_or(|old| {
                (ch.layer, ch.character_id) > (arena[old].layer, arena[old].character_id)
            }) {
                winners[slot] = Some(id);
            }
        }
        for (cell, winner) in self.cells.iter_mut().zip(winners) {
            *cell = winner.map(|id| arena[id].animation.current_character_visual.clone());
        }
    }

    fn step(&mut self) -> Result<()> {
        if self.hold_final {
            return Ok(());
        }
        if self.effect.next_frame(&mut self.ctx).is_none() {
            *self = Self::new(&self.effect_name, self.seed, self.settings)?;
        } else {
            self.snapshot();
        }
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn boot_phase_replay_vhstape_initial_frame_and_continuation() {
    use std::time::Instant;
    let settings = BootSceneSettings::default();
    let mut live = BootScene::new("vhstape", 42, settings).unwrap();
    // Native vhstape seed 42: known intermediate snapshot, not elapsed time.
    for step in 0..=17 {
        let mut replay = BootScene::new("vhstape", 42, settings).unwrap();
        replay
            .replay(
                BootPhase {
                    cycle: u64::MAX,
                    step,
                    hold_final: false,
                },
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(
            format!("{:?}", replay.cells),
            format!("{:?}", live.cells),
            "step={step}"
        );
        if step == 17 {
            for _ in 0..3 {
                replay.step().unwrap();
                live.step().unwrap();
            }
            assert_eq!(format!("{:?}", replay.cells), format!("{:?}", live.cells));
        } else {
            live.step().unwrap();
        }
    }
    let mut replay = BootScene::new("vhstape", 42, settings).unwrap();
    assert!(replay
        .replay(
            BootPhase {
                cycle: 0,
                step: 1,
                hold_final: false,
            },
            Instant::now(),
        )
        .is_err());
    assert!(replay
        .replay(
            BootPhase {
                cycle: 0,
                step: 10001,
                hold_final: false,
            },
            Instant::now() + Duration::from_secs(1)
        )
        .is_err());
}

#[cfg(test)]
#[test]
fn boot_handoff_replays_subdivided_decrypt_phase_instead_of_restarting() {
    let settings = BootSceneSettings::default();
    let phase = BootPhase {
        cycle: 0,
        step: 1576,
        hold_final: false,
    };
    let handoff = BootHandoff {
        effect: "decrypt".into(),
        seed: 22321466108495961,
        ready_file: PathBuf::from("/run/unused-ready"),
        unhide_cursor_on_ready: false,
        scene_settings: settings,
        phase: Some(phase),
    };
    let accepted = BootScene::for_handoff(&handoff).unwrap();
    let mut expected = BootScene::new("decrypt", handoff.seed, settings).unwrap();
    expected
        .replay(phase, std::time::Instant::now() + Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        format!("{:?}", accepted.cells),
        format!("{:?}", expected.cells)
    );
}

#[cfg(test)]
#[test]
#[ignore = "requires disposable container with root-owned /run tmpfs fixture"]
fn boot_phase_secure_file_acceptance() {
    let mut handoff = BootHandoff {
        effect: "vhstape".into(),
        seed: 42,
        ready_file: PathBuf::from("/run/unused-ready"),
        unhide_cursor_on_ready: false,
        scene_settings: BootSceneSettings::default(),
        phase: None,
    };
    // No test writes to /run. The harness supplies/removes the trusted fixture.
    let phase = BootPhase::read(&handoff);
    if std::env::var_os("TTFX_TEST_REJECT_PHASE").is_some() {
        assert!(phase.is_err());
        eprintln!(
            "boot phase secure read rejected fixture: {}",
            phase.unwrap_err()
        );
        return;
    }
    handoff.phase = Some(phase.unwrap());
    assert_eq!(
        handoff.phase,
        Some(BootPhase {
            cycle: 2,
            step: 17,
            hold_final: true,
        })
    );
    let accepted = BootScene::for_handoff(&handoff).unwrap();
    let mut live = BootScene::new("vhstape", 42, handoff.scene_settings).unwrap();
    for _ in 0..17 {
        live.step().unwrap();
    }
    assert_eq!(format!("{:?}", accepted.cells), format!("{:?}", live.cells));
    handoff.phase = Some(BootPhase {
        cycle: 0,
        step: 10001,
        hold_final: false,
    });
    let fallback = BootScene::for_handoff(&handoff).unwrap();
    let first = BootScene::new("vhstape", 42, handoff.scene_settings).unwrap();
    assert_eq!(
        format!("{:?}", fallback.cells),
        format!("{:?}", first.cells)
    );
}

#[cfg(test)]
#[test]
fn boot_handoff_hold_final_never_restarts_the_effect() {
    let handoff = BootHandoff {
        effect: "vhstape".into(),
        seed: 42,
        ready_file: PathBuf::from("/run/unused-ready"),
        unhide_cursor_on_ready: false,
        scene_settings: BootSceneSettings::default(),
        phase: Some(BootPhase {
            cycle: 0,
            step: 17,
            hold_final: true,
        }),
    };
    let mut scene = BootScene::for_handoff(&handoff).unwrap();
    let frozen = format!("{:?}", scene.cells);
    for _ in 0..100 {
        scene.step().unwrap();
    }
    assert_eq!(format!("{:?}", scene.cells), frozen);
}

#[cfg(test)]
#[test]
fn boot_surface_paints_half_blocks_without_font_metrics() {
    use gtk4::cairo::{Context, Format, ImageSurface};
    use ttfx::engine::animation::{CharacterVisual, VisualParams};
    use ttfx::utils::graphics::{Color, ColorPair};
    let mut scene = BootScene::new("vhstape", 42, BootSceneSettings::default()).unwrap();
    scene.cells.fill(None);
    scene.cells[0] = Some(Rc::new(CharacterVisual::new(
        "▄",
        VisualParams {
            colors: Some(ColorPair {
                fg_color: Some(Color::from_hex("ff0000").unwrap()),
                bg_color: None,
            }),
            ..Default::default()
        },
    )));
    let mut surface = ImageSurface::create(Format::ARgb32, 1280, 800).unwrap();
    let cr = Context::new(&surface).unwrap();
    scene.draw(&cr, 1280, 800, 1).unwrap();
    drop(cr);
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().unwrap();
    let pixel = |x: usize, y: usize| {
        u32::from_ne_bytes(
            data[y * stride + x * 4..y * stride + x * 4 + 4]
                .try_into()
                .unwrap(),
        )
    };
    assert_eq!(pixel(227, 270), 0xff0b0d10);
    assert_eq!(pixel(227, 274), 0xff0b0d10);
    assert_eq!(pixel(227, 275), 0xffff0000);
    assert_eq!(pixel(231, 279), 0xffff0000);
    assert_eq!(pixel(232, 279), 0xff0b0d10);
    assert_eq!(pixel(227, 280), 0xff0b0d10);
    drop(data);

    let mut hidpi = ImageSurface::create(Format::ARgb32, 2560, 1600).unwrap();
    hidpi.set_device_scale(2.0, 2.0);
    let cr = Context::new(&hidpi).unwrap();
    scene.draw(&cr, 1280, 800, 2).unwrap();
    drop(cr);
    hidpi.flush();
    let stride = hidpi.stride() as usize;
    let data = hidpi.data().unwrap();
    let pixel = |x: usize, y: usize| {
        u32::from_ne_bytes(
            data[y * stride + x * 4..y * stride + x * 4 + 4]
                .try_into()
                .unwrap(),
        )
    };
    assert_eq!(pixel(454, 549), 0xff0b0d10);
    assert_eq!(pixel(454, 550), 0xffff0000);
    assert_eq!(pixel(463, 559), 0xffff0000);
    assert_eq!(pixel(464, 559), 0xff0b0d10);
    assert_eq!(pixel(454, 560), 0xff0b0d10);
}

#[cfg(test)]
#[test]
fn boot_surface_preserves_letter_detail() {
    use gtk4::cairo::{Context, Format, ImageSurface};
    use ttfx::engine::animation::{CharacterVisual, VisualParams};
    use ttfx::utils::graphics::{Color, ColorPair};
    let mut scene = BootScene::new("vhstape", 42, BootSceneSettings::default()).unwrap();
    scene.cells.fill(None);
    scene.cells[0] = Some(Rc::new(CharacterVisual::new(
        "A",
        VisualParams {
            colors: Some(ColorPair {
                fg_color: Some(Color::from_hex("ff0000").unwrap()),
                bg_color: None,
            }),
            ..Default::default()
        },
    )));
    let mut surface = ImageSurface::create(Format::ARgb32, 1280, 800).unwrap();
    let cr = Context::new(&surface).unwrap();
    scene.draw(&cr, 1280, 800, 1).unwrap();
    drop(cr);
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().unwrap();
    let mut foreground = 0;
    for y in 270..280_usize {
        for x in 227..232_usize {
            let pixel = u32::from_ne_bytes(
                data[y * stride + x * 4..y * stride + x * 4 + 4]
                    .try_into()
                    .unwrap(),
            );
            if pixel == 0xffff0000 {
                foreground += 1;
            }
        }
    }
    assert!(foreground > 0);
    assert!(foreground < 50, "letter was flattened into a solid cell");
}

// Physical-pixel port of native/plymouth-ttfx-plugin/state.c. Never use
// VTE font metrics or the ordinary desktop resolution preference here.
struct BootGeometry {
    columns: u32,
    rows: u32,
    cell_height: u32,
    x: u32,
    y: u32,
}

impl BootGeometry {
    fn new(width: u32, height: u32, scale: u32) -> Self {
        let columns = (BOOT_COLUMNS as u32).min(width);
        let rows = (BOOT_ROWS as u32).min(height);
        let native_cell_height = 10_u32.saturating_mul(scale.max(1));
        let cell_height = if rows == 0 || columns == 0 {
            0
        } else {
            native_cell_height
                .min(height / rows)
                .min((u64::from(width) * 100 / (u64::from(columns) * 51)) as u32)
        };
        let grid_width = columns * cell_height * 51 / 100;
        Self {
            columns,
            rows,
            cell_height,
            x: (width - grid_width) / 2,
            y: ((u64::from(height - cell_height * rows) * 9) / 20) as u32,
        }
    }

    fn x_edge(&self, column: u32) -> u32 {
        column * self.cell_height * 51 / 100
    }
}

#[cfg(test)]
#[test]
fn boot_geometry_matches_native_physical_pixels() {
    let g = BootGeometry::new(1280, 800, 1);
    assert_eq!(
        (g.x, g.y, g.cell_height, g.x_edge(162)),
        (227, 270, 10, 826)
    );
    assert_eq!((g.x_edge(8), g.x_edge(10), g.x_edge(12)), (40, 51, 61));
    let hidpi = BootGeometry::new(2560, 1600, 2);
    assert_eq!(
        (hidpi.x, hidpi.y, hidpi.cell_height, hidpi.x_edge(162)),
        (454, 540, 20, 1652)
    );
}

#[derive(Clone, Debug, PartialEq)]
struct BootHandoff {
    effect: String,
    seed: u64,
    ready_file: PathBuf,
    unhide_cursor_on_ready: bool,
    scene_settings: BootSceneSettings,
    phase: Option<BootPhase>,
}

#[derive(Clone, Debug, PartialEq)]
struct RenderOptions {
    effect: String,
    cols: usize,
    rows: usize,
    intensity: i64,
    speed: i64,
    audio: bool,
    byline: String,
    ttfx_text: String,
    reactivity: i64,
    intro_size: i64,
    cell_aspect: f32,
    show_fps: bool,
    show_intro: bool,
    intro_beat_sync: bool,
    use_theme_colors: bool,
    transparent_background: bool,
    char_style: String,
    boot_char_style: String,
    seed: Option<u64>,
    paint_socket: Option<PathBuf>,
}

fn strict_flag_value(args: &[String], key: &str) -> Result<Option<String>> {
    let mut found = None;
    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        if arg == key {
            if found.is_some() {
                bail!("duplicate argument: {key}");
            }
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| anyhow!("missing value for {key}"))?;
            if value.is_empty() {
                bail!("empty value for {key}");
            }
            found = Some(value.clone());
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix(&format!("{key}=")) {
            if found.is_some() {
                bail!("duplicate argument: {key}");
            }
            if value.is_empty() {
                bail!("empty value for {key}");
            }
            found = Some(value.to_string());
        }
        index += 1;
    }
    Ok(found)
}

fn strict_switch(args: &[String], key: &str) -> Result<bool> {
    let count = args
        .iter()
        .skip(1)
        .filter(|arg| arg.as_str() == key)
        .count();
    if count > 1 {
        bail!("duplicate argument: {key}");
    }
    if args
        .iter()
        .skip(1)
        .any(|arg| arg.starts_with(&format!("{key}=")))
    {
        bail!("{key} does not take a value");
    }
    Ok(count == 1)
}

fn validate_effect(effect: &str) -> Result<()> {
    if effect.is_empty()
        || !effect
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        bail!("invalid effect syntax: {effect:?}");
    }
    if !is_valid_effect(effect) {
        bail!("unknown effect: {effect}");
    }
    Ok(())
}

fn parse_u64_arg(value: &str, key: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("{key} must be an unsigned 64-bit integer");
    }
    value
        .parse::<u64>()
        .map_err(|_| anyhow!("{key} is outside the unsigned 64-bit range"))
}

fn parse_controller_handoff(args: &[String]) -> Result<Option<BootHandoff>> {
    let effect = strict_flag_value(args, "--boot-effect")?;
    let seed = strict_flag_value(args, "--boot-seed")?;
    let ready_file = strict_flag_value(args, "--ready-file")?;
    let unhide_cursor_on_ready = strict_switch(args, "--unhide-cursor-on-ready")?;

    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--unhide-cursor-on-ready" {
            index += 1;
        } else if ["--boot-effect", "--boot-seed", "--ready-file"].contains(&arg.as_str()) {
            index += 2;
        } else if ["--boot-effect", "--boot-seed", "--ready-file"]
            .iter()
            .any(|key| arg.starts_with(&format!("{key}=")))
        {
            index += 1;
        } else {
            bail!("unknown controller argument: {arg}");
        }
    }

    match (effect, seed, ready_file) {
        (None, None, None) if !unhide_cursor_on_ready => Ok(None),
        (Some(effect), Some(seed), Some(ready_file)) => {
            validate_effect(&effect)?;
            Ok(Some(BootHandoff {
                effect,
                seed: parse_u64_arg(&seed, "--boot-seed")?,
                ready_file: PathBuf::from(ready_file),
                unhide_cursor_on_ready,
                scene_settings: BootSceneSettings::default(),
                phase: None,
            }))
        }
        _ => bail!("--boot-effect, --boot-seed, and --ready-file must be supplied together"),
    }
}

fn parse_render_options(args: &[String]) -> Result<RenderOptions> {
    use std::collections::HashSet;

    let mut values = std::collections::HashMap::<&str, String>::new();
    let mut switches = HashSet::<&str>::new();
    let mut index = 1;
    while index < args.len() {
        let arg = args[index].as_str();
        if matches!(arg, "--render" | "--no-intro") {
            if !switches.insert(arg) {
                bail!("duplicate argument: {arg}");
            }
            index += 1;
            continue;
        }

        let (key, inline_value) = match arg.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (arg, None),
        };
        let takes_value = matches!(
            key,
            "--effect"
                | "--cols"
                | "--rows"
                | "--intensity"
                | "--speed"
                | "--audio"
                | "--byline"
                | "--ttfx-text"
                | "--reactivity"
                | "--intro-size"
                | "--cell-aspect"
                | "--show-fps"
                | "--intro-beat-sync"
                | "--use-theme-colors"
                | "--transparent-background"
                | "--char-style"
                | "--boot-char-style"
                | "--seed"
                | "--paint-socket"
        );
        if !takes_value {
            bail!("unknown renderer argument: {arg}");
        }
        if values.contains_key(key) {
            bail!("duplicate argument: {key}");
        }

        let value = match inline_value {
            Some(value) => value.to_string(),
            None => {
                index += 1;
                args.get(index)
                    .cloned()
                    .ok_or_else(|| anyhow!("missing value for {key}"))?
            }
        };
        values.insert(key, value);
        index += 1;
    }

    let effect = values.remove("--effect").unwrap_or_else(|| "matrix".into());
    validate_effect(&effect)?;
    let bool_or =
        |value: Option<&String>, default| value.map(|value| value == "1").unwrap_or(default);
    let seed = values
        .remove("--seed")
        .map(|value| parse_u64_arg(&value, "--seed"))
        .transpose()?;
    let paint_socket = values
        .remove("--paint-socket")
        .map(|value| {
            if value.is_empty() {
                bail!("empty value for --paint-socket");
            }
            Ok(PathBuf::from(value))
        })
        .transpose()?;
    let char_style = sanitize_char_style(
        values
            .get("--char-style")
            .map(String::as_str)
            .unwrap_or("native"),
    )
    .into();
    let boot_char_style = sanitize_char_style(
        values
            .get("--boot-char-style")
            .map(String::as_str)
            .unwrap_or("native"),
    )
    .into();

    Ok(RenderOptions {
        effect,
        cols: values
            .get("--cols")
            .and_then(|value| value.parse().ok())
            .unwrap_or(200),
        rows: values
            .get("--rows")
            .and_then(|value| value.parse().ok())
            .unwrap_or(100),
        intensity: values
            .get("--intensity")
            .and_then(|value| value.parse().ok())
            .unwrap_or(5),
        speed: values
            .get("--speed")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(5)
            .clamp(1, 100),
        audio: bool_or(values.get("--audio"), false),
        byline: values.remove("--byline").unwrap_or_default(),
        ttfx_text: values
            .remove("--ttfx-text")
            .unwrap_or_else(|| "OMARCHY".into()),
        reactivity: values
            .get("--reactivity")
            .and_then(|value| value.parse().ok())
            .unwrap_or(2),
        intro_size: values
            .get("--intro-size")
            .and_then(|value| value.parse().ok())
            .unwrap_or(2),
        cell_aspect: values
            .get("--cell-aspect")
            .and_then(|value| value.parse().ok())
            .unwrap_or(2.0),
        show_fps: bool_or(values.get("--show-fps"), false),
        show_intro: paint_socket.is_none() && !switches.contains("--no-intro"),
        intro_beat_sync: bool_or(values.get("--intro-beat-sync"), true),
        use_theme_colors: bool_or(values.get("--use-theme-colors"), true),
        transparent_background: bool_or(values.get("--transparent-background"), false),
        char_style,
        boot_char_style,
        seed,
        paint_socket,
    })
}

fn append_renderer_handoff_args(
    argv: &mut Vec<String>,
    handoff: Option<&BootHandoff>,
    paint_socket: Option<&Path>,
) {
    if let Some(handoff) = handoff {
        argv.extend(["--seed".into(), handoff.seed.to_string()]);
    }
    if let Some(path) = paint_socket {
        argv.extend(["--paint-socket".into(), path.to_string_lossy().into_owned()]);
    }
}

#[derive(Clone, Debug)]
struct ReadyMarker {
    path: PathBuf,
    effect: String,
    seed: u64,
}

static READY_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static PAINT_SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);
const PAINT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const HANDOFF_NORMALIZATION_DEADLINE: Duration = Duration::from_secs(8);
const SAVED_OFF_FADE_DURATION: Duration = Duration::from_millis(150);

#[derive(Default)]
struct HandoffNormalization {
    normalized: bool,
}

impl HandoffNormalization {
    fn resolve(&mut self, successor_ready: bool, elapsed: Duration, monitor_changed: bool) -> bool {
        if self.normalized
            || !(successor_ready || elapsed >= HANDOFF_NORMALIZATION_DEADLINE || monitor_changed)
        {
            return false;
        }
        self.normalized = true;
        true
    }

    fn handoff_active(&self) -> bool {
        !self.normalized
    }
}

impl ReadyMarker {
    fn new(path: PathBuf, effect: &str, seed: u64) -> Self {
        Self {
            path,
            effect: effect.to_owned(),
            seed,
        }
    }

    fn publish(&self) -> std::io::Result<()> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;

        let mut now: libc::timespec = unsafe { std::mem::zeroed() };
        if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut now) } != 0
            || now.tv_sec < 0
            || now.tv_nsec < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let ready_ns = (now.tv_sec as u64)
            .checked_mul(1_000_000_000)
            .and_then(|seconds| seconds.checked_add(now.tv_nsec as u64))
            .ok_or_else(|| std::io::Error::other("boot clock overflow"))?;
        let content = format!(
            "status=frame-ready\neffect={}\nseed={}\nready_ns={}\n",
            self.effect, self.seed, ready_ns
        );

        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = self.path.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "ready path has no file name",
            )
        })?;
        let mut temp_path = None;
        let mut temp_file = None;
        for _ in 0..32 {
            let counter = READY_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{}.{}.{}.tmp",
                file_name.to_string_lossy(),
                std::process::id(),
                counter
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&candidate)
            {
                Ok(file) => {
                    temp_path = Some(candidate);
                    temp_file = Some(file);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        let temp_path = temp_path.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate ready marker temporary file",
            )
        })?;
        let mut file = temp_file.expect("temporary path and file are created together");
        if let Err(error) = file
            .write_all(content.as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = std::fs::remove_file(&temp_path);
            return Err(error);
        }
        drop(file);

        let old = CString::new(temp_path.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "temporary path contains NUL",
            )
        })?;
        let new = CString::new(self.path.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "ready path contains NUL")
        })?;
        let rc = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                old.as_ptr(),
                libc::AT_FDCWD,
                new.as_ptr(),
                0,
            )
        };
        if rc == 0 {
            if let Ok(directory) = std::fs::File::open(parent) {
                let _ = directory.sync_all();
            }
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        let _ = std::fs::remove_file(&temp_path);
        Err(error)
    }
}

struct PaintCoordinator {
    expected: usize,
    painted: std::collections::HashSet<u32>,
    marker: Option<ReadyMarker>,
    unhide_cursor: bool,
    ready: bool,
}

impl PaintCoordinator {
    fn new(expected: usize, marker: ReadyMarker, unhide_cursor: bool) -> Self {
        Self {
            expected,
            painted: std::collections::HashSet::new(),
            marker: Some(marker),
            unhide_cursor,
            ready: false,
        }
    }

    fn after_paint(&mut self, monitor: u32) -> std::io::Result<bool> {
        if self.ready
            || self.expected == 0
            || monitor as usize >= self.expected
            || !self.painted.insert(monitor)
        {
            return Ok(false);
        }
        if self.painted.len() != self.expected {
            return Ok(false);
        }
        if let Some(marker) = self.marker.as_ref() {
            marker.publish()?;
        }
        self.marker = None;
        self.ready = true;
        if self.unhide_cursor {
            restore_cursor_bounded();
        }
        Ok(true)
    }

    fn painted_count(&self) -> usize {
        self.painted.len()
    }
    fn is_ready(&self) -> bool {
        self.ready
    }
}

struct PendingSuccessor {
    windows: Rc<RefCell<Vec<gtk4::ApplicationWindow>>>,
    coordinator: Rc<RefCell<PaintCoordinator>>,
    config: Config,
    failed: bool,
    started: std::time::Instant,
}

// No underlying wallpaper presentation protocol exists for saved-off.
// Its bounded deadline is explicitly degraded recovery, not readiness.
fn successor_wait_finished(ready: bool, elapsed: Duration) -> bool {
    ready || elapsed >= HANDOFF_NORMALIZATION_DEADLINE
}

type BackgroundGeometry = (String, i32, i32);

fn omarchy_background_mapped_in(layers: &str, expected: &[BackgroundGeometry]) -> bool {
    if expected.is_empty() {
        return false;
    }
    let mut monitor = None::<String>;
    let mut found = Vec::<BackgroundGeometry>::new();
    let mut bars = Vec::<(String, i32)>::new();
    for raw in layers.lines() {
        let line = raw.trim();
        if let Some(name) = line
            .strip_prefix("Monitor ")
            .and_then(|name| name.strip_suffix(':'))
        {
            monitor = Some(name.to_string());
            continue;
        }
        if !line.starts_with("Layer ") {
            continue;
        }
        let is_background = line.contains(", a: 1, namespace: omarchy-background,");
        let is_bar = line.contains(", a: 1, namespace: omarchy-bar,");
        if !is_background && !is_bar {
            continue;
        }
        let Some(coords) = line
            .split_once("xywh: ")
            .and_then(|(_, tail)| tail.split_once(',').map(|(coords, _)| coords))
        else {
            continue;
        };
        let values = coords
            .split_whitespace()
            .map(str::parse::<i32>)
            .collect::<std::result::Result<Vec<_>, _>>();
        let Ok(values) = values else { continue };
        if values.len() != 4 || values[0] != 0 || values[1] != 0 || values[2] <= 0 || values[3] <= 0
        {
            continue;
        }
        if let Some(name) = monitor.as_ref() {
            if is_background {
                found.push((name.clone(), values[2], values[3]));
            } else {
                bars.push((name.clone(), values[2]));
            }
        }
    }
    let mut expected = expected.to_vec();
    expected.sort();
    found.sort();
    let mut expected_bars = expected
        .iter()
        .map(|(name, width, _)| (name.clone(), *width))
        .collect::<Vec<_>>();
    expected_bars.sort();
    bars.sort();
    found == expected && bars == expected_bars
}

fn expected_background_geometry() -> Vec<BackgroundGeometry> {
    validated_monitors()
        .into_iter()
        .filter_map(|monitor| {
            let name = monitor.connector()?.to_string();
            let geometry = monitor.geometry();
            (geometry.width() > 0 && geometry.height() > 0).then_some((
                name,
                geometry.width(),
                geometry.height(),
            ))
        })
        .collect()
}

fn wallpaper_ready_markers_in(runtime: &Path, expected: &[BackgroundGeometry], uid: u32) -> bool {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let Ok(directory) = std::fs::symlink_metadata(runtime) else {
        return false;
    };
    if !directory.is_dir() || directory.uid() != uid || directory.mode() & 0o077 != 0 {
        return false;
    }
    expected.iter().all(|(name, _, _)| {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return false;
        }
        let path = runtime.join(format!("omarchy-wallpaper-ready.{name}"));
        let Ok(mut file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        else {
            return false;
        };
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        if !metadata.is_file()
            || metadata.uid() != uid
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.len() == 0
            || metadata.len() > 128
        {
            return false;
        }
        let mut record = String::new();
        file.read_to_string(&mut record).is_ok()
            && record == format!("status=frame-ready\nmonitor={name}\n")
    })
}

fn wallpaper_ready_markers(expected: &[BackgroundGeometry]) -> bool {
    let uid = unsafe { libc::getuid() };
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));
    wallpaper_ready_markers_in(&runtime, expected, uid)
}

fn probe_omarchy_background(expected: Vec<BackgroundGeometry>) -> bool {
    const MAX_OUTPUT: usize = 1024 * 1024;
    const TIMEOUT: Duration = Duration::from_millis(250);
    let mut child = match std::process::Command::new("/usr/bin/hyprctl")
        .arg("layers")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((MAX_OUTPUT + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = std::time::Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let Ok(Ok(bytes)) = reader.join() else {
        return false;
    };
    status.is_some_and(|status| status.success())
        && bytes.len() <= MAX_OUTPUT
        && String::from_utf8(bytes).ok().is_some_and(|layers| {
            omarchy_background_mapped_in(&layers, &expected) && wallpaper_ready_markers(&expected)
        })
}

const BACKGROUND_STABILITY_DURATION: Duration = Duration::from_millis(750);

#[derive(Default)]
struct BackgroundStability {
    since: Option<std::time::Instant>,
}

impl BackgroundStability {
    fn observe(&mut self, ready: bool, now: std::time::Instant) -> bool {
        if !ready {
            self.since = None;
            return false;
        }
        let since = self.since.get_or_insert(now);
        now.saturating_duration_since(*since) >= BACKGROUND_STABILITY_DURATION
    }
}

#[derive(Default)]
enum BackgroundProbeWorker {
    #[default]
    Idle,
    Running(std::sync::mpsc::Receiver<bool>),
}

#[derive(Default)]
struct BackgroundProbe {
    worker: BackgroundProbeWorker,
    stability: BackgroundStability,
}

impl BackgroundProbe {
    fn poll(&mut self, expected: Vec<BackgroundGeometry>) -> bool {
        let observation = match &mut self.worker {
            BackgroundProbeWorker::Running(receiver) => match receiver.try_recv() {
                Ok(value) => {
                    self.worker = BackgroundProbeWorker::Idle;
                    Some(value)
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.worker = BackgroundProbeWorker::Idle;
                    Some(false)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            },
            BackgroundProbeWorker::Idle => {
                let (sender, receiver) = std::sync::mpsc::channel();
                thread::spawn(move || {
                    let _ = sender.send(probe_omarchy_background(expected));
                });
                self.worker = BackgroundProbeWorker::Running(receiver);
                None
            }
        };
        observation.is_some_and(|ready| self.stability.observe(ready, std::time::Instant::now()))
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

fn saved_off_fade_opacity(elapsed: Duration) -> f64 {
    (1.0 - elapsed.as_secs_f64() / SAVED_OFF_FADE_DURATION.as_secs_f64()).clamp(0.0, 1.0)
}

fn saved_off_release_layer() -> Layer {
    Layer::Background
}

#[cfg(test)]
fn saved_off_surface_retained(elapsed: Duration) -> bool {
    // The bridge is closed (not retained) once the fade reaches zero opacity.
    saved_off_fade_opacity(elapsed) > 0.0
}

fn fade_saved_off_bridge(windows: Rc<RefCell<Vec<gtk4::ApplicationWindow>>>) {
    let started = std::time::Instant::now();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let opacity = saved_off_fade_opacity(started.elapsed());
        for window in windows.borrow().iter() {
            window.set_opacity(opacity);
        }
        if opacity > 0.0 {
            glib::ControlFlow::Continue
        } else {
            // Close the bridge once the fade completes. It used to stay mapped
            // at zero opacity because unmapping made Hyprland clear one output
            // frame — that visible glitch came from the full modeset i915 did
            // at the handoff, which is fixed now. Keeping the surface mapped
            // leaves the boot frame visible as a faint ghost when the window
            // opacity does not propagate to the layer-shell surface.
            for window in windows.borrow().iter() {
                window.close();
            }
            glib::ControlFlow::Break
        }
    });
}

#[cfg(test)]
#[test]
fn saved_off_release_requires_background_shell_bar_and_crossfade() {
    let expected = vec![("eDP-1".to_string(), 1280, 800)];
    let background_only = "Monitor eDP-1:\n\tLayer level 0 (background):\n\t\tLayer abc: xywh: 0 0 1280 800, a: 1, namespace: omarchy-background, pid: 12\n";
    let mapped = "Monitor eDP-1:\n\tLayer level 0 (background):\n\t\tLayer abc: xywh: 0 0 1280 800, a: 1, namespace: omarchy-background, pid: 12\n\tLayer level 2 (top):\n\t\tLayer def: xywh: 0 0 1280 26, a: 1, namespace: omarchy-bar, pid: 12\n";
    let partial = "Monitor eDP-1:\n\tLayer level 0 (background):\n\t\tLayer abc: xywh: 0 0 640 480, a: 1, namespace: omarchy-background, pid: 12\n";
    assert!(!omarchy_background_mapped_in("", &expected));
    assert!(!omarchy_background_mapped_in(partial, &expected));
    assert!(!omarchy_background_mapped_in(background_only, &expected));
    assert!(!omarchy_background_mapped_in(
        "Layer abc: xywh: 0 0 1280 800, a: 1, namespace: ttfx-bg, pid: 12",
        &expected,
    ));
    assert!(omarchy_background_mapped_in(mapped, &expected));
    let start = std::time::Instant::now();
    let mut stability = BackgroundStability::default();
    assert!(!stability.observe(true, start));
    assert!(!stability.observe(true, start + Duration::from_millis(749)));
    assert!(stability.observe(true, start + Duration::from_millis(750)));
    assert!(!stability.observe(false, start + Duration::from_millis(751)));
    assert!(!stability.observe(true, start + Duration::from_millis(2000)));
    let marker_dir =
        std::env::temp_dir().join(format!("ttfx-wallpaper-marker-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&marker_dir);
    std::fs::create_dir(&marker_dir).unwrap();
    std::fs::set_permissions(
        &marker_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let marker = marker_dir.join("omarchy-wallpaper-ready.eDP-1");
    std::fs::write(&marker, b"status=frame-ready\nmonitor=eDP-1\n").unwrap();
    std::fs::set_permissions(&marker, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    assert!(wallpaper_ready_markers_in(&marker_dir, &expected, unsafe {
        libc::getuid()
    }));
    std::fs::write(&marker, b"status=starting\nmonitor=eDP-1\n").unwrap();
    assert!(!wallpaper_ready_markers_in(
        &marker_dir,
        &expected,
        unsafe { libc::getuid() }
    ));
    std::fs::remove_dir_all(marker_dir).unwrap();
    assert_eq!(saved_off_fade_opacity(Duration::ZERO), 1.0);
    assert!(saved_off_fade_opacity(Duration::from_millis(75)) > 0.49);
    assert_eq!(saved_off_fade_opacity(Duration::from_millis(150)), 0.0);
    assert!(!saved_off_surface_retained(Duration::from_secs(60)));
    assert_eq!(saved_off_release_layer(), Layer::Background);
}

struct ControllerHandoff {
    handoff: Option<BootHandoff>,
    coordinator: Option<Rc<RefCell<PaintCoordinator>>>,
    normalization: HandoffNormalization,
    successor: Option<PendingSuccessor>,
    cursor_released: bool,
    background_probe: BackgroundProbe,
    started: std::time::Instant,
}

impl ControllerHandoff {
    fn new(handoff: Option<BootHandoff>, expected_monitors: usize) -> Self {
        let coordinator = handoff.as_ref().map(|handoff| {
            Rc::new(RefCell::new(PaintCoordinator::new(
                expected_monitors,
                ReadyMarker::new(handoff.ready_file.clone(), &handoff.effect, handoff.seed),
                handoff.unhide_cursor_on_ready,
            )))
        });
        Self {
            handoff,
            coordinator,
            normalization: HandoffNormalization::default(),
            successor: None,
            cursor_released: false,
            background_probe: BackgroundProbe::default(),
            started: std::time::Instant::now(),
        }
    }

    fn is_active(&self) -> bool {
        self.handoff.is_some() && self.normalization.handoff_active()
    }
}

#[derive(Default)]
struct SurfaceReadiness {
    armed: bool,
    content_seen: bool,
    reported: bool,
}

impl SurfaceReadiness {
    fn arm(&mut self) {
        self.armed = true;
        self.content_seen = false;
    }
    fn is_armed(&self) -> bool {
        self.armed
    }
    fn contents_changed(&mut self) -> bool {
        if !self.armed || self.reported {
            return false;
        }
        self.content_seen = true;
        true
    }
    fn after_paint(&mut self) -> bool {
        if !self.armed || !self.content_seen || self.reported {
            return false;
        }
        self.reported = true;
        true
    }
}

struct PreparedFrameHandshake {
    path: PathBuf,
    timeout: Duration,
    complete: bool,
}

impl PreparedFrameHandshake {
    fn new(path: PathBuf) -> Self {
        Self::with_timeout(path, PAINT_HANDSHAKE_TIMEOUT)
    }
    fn with_timeout(path: PathBuf, timeout: Duration) -> Self {
        Self {
            path,
            timeout,
            complete: false,
        }
    }
    fn before_first_frame(&mut self) -> std::io::Result<()> {
        if self.complete {
            return Ok(());
        }
        let mut stream = UnixStream::connect(&self.path)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        stream.write_all(b"PREPARED\n")?;
        let mut ack = [0u8; 4];
        stream.read_exact(&mut ack)?;
        if &ack != b"ACK\n" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid paint ACK",
            ));
        }
        self.complete = true;
        Ok(())
    }
}

fn gate_prepared_frame<T>(
    frame: Option<T>,
    handshake: &mut Option<PreparedFrameHandshake>,
) -> Result<Option<T>> {
    if frame.is_some() {
        if let Some(handshake) = handshake.as_mut() {
            handshake.before_first_frame()?;
        }
    }
    Ok(frame)
}

struct ParentPaintSocket {
    listener: UnixListener,
    path: PathBuf,
    deadline: std::time::Instant,
    expected_pid: Option<u32>,
    pending: Option<(UnixStream, u32)>,
    received: Vec<u8>,
    finished: bool,
}

impl ParentPaintSocket {
    fn bind(monitor: u32) -> std::io::Result<Self> {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self::bind_in(&base, monitor, PAINT_HANDSHAKE_TIMEOUT)
    }

    fn bind_in(base: &Path, monitor: u32, timeout: Duration) -> std::io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        for _ in 0..32 {
            let counter = PAINT_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "ttfx-p-{}-{monitor}-{counter}.sock",
                std::process::id()
            ));
            if path.as_os_str().as_bytes().len() >= 108 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "paint socket path too long",
                ));
            }
            match UnixListener::bind(&path) {
                Ok(listener) => {
                    if let Err(error) =
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    {
                        let _ = std::fs::remove_file(&path);
                        return Err(error);
                    }
                    if let Err(error) = listener.set_nonblocking(true) {
                        let _ = std::fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(Self {
                        listener,
                        path,
                        deadline: std::time::Instant::now() + timeout,
                        expected_pid: None,
                        pending: None,
                        received: Vec::new(),
                        finished: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "could not allocate paint socket",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn set_expected_pid(&mut self, pid: u32) {
        self.expected_pid = Some(pid);
    }

    fn poll(&mut self, surface: &mut SurfaceReadiness) -> std::io::Result<bool> {
        if self.finished {
            return Ok(true);
        }
        if std::time::Instant::now() >= self.deadline {
            self.cleanup();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "prepared-frame handshake timed out",
            ));
        }
        if self.pending.is_none() {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    let peer_pid = peer_pid(&stream)?;
                    self.pending = Some((stream, peer_pid));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        let (stream, peer_pid) = self.pending.as_mut().expect("accepted stream is retained");
        if self.received.len() < b"PREPARED\n".len() {
            let mut bytes = [0u8; 32];
            match stream.read(&mut bytes) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "paint socket closed before PREPARED",
                    ))
                }
                Ok(count) => self.received.extend_from_slice(&bytes[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        if self.received.len() < b"PREPARED\n".len() {
            return Ok(false);
        }
        if self.received.as_slice() != b"PREPARED\n" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid prepared-frame message",
            ));
        }
        let Some(expected_pid) = self.expected_pid else {
            return Ok(false);
        };
        if *peer_pid != expected_pid {
            self.pending = None;
            self.received.clear();
            return Ok(false);
        }
        surface.arm();
        stream.set_nonblocking(false)?;
        stream.set_write_timeout(Some(Duration::from_millis(100)))?;
        stream.write_all(b"ACK\n")?;
        self.cleanup();
        Ok(true)
    }

    fn cleanup(&mut self) {
        self.finished = true;
        self.pending = None;
        let _ = std::fs::remove_file(&self.path);
    }
}

fn authenticate_spawned_renderer(server: &mut ParentPaintSocket, pid: u32) {
    server.set_expected_pid(pid);
}

fn handle_renderer_spawn_result(
    server: Option<&Rc<RefCell<ParentPaintSocket>>>,
    result: Result<i32, String>,
) -> Result<i32, String> {
    let pid = result?;
    if pid <= 0 {
        return Err("renderer returned an invalid PID".to_string());
    }
    if let Some(server) = server {
        authenticate_spawned_renderer(&mut server.borrow_mut(), pid as u32);
    }
    Ok(pid)
}

fn renderer_spawn_diagnostic(effect: &str, cols: usize, rows: usize) -> String {
    format!("spawn_async: effect={effect} {cols}x{rows}")
}

fn ttfx_runtime_diagnostic(
    effect: &str,
    cols: usize,
    rows: usize,
    reactivity: i64,
    audio_enabled: bool,
) -> String {
    format!("run_ttfx: effect={effect} {cols}x{rows} reactivity={reactivity} audio_enabled={audio_enabled}")
}

fn render_runtime_diagnostic(
    effect: &str,
    cols: usize,
    rows: usize,
    audio: bool,
    with_intro: bool,
) -> String {
    format!("run_render: effect={effect} {cols}x{rows} audio={audio} with_intro={with_intro}")
}

fn peer_pid(stream: &UnixStream) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd;
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut libc::ucred as *mut libc::c_void,
            &mut length,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() || credentials.pid <= 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid paint peer credentials",
        ));
    }
    Ok(credentials.pid as u32)
}

impl Drop for ParentPaintSocket {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn should_spawn_layers(saved_running: bool, initial_handoff: bool) -> bool {
    saved_running || initial_handoff
}

fn restore_cursor_bounded() {
    let child = std::process::Command::new("hyprctl")
        .args(["keyword", "cursor:invisible", "false"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return;
    };
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Err(_) => return,
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

fn arm_parent_death_signal() {
    let parent = unsafe { libc::getppid() };
    if parent <= 1 {
        return;
    }
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
    // Parent may have died between getppid() and prctl().
    if unsafe { libc::getppid() } != parent {
        std::process::exit(0);
    }
}

fn try_controller_lock() -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let path = format!("{base}/omarchy-audio-background-{}.lock", unsafe {
        libc::getuid()
    });
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Some(file)
    } else {
        None
    }
}

fn selected_saved_effect(cfg: &Config) -> String {
    if is_valid_effect(&cfg.effect) && cfg.effects.contains(&cfg.effect) {
        cfg.effect.clone()
    } else {
        cfg.effects
            .first()
            .cloned()
            .unwrap_or_else(|| "matrix".into())
    }
}

fn normalize_handoff(
    handoff: &Rc<RefCell<ControllerHandoff>>,
    elapsed: Duration,
    monitor_changed: bool,
    windows: &Rc<RefCell<Vec<gtk4::ApplicationWindow>>>,
    self_bin: &str,
    cfg: &Config,
    last_cfg: &Rc<RefCell<Config>>,
    active_effect: &Rc<RefCell<String>>,
) -> bool {
    let mut state = handoff.borrow_mut();
    if !state.is_active() {
        return false;
    }
    if monitor_changed {
        state.background_probe.reset();
    }
    if elapsed >= HANDOFF_NORMALIZATION_DEADLINE && !state.cursor_released {
        restore_cursor_bounded();
        state.cursor_released = true;
    }
    // A missing config is not an implicit matrix selection. Keep the selected
    // live boot scene as the session default without writing preferences.
    let explicit_config = LAST_GOOD_CONFIG.with(|saved| saved.borrow().is_some());
    if !explicit_config {
        return false;
    }
    if state
        .successor
        .as_ref()
        .is_some_and(|pending| pending.config != *cfg)
        || monitor_changed
    {
        if let Some(pending) = state.successor.take() {
            for window in pending.windows.borrow_mut().drain(..) {
                window.close();
            }
        }
    }
    if !cfg.running {
        // Saved-off means the normal Omarchy background owns the final scene.
        // Never remove the live boot bridge on a clock alone: wait until Hyprland
        // reports the mapped full-screen background, then reveal it by alpha.
        if elapsed < HANDOFF_NORMALIZATION_DEADLINE
            || !state.background_probe.poll(expected_background_geometry())
        {
            return false;
        }
        if let Some(pending) = state.successor.take() {
            for window in pending.windows.borrow_mut().drain(..) {
                window.close();
            }
        }
        for window in windows.borrow().iter() {
            window.set_layer(saved_off_release_layer());
        }
        log_dbg("handoff saved-off: mapped omarchy background; crossfading bridge");
        fade_saved_off_bridge(windows.clone());
    } else {
        if state.successor.is_none() {
            let boot_ready = state
                .coordinator
                .as_ref()
                .is_some_and(|c| c.borrow().is_ready());
            if !boot_ready && elapsed < HANDOFF_NORMALIZATION_DEADLINE {
                return false;
            }
            let monitors = validated_monitors();
            let coordinator = Rc::new(RefCell::new(PaintCoordinator {
                expected: monitors.len(),
                painted: Default::default(),
                marker: None,
                unhide_cursor: false,
                ready: false,
            }));
            let next = Rc::new(RefCell::new(Vec::new()));
            rebuild_layers_for_monitors(
                &next,
                self_bin,
                cfg,
                &selected_saved_effect(cfg),
                false,
                None,
                Some(coordinator.clone()),
                monitors,
            );
            state.successor = Some(PendingSuccessor {
                windows: next,
                coordinator,
                config: cfg.clone(),
                started: std::time::Instant::now(),
                failed: false,
            });
            return false;
        }
        let pending = state.successor.as_mut().unwrap();
        if pending.failed {
            return false;
        }
        let ready = pending.coordinator.borrow().is_ready();
        if !successor_wait_finished(ready, pending.started.elapsed()) {
            return false;
        }
        if !ready {
            // Bound preparation without exposing an unpainted/failed candidate.
            // Keep the live bridge; a changed config/monitor set permits retry.
            log_dbg("successor paint deadline: retaining live boot surface (degraded)");
            for window in pending.windows.borrow_mut().drain(..) {
                window.close();
            }
            pending.failed = true;
            return false;
        }
        let pending = state.successor.take().unwrap();
        for window in pending.windows.borrow().iter() {
            window.set_layer(Layer::Bottom);
        }
        for window in windows.borrow_mut().drain(..) {
            window.close();
        }
        windows
            .borrow_mut()
            .extend(pending.windows.borrow_mut().drain(..));
    }
    state.normalization.resolve(true, elapsed, false);
    state.coordinator = None;
    state.handoff = None;
    *active_effect.borrow_mut() = selected_saved_effect(cfg);
    *last_cfg.borrow_mut() = cfg.clone();
    true
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--render") {
        return run_render(parse_render_options(&args)?);
    }

    // Drive a vendored ttfx effect directly (library bridge proof).
    if args.iter().any(|a| a == "--ttfx") {
        let effect = arg_value(&args, "--effect").unwrap_or_else(|| "matrix".into());
        let cols = arg_value(&args, "--cols")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(80);
        let rows = arg_value(&args, "--rows")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(24);
        let ttfx_text = arg_value(&args, "--ttfx-text").unwrap_or_else(|| "OMARCHY".into());
        let reactivity = arg_value(&args, "--reactivity")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(2);
        let state = AudioState::start(false); // standalone --ttfx: no audio capture
        let show_fps = arg_value(&args, "--show-fps")
            .map(|s| s == "1")
            .unwrap_or(false);
        return run_ttfx(
            &effect, cols, rows, &ttfx_text, &state, false, 5, 5, reactivity, false, "native",
            show_fps, 2.0, None, None,
        );
    }

    let mut boot_handoff = parse_controller_handoff(&args)?;
    if let Some(handoff) = boot_handoff.as_mut() {
        handoff.scene_settings = BootSceneSettings::load_for_handoff(handoff)?;
        // Read exactly once, before any monitor constructs its BootScene.
        handoff.phase = match BootPhase::read(handoff) {
            Ok(phase) => Some(phase),
            Err(error) => {
                log_dbg(&format!(
                    "boot phase fallback: {error}; restarting initial frame"
                ));
                None
            }
        };
    }
    arm_parent_death_signal();
    let _controller_lock = match try_controller_lock() {
        Some(lock) => lock,
        None => {
            log_dbg("controller already running; duplicate exits");
            return Ok(());
        }
    };

    gtk4::init()?;
    let self_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ttfx-bg-rs".into());

    let windows: Rc<RefCell<Vec<gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(Vec::new()));
    let cfg0 = read_config();
    let initial_monitors = validated_monitors();
    let controller_handoff = Rc::new(RefCell::new(ControllerHandoff::new(
        boot_handoff.clone(),
        initial_monitors.len(),
    )));
    let active_effect: Rc<RefCell<String>> = Rc::new(RefCell::new(
        // Only honor the saved effect if it's still enabled in the rotation set.
        // Otherwise the user disabled it — start with the first enabled one.
        if let Some(handoff) = boot_handoff.as_ref() {
            handoff.effect.clone()
        } else if is_valid_effect(&cfg0.effect) && cfg0.effects.contains(&cfg0.effect) {
            cfg0.effect.clone()
        } else {
            cfg0.effects
                .first()
                .cloned()
                .unwrap_or_else(|| "matrix".into())
        },
    ));
    let last_cfg: Rc<RefCell<Config>> = Rc::new(RefCell::new(cfg0));

    let initial_coordinator = controller_handoff.borrow().coordinator.clone();
    rebuild_layers_for_monitors(
        &windows,
        &self_bin,
        &last_cfg.borrow(),
        &active_effect.borrow(),
        false,
        boot_handoff.as_ref(),
        initial_coordinator,
        initial_monitors,
    );

    // At the recovery boundary restore the cursor and honor explicit saved-off.
    // Missing configuration remains the live selected boot scene, not matrix.
    {
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        let ae = active_effect.clone();
        let handoff = controller_handoff.clone();
        glib::timeout_add_local_once(HANDOFF_NORMALIZATION_DEADLINE, move || {
            let cfg = read_config();
            normalize_handoff(
                &handoff,
                HANDOFF_NORMALIZATION_DEADLINE,
                false,
                &w,
                &b,
                &cfg,
                &lc,
                &ae,
            );
        });
    }

    // Rebuild ONLY when the monitor geometry actually changed (count + size +
    // scale). items-changed also fires on spurious signals; rebuilding on those
    // was the source of visible flicker (screen clear + renderer respawn).
    if let Some(display) = gdk::Display::default() {
        let monitors = display.monitors();
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        let ae = active_effect.clone();
        let handoff = controller_handoff.clone();
        let last_sig: Rc<RefCell<String>> = Rc::new(RefCell::new(monitor_signature()));
        let rebuilding: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        monitors.connect_items_changed(move |_, _pos, _removed, _added| {
            let sig = monitor_signature();
            if sig == *last_sig.borrow() {
                return;
            }
            *last_sig.borrow_mut() = sig;
            // Recreate only the pending successor for the new exact monitor set;
            // keep the covering boot generation until replacement paint.
            if handoff.borrow().is_active() {
                let cfg = read_config();
                let handoff_elapsed = handoff.borrow().started.elapsed();
                normalize_handoff(&handoff, handoff_elapsed, true, &w, &b, &cfg, &lc, &ae);
                return;
            }
            if *rebuilding.borrow() {
                return;
            }
            *rebuilding.borrow_mut() = true;
            // Debounce: coalesce a burst of items-changed into one rebuild.
            let w = w.clone();
            let b = b.clone();
            let lc = lc.clone();
            let ae = ae.clone();
            let rebuilding = rebuilding.clone();
            let handoff = handoff.clone();
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                println!("monitors changed — rebuilding background layers");
                let cfg = read_config();
                let handoff_elapsed = handoff.borrow().started.elapsed();
                if !normalize_handoff(&handoff, handoff_elapsed, true, &w, &b, &cfg, &lc, &ae) {
                    *lc.borrow_mut() = cfg.clone();
                    rebuild_layers(&w, &b, &cfg, &ae.borrow(), true, None, None);
                }
                *rebuilding.borrow_mut() = false;
            });
        });
    }

    // Poll the panel's state.json; if anything changed, rebuild.
    {
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        let ae = active_effect.clone();
        let handoff = controller_handoff.clone();
        glib::timeout_add_local(Duration::from_millis(700), move || {
            let cfg = read_config();
            let handoff_elapsed = handoff.borrow().started.elapsed();
            if normalize_handoff(&handoff, handoff_elapsed, false, &w, &b, &cfg, &lc, &ae) {
                return glib::ControlFlow::Continue;
            }
            // Boot readiness is not successor readiness. Keep the bridge while
            // the candidate paints, or as the documented degraded fallback.
            if handoff.borrow().is_active() {
                return glib::ControlFlow::Continue;
            }
            let changed = *lc.borrow() != cfg;
            if changed {
                let old = lc.borrow().clone();
                log_dbg(&format!("poll changed: old effect={} effects={:?} -> new effect={} effects={:?} ae={} running={}", old.effect, old.effects, cfg.effect, cfg.effects, ae.borrow(), cfg.running));
                // If the active-effect selection changed, honor it; otherwise
                // keep rotating from where we are. Only honor if still enabled.
                if cfg.effect != old.effect
                    && is_valid_effect(&cfg.effect)
                    && cfg.effects.contains(&cfg.effect)
                {
                    *ae.borrow_mut() = cfg.effect.clone();
                } else if !cfg.effects.contains(&ae.borrow().clone()) {
                    // Active effect was disabled via toggle — jump to first enabled.
                    if let Some(first) = cfg.effects.first() {
                        *ae.borrow_mut() = first.clone();
                    }
                }
                *lc.borrow_mut() = cfg.clone();
                // Rotation timing and intensity are applied by the running renderer.
                // Keep the layer alive for those controls: killing its PTY on every
                // slider tick was the visible "restart" users reported.
                let visual = cfg.running != old.running
                    || cfg.effect != old.effect
                    || cfg.effects != old.effects
                    || cfg.audio != old.audio
                    || cfg.byline != old.byline
                    || cfg.ttfx_text != old.ttfx_text
                    || cfg.restart != old.restart
                    || cfg.intro_size != old.intro_size
                    || cfg.show_fps != old.show_fps
                    || cfg.resolution != old.resolution
                    || cfg.intro_beat_sync != old.intro_beat_sync
                    || cfg.use_theme_colors != old.use_theme_colors
                    || cfg.transparent_background != old.transparent_background
                    || cfg.char_style != old.char_style
                    || cfg.boot_char_style != old.boot_char_style;
                // Boot Between Backgrounds controls every ordinary effect transition,
                // including a manual picker selection. Explicit restart and changes to
                // intro content still deliberately replay it.
                let intro = (cfg.effect != old.effect && cfg.boot_between)
                    || cfg.restart != old.restart
                    || cfg.intro_size != old.intro_size
                    || cfg.ttfx_text != old.ttfx_text
                    || cfg.byline != old.byline
                    || cfg.intro_beat_sync != old.intro_beat_sync
                    || cfg.boot_char_style != old.boot_char_style;
                if visual {
                    rebuild_layers(&w, &b, &cfg, &ae.borrow(), intro, None, None);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // Rotate through the enabled effects when more than one is enabled. Fires every
    // second and counts up to cfg.rotate_secs so the interval is live-configurable
    // from the panel (no respawn needed to change it). Rotating respects
    // boot_between for whether the splash replays.
    {
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        let ae = active_effect.clone();
        let handoff = controller_handoff.clone();
        let elapsed = std::rc::Rc::new(std::cell::Cell::new(0u64));
        glib::timeout_add_local(Duration::from_secs(1), move || {
            if handoff.borrow().is_active() {
                elapsed.set(0);
                return glib::ControlFlow::Continue;
            }
            let cfg = lc.borrow().clone();
            if cfg.running && cfg.effects.len() > 1 {
                // If the currently displayed effect was just disabled, switch immediately
                // instead of waiting for the full interval (otherwise a disabled
                // effect stays visible for up to rotate_secs).
                let cur = ae.borrow().clone();
                if !cfg.effects.contains(&cur) {
                    let next = cfg.effects[0].clone();
                    *ae.borrow_mut() = next.clone();
                    elapsed.set(0);
                    rebuild_layers(&w, &b, &cfg, &next, false, None, None);
                    return glib::ControlFlow::Continue;
                }
                let e = elapsed.get() + 1;
                if e >= cfg.rotate_secs.clamp(3, 3600) as u64 {
                    elapsed.set(0);
                    let next = match cfg.effects.iter().position(|x| *x == cur) {
                        Some(i) => cfg.effects[(i + 1) % cfg.effects.len()].clone(),
                        None => cfg.effects[0].clone(),
                    };
                    if next != cur {
                        *ae.borrow_mut() = next.clone();
                        rebuild_layers(&w, &b, &cfg, &next, cfg.boot_between, None, None);
                    }
                } else {
                    elapsed.set(e);
                }
            } else {
                elapsed.set(0);
            }
            glib::ControlFlow::Continue
        });
    }

    glib::MainLoop::new(None, false).run();
    Ok(())
}

// Signature of the current monitor set: count + geometry + scale. Rebuilds
// only happen when this actually changes.
fn monitor_signature() -> String {
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => return String::new(),
    };
    let monitors = display.monitors();
    let mut parts = Vec::new();
    for i in 0..monitors.n_items() {
        if let Some(m) = monitors.item(i).and_downcast::<gdk::Monitor>() {
            let g = m.geometry();
            parts.push(format!(
                "{}x{}@{},{}s{}",
                g.width(),
                g.height(),
                g.x(),
                g.y(),
                m.scale_factor()
            ));
        }
    }
    parts.join("|")
}

fn collect_valid_items<T>(raw_count: u32, mut item: impl FnMut(u32) -> Option<T>) -> Vec<T> {
    (0..raw_count).filter_map(&mut item).collect()
}

fn validated_monitors() -> Vec<gdk::Monitor> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let monitors = display.monitors();
    collect_valid_items(monitors.n_items(), |index| {
        monitors.item(index).and_downcast::<gdk::Monitor>()
    })
}

fn append_private_log(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "debug log is not a private regular file",
        ));
    }
    file.write_all(bytes)
}

fn debug_log_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("ttfx-bg-debug.log")
}

fn log_dbg(msg: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() % 86400)
        .unwrap_or(0);
    let h = ts / 3600;
    let m = (ts % 3600) / 60;
    let s = ts % 60;
    let line = format!("[{h:02}:{m:02}:{s:02}] {msg}\n");
    let _ = append_private_log(&debug_log_path(), line.as_bytes());
    eprintln!("{msg}");
}

fn rebuild_layers(
    windows: &Rc<RefCell<Vec<gtk4::ApplicationWindow>>>,
    self_bin: &str,
    cfg: &Config,
    effect: &str,
    show_intro: bool,
    handoff: Option<&BootHandoff>,
    coordinator: Option<Rc<RefCell<PaintCoordinator>>>,
) {
    rebuild_layers_for_monitors(
        windows,
        self_bin,
        cfg,
        effect,
        show_intro,
        handoff,
        coordinator,
        validated_monitors(),
    );
}

fn rebuild_layers_for_monitors(
    windows: &Rc<RefCell<Vec<gtk4::ApplicationWindow>>>,
    self_bin: &str,
    cfg: &Config,
    effect: &str,
    show_intro: bool,
    handoff: Option<&BootHandoff>,
    coordinator: Option<Rc<RefCell<PaintCoordinator>>>,
    monitors: Vec<gdk::Monitor>,
) {
    log_dbg(&format!("rebuild_layers: effect={effect} show_intro={show_intro} running={} audio={} intensity={} reactivity={} resolution={} rotate_secs={} boot_between={} effect_field={} effects={:?}", cfg.running, cfg.audio, cfg.intensity, cfg.reactivity, cfg.resolution, cfg.rotate_secs, cfg.boot_between, cfg.effect, cfg.effects));
    // Each window owns its exact renderer PID, including late async spawns.
    // Never kill all direct children: another generation may be preparing.
    for w in windows.borrow_mut().drain(..) {
        w.close();
    }
    // Toggled off: leave NO layer window at all. The desktop then shows the
    // default wallpaper instead of an empty black VTE covering the screen.
    if !should_spawn_layers(cfg.running, handoff.is_some()) {
        println!("running=false: no layer windows, desktop wallpaper visible");
        log_dbg("rebuild_layers: running=false, no windows spawned");
        return;
    }
    let n = monitors.len();
    println!(
        "found {n} monitor(s), running={} effect={effect}",
        cfg.running
    );
    for (monitor_id, monitor) in monitors.into_iter().enumerate() {
        let w = spawn_layer_for_monitor(
            &monitor,
            monitor_id as u32,
            self_bin,
            cfg,
            effect,
            show_intro,
            handoff,
            coordinator.clone(),
        );
        windows.borrow_mut().push(w);
    }
}

fn attach_boot_surface(
    window: &gtk4::ApplicationWindow,
    handoff: &BootHandoff,
    monitor_id: u32,
    coordinator: Option<Rc<RefCell<PaintCoordinator>>>,
) -> Result<()> {
    // Reconstruct the frozen native snapshot before mapping the opaque surface.
    let scene = Rc::new(RefCell::new(BootScene::for_handoff(handoff)?));
    let area = gtk4::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    let painted = Rc::new(std::cell::Cell::new(false));
    let armed = Rc::new(std::cell::Cell::new(false));
    let draw_scene = scene.clone();
    let draw_painted = painted.clone();
    area.set_draw_func(move |area, cr, width, height| {
        if let Err(error) = draw_scene
            .borrow()
            .draw(cr, width, height, area.scale_factor())
        {
            log_dbg(&format!("boot surface draw failed: {error}"));
            return;
        }
        if !armed.get() {
            if let Some(clock) = area.frame_clock() {
                armed.set(true);
                let painted = draw_painted.clone();
                let coordinator = coordinator.clone();
                clock.connect_after_paint(move |_| {
                    if !painted.replace(true) {
                        if let Some(coordinator) = &coordinator {
                            if let Err(error) = coordinator.borrow_mut().after_paint(monitor_id) {
                                log_dbg(&format!("boot surface ready marker failed: {error}"));
                            }
                        }
                    }
                });
            }
        }
    });
    window.set_child(Some(&area));
    let weak_area = area.downgrade();
    let mut failed = false;
    let timer = Rc::new(RefCell::new(Some(gtk4::glib::timeout_add_local(
        Duration::from_secs_f64(1.0 / f64::from(BOOT_FRAME_RATE)),
        move || {
            let Some(area) = weak_area.upgrade() else {
                return gtk4::glib::ControlFlow::Break;
            };
            // Hold the initial frame until its actual GTK after-paint. Advance
            // only in this timer, never in per-monitor draw callbacks.
            if painted.get() && !failed {
                for _ in 0..BOOT_STEPS_PER_TICK {
                    if let Err(error) = scene.borrow_mut().step() {
                        log_dbg(&format!("boot scene stopped on engine error: {error}"));
                        failed = true;
                        // Retain the last valid scene; never clear or loop errors.
                        return gtk4::glib::ControlFlow::Continue;
                    }
                }
                area.queue_draw();
            }
            gtk4::glib::ControlFlow::Continue
        },
    ))));
    window.connect_close_request(move |_| {
        if let Some(timer) = timer.borrow_mut().take() {
            timer.remove();
        }
        gtk4::glib::Propagation::Proceed
    });
    Ok(())
}

fn spawn_layer_for_monitor(
    monitor: &gdk::Monitor,
    monitor_id: u32,
    self_bin: &str,
    cfg: &Config,
    effect: &str,
    show_intro: bool,
    handoff: Option<&BootHandoff>,
    coordinator: Option<Rc<RefCell<PaintCoordinator>>>,
) -> gtk4::ApplicationWindow {
    let window = gtk4::ApplicationWindow::builder().title("ttfx-bg").build();

    window.init_layer_shell();
    window.set_namespace(Some("ttfx-bg"));
    // Prepare a successor underneath the still-live Bottom bridge. Both
    // generations stay mapped so VTE/GDK can report actual content + paint.
    window.set_layer(if coordinator.is_some() && handoff.is_none() {
        Layer::Background
    } else {
        Layer::Bottom
    });
    window.set_monitor(Some(monitor));
    for e in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);

    if let Some(handoff) = handoff {
        match attach_boot_surface(&window, handoff, monitor_id, coordinator) {
            Ok(()) => window.present(),
            Err(error) => log_dbg(&format!("boot surface creation failed: {error}")),
        }
        return window;
    }

    let term = vte4::Terminal::new();
    // Font size in *physical pixels* (not points) so the grid scales with the
    // panel resolution: GTK applies the monitor scale to point sizes, which on
    // a 4K/scaled display produced a handful of giant characters.
    let scale = monitor.scale_factor().max(1) as f64;
    // `resolution` scales the cell size: 1 = full-res grid (most CPU), higher =
    // bigger cells => fewer cols/rows => much less CPU (for old machines).
    let cell_px = (9.0 / scale * cfg.resolution.clamp(1, 8) as f64).max(3.0);
    let font = gtk4::gdk::pango::FontDescription::from_string(&format!("monospace {cell_px:.0}px"));
    term.set_font(Some(&font));
    term.set_scrollback_lines(0);
    term.set_hexpand(true);
    term.set_vexpand(true);
    // When transparent_background is enabled, make Vte's background transparent
    // so the user's wallpaper shows through the empty cells.
    if cfg.transparent_background {
        term.set_color_background(&gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    }
    window.set_child(Some(&term));
    let child_pid = Rc::new(std::cell::Cell::new(None::<i32>));
    let closed = Rc::new(std::cell::Cell::new(false));
    let close_pid = child_pid.clone();
    let close_flag = closed.clone();
    window.connect_close_request(move |_| {
        close_flag.set(true);
        if let Some(pid) = close_pid.take() {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        gtk4::glib::Propagation::Proceed
    });
    let exited_pid = child_pid.clone();
    term.connect_child_exited(move |_, _| {
        exited_pid.set(None);
    });

    if should_spawn_layers(cfg.running, handoff.is_some()) {
        let geo = monitor.geometry();
        // Measure the real cell size from the font metrics so the grid fills
        // the screen exactly regardless of DPI / scale factor.
        let font_size = font.size() as f64 / gtk4::pango::SCALE as f64;
        let ctx = term.pango_context();
        let metrics = ctx.metrics(Some(&font), None);
        let char_w = metrics.approximate_char_width() as f64 / gtk4::pango::SCALE as f64;
        let ascent = metrics.ascent() as f64 / gtk4::pango::SCALE as f64;
        let descent = metrics.descent() as f64 / gtk4::pango::SCALE as f64;
        let cw = if char_w > 0.0 {
            char_w
        } else {
            font_size * 0.6
        };
        let ch = if (ascent + descent) > 0.0 {
            ascent + descent
        } else {
            font_size * 1.2
        };
        let cols = ((geo.width() as f64) / cw).floor().max(80.0) as usize;
        let rows = ((geo.height() as f64) / ch).floor().max(24.0) as usize;
        // Cell aspect (height/width in px) so effects can draw true circles.
        let cell_aspect = if cw > 0.0 { ch / cw } else { 2.0 };
        let byline = if cfg.byline.trim().is_empty() {
            DEFAULT_BYLINE
        } else {
            cfg.byline.trim()
        }
        .to_string();
        let audio = if cfg.audio { "1" } else { "0" };
        let mut argv: Vec<String> = vec![
            self_bin.to_string(),
            "--render".into(),
            "--effect".into(),
            effect.to_string(),
            "--cols".into(),
            cols.to_string(),
            "--rows".into(),
            rows.to_string(),
            "--intensity".into(),
            cfg.intensity.to_string(),
            "--speed".into(),
            cfg.speed.to_string(),
            "--audio".into(),
            audio.into(),
            "--byline".into(),
            byline,
            "--ttfx-text".into(),
            cfg.ttfx_text.clone(),
            "--reactivity".into(),
            cfg.reactivity.to_string(),
            "--intro-size".into(),
            cfg.intro_size.to_string(),
            "--cell-aspect".into(),
            format!("{cell_aspect:.3}"),
            "--show-fps".into(),
            if cfg.show_fps { "1".into() } else { "0".into() },
            "--use-theme-colors".into(),
            if cfg.use_theme_colors {
                "1".into()
            } else {
                "0".into()
            },
            "--transparent-background".into(),
            if cfg.transparent_background {
                "1".into()
            } else {
                "0".into()
            },
            "--char-style".into(),
            cfg.char_style.clone(),
            "--boot-char-style".into(),
            cfg.boot_char_style.clone(),
        ];
        // A handoff never paints an intro: only the selected effect frame may arm readiness.
        if !show_intro || handoff.is_some() {
            argv.push("--no-intro".into());
        }
        // Pass intro_beat_sync as CLI arg so the renderer uses it
        let intro_beat_sync_arg = if cfg.intro_beat_sync { "1" } else { "0" };
        argv.push("--intro-beat-sync".into());
        argv.push(intro_beat_sync_arg.into());
        let state = coordinator
            .as_ref()
            .map(|_| Rc::new(RefCell::new(SurfaceReadiness::default())));
        let paint_server = if coordinator.is_some() {
            match ParentPaintSocket::bind(monitor_id) {
                Ok(server) => Some(Rc::new(RefCell::new(server))),
                Err(error) => {
                    log_dbg(&format!(
                        "paint socket setup failed for monitor {monitor_id}: {error}"
                    ));
                    None
                }
            }
        } else {
            None
        };
        let paint_path = paint_server
            .as_ref()
            .map(|server| server.borrow().path().to_path_buf());
        append_renderer_handoff_args(&mut argv, handoff, paint_path.as_deref());
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        // Pre-clear so Vte doesn't flash its "N by M cells" placeholder while
        // the renderer's PTY is still connecting. Use transparent background
        // when the option is enabled.
        if cfg.transparent_background {
            term.feed(b"\x1b[2J\x1b[H");
        } else {
            term.feed(b"\x1b[2J\x1b[H\x1b[40m");
        }
        if let (Some(coordinator), Some(state)) = (coordinator, state.as_ref()) {
            let state = state.clone();
            term.connect_contents_changed(move |terminal| {
                if !state.borrow_mut().contents_changed() {
                    return;
                }
                let Some(frame_clock) = terminal.frame_clock() else {
                    return;
                };
                let coordinator = coordinator.clone();
                let state = state.clone();
                frame_clock.connect_after_paint(move |_| {
                    if state.borrow_mut().after_paint() {
                        if let Err(error) = coordinator.borrow_mut().after_paint(monitor_id) {
                            log_dbg(&format!("desktop handoff ready marker failed: {error}"));
                        }
                    }
                });
            });
        }
        if let (Some(server), Some(state)) = (paint_server.as_ref(), state.as_ref()) {
            let server = server.clone();
            let state = state.clone();
            glib::timeout_add_local(Duration::from_millis(10), move || {
                let poll_result = {
                    let mut server = server.borrow_mut();
                    let mut state = state.borrow_mut();
                    server.poll(&mut state)
                };
                match poll_result {
                    Ok(true) => glib::ControlFlow::Break,
                    Ok(false) => glib::ControlFlow::Continue,
                    Err(error) => {
                        log_dbg(&format!(
                            "prepared-frame handshake failed for monitor {monitor_id}: {error}"
                        ));
                        server.borrow_mut().cleanup();
                        glib::ControlFlow::Break
                    }
                }
            });
        }
        if let Some(server) = paint_server.as_ref() {
            let cleanup = server.clone();
            window.connect_close_request(move |_| {
                cleanup.borrow_mut().cleanup();
                gtk4::glib::Propagation::Proceed
            });
            let cleanup = server.clone();
            term.connect_child_exited(move |_, _| cleanup.borrow_mut().cleanup());
        }
        let effect_for_log = effect.to_string();
        let spawn_server = paint_server.clone();
        log_dbg(&renderer_spawn_diagnostic(&effect_for_log, cols, rows));
        term.spawn_async(
            vte4::PtyFlags::DEFAULT,
            None,
            &argv_refs,
            &[],
            gtk4::glib::SpawnFlags::empty(),
            || {},
            2000,
            None::<&gtk4::gio::Cancellable>,
            move |res| match res {
                Ok(pid) => {
                    let raw_pid = pid.0 as i32;
                    if closed.get() {
                        unsafe {
                            libc::kill(raw_pid, libc::SIGTERM);
                        }
                        return;
                    }
                    child_pid.set(Some(raw_pid));
                    match handle_renderer_spawn_result(spawn_server.as_ref(), Ok(raw_pid)) {
                        Ok(_) => {
                            println!("renderer spawned ({cols}x{rows}, effect={effect_for_log})");
                            log_dbg(&format!("spawn ok: {effect_for_log} {cols}x{rows}"));
                        }
                        Err(error) => {
                            if let Some(server) = spawn_server.as_ref() {
                                server.borrow_mut().cleanup();
                            }
                            log_dbg(&format!("spawn PID authentication failed: {error}"));
                        }
                    }
                }
                Err(e) => {
                    if let Some(server) = spawn_server.as_ref() {
                        server.borrow_mut().cleanup();
                    }
                    eprintln!("spawn error: {e:?}");
                    log_dbg(&format!("spawn error {effect_for_log}: {e:?}"));
                }
            },
        );
    }

    window.present();
    window
}

// ---------------------------------------------------------------------------
// Renderer child process
// ---------------------------------------------------------------------------

// Shared audio state fed by the parec capture thread. Per-band spectrum (like
// the node@192.168.1.177 analyzer_light.py that reacted correctly): Goertzel
// per band with adaptive rolling-peak normalization, plus volume + beat.
const NBANDS: usize = 24;
const SAMPLE_RATE: f32 = 24000.0;
const CHUNK: usize = 1024; // samples per analysis frame (~43ms)

#[derive(Clone)]
struct AudioState {
    bands: Arc<Vec<AtomicU32>>, // per-band energy 0..1 (f32 bits)
    volume: Arc<AtomicU32>,     // overall volume 0..1
    beat: Arc<AtomicU32>,       // 1 shortly after a beat
}

impl AudioState {
    fn start(enabled: bool) -> Self {
        let st = AudioState {
            bands: Arc::new(
                (0..NBANDS)
                    .map(|_| AtomicU32::new(0f32.to_bits()))
                    .collect(),
            ),
            volume: Arc::new(AtomicU32::new(0f32.to_bits())),
            beat: Arc::new(AtomicU32::new(0)),
        };
        if enabled {
            let s = st.clone();
            thread::spawn(move || audio_capture_loop(s));
        }
        st
    }
    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
    fn beat(&self) -> bool {
        self.beat.load(Ordering::Relaxed) != 0
    }
    // Band energy for screen column `c` out of `cols` (rain-equalizer mapping).
    fn band_at(&self, c: usize, cols: usize) -> f32 {
        let b = c * NBANDS / cols.max(1);
        f32::from_bits(self.bands[b.min(NBANDS - 1)].load(Ordering::Relaxed))
    }
}

// Goertzel: energy of `freq` in the sample window (no full FFT needed).
fn goertzel(samples: &[f32], rate: f32, freq: f32) -> f32 {
    let n = samples.len();
    let mut k = (0.5 + (n as f32 * freq) / rate) as i32;
    if k <= 0 || k >= n as i32 {
        k = k.clamp(1, n as i32 - 1);
    }
    let w = 2.0 * std::f32::consts::PI * k as f32 / n as f32;
    let coeff = 2.0 * w.cos();
    let (mut s_prev, mut s_prev2) = (0f32, 0f32);
    for &x in samples {
        let s = x + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    s_prev2 * s_prev2 + s_prev * s_prev - coeff * s_prev * s_prev2
}

// Capture the default sink monitor via parec and keep per-band energies.
// Log-spaced bands 60Hz..10kHz, rolling-peak auto-normalization (quiet audio
// still moves), fast-attack/slow-decay per band so it follows without jitter.
fn audio_capture_loop(st: AudioState) {
    let band_freqs: Vec<f32> = (0..NBANDS)
        .map(|i| 60.0 * (10000.0f32 / 60.0).powf(i as f32 / (NBANDS - 1) as f32))
        .collect();
    loop {
        let sink = std::process::Command::new("pactl")
            .args(["get-default-sink"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        let sink = match sink {
            Some(s) if !s.is_empty() => s,
            _ => {
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        let monitor = format!("{sink}.monitor");
        let child = std::process::Command::new("parec")
            .args([
                "--device",
                &monitor,
                "--format=s16le",
                "--rate",
                &format!("{}", SAMPLE_RATE as u32),
                "--channels=1",
                "--latency-msec=40",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(_) => {
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        let mut out = child.stdout.take().unwrap();
        let mut buf = vec![0u8; CHUNK * 2];
        let mut samples = vec![0f32; CHUNK];
        let mut peak = 1e-6f32;
        let mut prev_rms = 0f32;
        let mut beat_hold = 0u32;
        loop {
            match out.read_exact(&mut buf) {
                Ok(()) => {
                    for i in 0..CHUNK {
                        samples[i] =
                            i16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as f32 / 32768.0;
                    }
                    let rms = (samples.iter().map(|v| v * v).sum::<f32>() / CHUNK as f32).sqrt();
                    // per-band energies
                    let mut frame_max = 0f32;
                    let mut mags = [0f32; NBANDS];
                    for (bi, &f) in band_freqs.iter().enumerate() {
                        let m = goertzel(&samples, SAMPLE_RATE, f);
                        mags[bi] = m;
                        if m > frame_max {
                            frame_max = m;
                        }
                    }
                    // rolling-peak adaptive normalization
                    peak = (peak * 0.995).max(frame_max).max(1e-6);
                    for (bi, &m) in mags.iter().enumerate() {
                        let norm = (m / peak).sqrt().min(1.0);
                        st.bands[bi].store(norm.to_bits(), Ordering::Relaxed);
                    }
                    // volume + beat
                    st.volume
                        .store((rms * 4.0).min(1.0).to_bits(), Ordering::Relaxed);
                    let beat = if rms > prev_rms * 1.35 && rms > 0.02 {
                        beat_hold = 3;
                        1
                    } else if beat_hold > 0 {
                        beat_hold -= 1;
                        1
                    } else {
                        0
                    };
                    st.beat.store(beat, Ordering::Relaxed);
                    prev_rms = rms;
                }
                Err(_) => break,
            }
        }
        let _ = child.kill();
        thread::sleep(Duration::from_secs(1));
    }
}

// Frame writer: alt-screen + hidden cursor, home each frame, and crucially NO
// trailing newline on the last row (a trailing newline scrolls the terminal
// and was the source of the visible flicker).
struct Screen {
    cols: usize,
    rows: usize,
    out: String,
    grid: Vec<Vec<char>>,
    color: Vec<Vec<String>>, // per-cell ANSI color string (empty = default)
    dirty: Vec<Vec<bool>>,   // cells painted this frame (need clearing next frame)
    show_fps: bool,
    fps_frames: u32,
    fps_since: std::time::Instant,
    fps_value: f32,
    paint_handshake: Option<PreparedFrameHandshake>,
}

impl Screen {
    fn new(cols: usize, rows: usize, show_fps: bool) -> Self {
        print!("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        std::io::stdout().flush().ok();
        Screen {
            cols,
            rows,
            out: String::with_capacity(cols * rows * 8),
            grid: vec![vec![' '; cols]; rows],
            color: vec![vec![String::new(); cols]; rows],
            dirty: vec![vec![false; cols]; rows],
            show_fps,
            fps_frames: 0,
            fps_since: std::time::Instant::now(),
            fps_value: 0.0,
            paint_handshake: None,
        }
    }
    fn set_paint_handshake(&mut self, path: Option<PathBuf>) {
        self.paint_handshake = path.map(PreparedFrameHandshake::new);
    }
    // Call once per frame; draw the FPS readout at bottom-left, clear of the
    // persistent Omarchy bar that covers the top terminal rows.
    // the user enabled it in preferences. `intended_ms` is the frame's intended
    // sleep — feeding it to the auto-degrade watcher lets it detect sustained
    // pacing drops (actual >> intended) and coarsen the grid.
    fn fps_overlay(&mut self, intended_ms: f64) {
        AUTO_DEGRADE.with(|a| a.borrow_mut().tick(intended_ms));
        self.fps_frames += 1;
        let el = self.fps_since.elapsed().as_secs_f32();
        if el >= 0.5 {
            self.fps_value = self.fps_frames as f32 / el;
            self.fps_frames = 0;
            self.fps_since = std::time::Instant::now();
        }
        if !self.show_fps {
            return;
        }
        let text = format!("FPS {:.0}", self.fps_value);
        let row = self.rows.saturating_sub(1);
        for (i, ch) in text.chars().enumerate() {
            if i < self.cols {
                self.put(i, row, ch, "\x1b[37m".to_string());
            }
        }
    }
    fn clear(&mut self) {
        for r in self.grid.iter_mut() {
            r.fill(' ');
        }
        for r in self.color.iter_mut() {
            r.iter_mut().for_each(|c| c.clear());
        }
        for r in self.dirty.iter_mut() {
            r.fill(false);
        }
    }
    // Clear only cells that were painted the previous frame (dirty tracking).
    // Cells NOT painted keep their color — this enables crossfade between themes.
    fn clear_dirty(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                if self.dirty[r][c] {
                    self.grid[r][c] = ' ';
                    self.color[r][c].clear();
                    self.dirty[r][c] = false;
                }
            }
        }
    }
    fn put(&mut self, x: usize, y: usize, ch: char, color: String) {
        if x < self.cols && y < self.rows {
            self.grid[y][x] = ch;
            self.color[y][x] = color;
            self.dirty[y][x] = true;
        }
    }
    // Erase a single cell (restore to blank, no color) without dirty tracking.
    fn erase(&mut self, x: usize, y: usize) {
        if x < self.cols && y < self.rows {
            self.grid[y][x] = ' ';
            self.color[y][x].clear();
            self.dirty[y][x] = false;
        }
    }
    // Return the ANSI color currently stored at a cell (empty = default).
    fn get_color(&self, x: usize, y: usize) -> String {
        if x < self.cols && y < self.rows {
            self.color[y][x].clone()
        } else {
            String::new()
        }
    }
    fn present(&mut self) -> Result<()> {
        if let Some(handshake) = self.paint_handshake.as_mut() {
            handshake.before_first_frame()?;
        }
        self.out.clear();
        self.out.push_str("\x1b[H");
        let mut cur: &str = "";
        for r in 0..self.rows {
            for c in 0..self.cols {
                let col = &self.color[r][c];
                if col.is_empty() {
                    if cur != "" {
                        self.out.push_str("\x1b[0m");
                        cur = "";
                    }
                } else if col != cur {
                    self.out.push_str(col);
                    cur = col;
                }
                self.out.push(self.grid[r][c]);
            }
            if r + 1 < self.rows {
                self.out.push('\n');
            }
        }
        if cur != "" {
            self.out.push_str("\x1b[0m");
        }
        print!("{}", self.out);
        std::io::stdout().flush()?;
        Ok(())
    }
    // Re-read the PTY size; if it changed (window resized / Vte expanded it
    // after spawn), reallocate the grid and report the change so the caller
    // restarts the effect cleanly at the new size.
    fn maybe_resize(&mut self) -> bool {
        if let Some((c, r)) = pty_size() {
            if c != self.cols || r != self.rows {
                self.cols = c;
                self.rows = r;
                self.grid = vec![vec![' '; c]; r];
                self.color = vec![vec![String::new(); c]; r];
                self.dirty = vec![vec![false; c]; r];
                self.out = String::with_capacity(c * r * 8);
                print!("\x1b[2J\x1b[H");
                return true;
            }
        }
        false
    }
}

// Actual size of the PTY we render into. The parent computes cols/rows from
// font metrics, but Vte may round differently — rendering more rows than the
// PTY has caused a visible one-line scroll ("jump") every frame. The child
// queries the real size and clamps to it.
fn pty_size() -> Option<(usize, usize)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            Some((ws.ws_col as usize, ws.ws_row as usize))
        } else {
            None
        }
    }
}

// Wait for Vte to size the PTY to the real widget dimensions (it spawns at 80x24
// and resizes a beat later), returning the settled (cols, rows). Reading too early
// lays a one-shot layout (the intro, a ttfx canvas) out on an 80x24 grid.
fn settle_pty_size(fallback_cols: usize, fallback_rows: usize) -> (usize, usize) {
    let fallback = (fallback_cols, fallback_rows);
    let mut prev = pty_size().unwrap_or(fallback);
    let mut settled = if prev != (80, 24) { prev } else { fallback };
    for _ in 0..150 {
        // up to ~1.5s
        let cur = pty_size().unwrap_or(fallback);
        if cur != (80, 24) && cur == prev {
            settled = cur;
            break;
        }
        prev = cur;
        thread::sleep(Duration::from_millis(10));
    }
    settled
}

// 5-row ASCII bitmap font (FIGlet-style, pure ASCII '#'/space). Each glyph is 5
// rows tall; `intro_size` scales each font pixel into an N×N cell block, so the
// title renders as real ASCII-art letters — not a per-char solid block.
fn glyph_rows(c: char) -> [&'static str; 5] {
    match c {
        'A' => [" ### ", "#   #", "#   #", "#####", "#   #"],
        'B' => ["#### ", "#   #", "#### ", "#   #", "#### "],
        'C' => [" ####", "#    ", "#    ", "#    ", " ####"],
        'D' => ["#### ", "#   #", "#   #", "#   #", "#### "],
        'E' => ["#####", "#    ", "#### ", "#    ", "#####"],
        'F' => ["#####", "#    ", "#### ", "#    ", "#    "],
        'G' => [" ####", "#    ", "#  ##", "#   #", " ####"],
        'H' => ["#   #", "#   #", "#####", "#   #", "#   #"],
        'I' => ["#####", "  #  ", "  #  ", "  #  ", "#####"],
        'J' => ["#####", "   # ", "   # ", "#  # ", " ##  "],
        'K' => ["#   #", "#  # ", "###  ", "#  # ", "#   #"],
        'L' => ["#    ", "#    ", "#    ", "#    ", "#####"],
        'M' => ["#   #", "## ##", "# # #", "#   #", "#   #"],
        'N' => ["#   #", "##  #", "# # #", "#  ##", "#   #"],
        'O' => [" ### ", "#   #", "#   #", "#   #", " ### "],
        'P' => ["#### ", "#   #", "#### ", "#    ", "#    "],
        'Q' => [" ### ", "#   #", "# # #", "#  # ", " ## #"],
        'R' => ["#### ", "#   #", "#### ", "#  # ", "#   #"],
        'S' => [" ####", "#    ", " ### ", "    #", "#### "],
        'T' => ["#####", "  #  ", "  #  ", "  #  ", "  #  "],
        'U' => ["#   #", "#   #", "#   #", "#   #", " ### "],
        'V' => ["#   #", "#   #", "#   #", " # # ", "  #  "],
        'W' => ["#   #", "#   #", "# # #", "## ##", "#   #"],
        'X' => ["#   #", " # # ", "  #  ", " # # ", "#   #"],
        'Y' => ["#   #", " # # ", "  #  ", "  #  ", "  #  "],
        'Z' => ["#####", "   # ", "  #  ", " #   ", "#####"],
        '0' => [" ### ", "#  ##", "# # #", "##  #", " ### "],
        '1' => ["  #  ", " ##  ", "  #  ", "  #  ", "#####"],
        '2' => [" ### ", "#   #", "  ## ", " #   ", "#####"],
        '3' => ["#### ", "    #", " ### ", "    #", "#### "],
        '4' => ["#  # ", "#  # ", "#####", "   # ", "   # "],
        '5' => ["#####", "#    ", "#### ", "    #", "#### "],
        '6' => [" ### ", "#    ", "#### ", "#   #", " ### "],
        '7' => ["#####", "   # ", "  #  ", " #   ", "#    "],
        '8' => [" ### ", "#   #", " ### ", "#   #", " ### "],
        '9' => [" ### ", "#   #", " ####", "    #", " ### "],
        '.' => ["     ", "     ", "     ", "     ", "  #  "],
        ',' => ["     ", "     ", "     ", "  #  ", " #   "],
        '/' => ["    #", "   # ", "  #  ", " #   ", "#    "],
        '@' => [" ### ", "# ###", "# # #", "# ## ", " ### "],
        '-' => ["     ", "     ", " ### ", "     ", "     "],
        ':' => ["     ", "  #  ", "     ", "  #  ", "     "],
        '!' => ["  #  ", "  #  ", "  #  ", "     ", "  #  "],
        '?' => [" ### ", "#   #", "  ## ", "     ", "  #  "],
        '+' => ["     ", "  #  ", " ### ", "  #  ", "     "],
        '_' => ["     ", "     ", "     ", "     ", "#####"],
        '(' => ["   # ", "  #  ", "  #  ", "  #  ", "   # "],
        ')' => ["#    ", " #   ", " #   ", " #   ", "#    "],
        '\'' => ["  #  ", "  #  ", "     ", "     ", "     "],
        _ => ["     ", "     ", "     ", "     ", "     "],
    }
}

// Uppercase + normalize dashes so any byline / effect name maps onto the font.
fn prep(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '—' | '–' => '-',
            other => other.to_ascii_uppercase(),
        })
        .collect()
}

// Render the first `upto` chars of `text` as scaled ASCII-art rows (each font
// pixel becomes an N×N cell block). All glyphs are 5 rows × 5 cols.
fn art_prefix(text: &str, upto: usize, scale: usize, gap: usize) -> Vec<String> {
    let mut rows = vec![String::new(); 5 * scale];
    for ch in text.chars().take(upto) {
        let g = glyph_rows(ch);
        for r in 0..5 {
            let mut scaled = String::new();
            for c in g[r].chars() {
                for _ in 0..scale {
                    scaled.push(c);
                }
            }
            for sy in 0..scale {
                rows[r * scale + sy].push_str(&scaled);
                for _ in 0..gap {
                    rows[r * scale + sy].push(' ');
                }
            }
        }
    }
    rows
}

// Nominal rendered width of `text` as ASCII art (glyphs are 5 cols + `gap`).
fn art_width(text: &str, scale: usize, gap: usize) -> usize {
    let n = text.chars().count();
    if n == 0 {
        0
    } else {
        n * (5 * scale + gap) - gap
    }
}

// Intro: the title, byline and effect tag ALL render as ASCII art (scaled
// ×1..×3), centered on BOTH axes as a single stack. The title types in letter by
// letter, then the byline, then the effect tag, then it holds.
// Audio-reactive: each letter step pulses to the beat (volume/beat => faster typing).
fn show_intro(
    scr: &mut Screen,
    palette: &[String],
    byline: &str,
    effect: &str,
    intro_size: i64,
    audio: &AudioState,
    intro_beat_sync: bool,
    boot_char_style: &str,
) -> Result<()> {
    let scale = intro_size.clamp(1, 3) as usize;
    let gap = scale.max(1); // spaces between ASCII-art letters
    let title = "OMARCHY AUDIO BACKGROUND".to_string();
    let by = prep(if byline.trim().is_empty() {
        DEFAULT_BYLINE
    } else {
        byline.trim()
    });
    let tag = format!("- {} -", prep(effect));

    let bh = 5 * scale; // every block is 5 glyph rows × scale
    let vgap = 1usize; // blank row between blocks

    let title_w = art_width(&title, scale, gap);
    let by_w = art_width(&by, scale, gap);
    let tag_w = art_width(&tag, scale, gap);

    // Center the whole 3-block stack vertically, each block centered horizontally.
    let total_h = 3 * bh + 2 * vgap;
    let ty = scr.rows.saturating_sub(total_h) / 2;
    let by_y = ty + bh + vgap;
    let tag_y = by_y + bh + vgap;
    let tx_title = scr.cols.saturating_sub(title_w) / 2;
    let tx_by = scr.cols.saturating_sub(by_w) / 2;
    let tx_tag = scr.cols.saturating_sub(tag_w) / 2;

    // Draw the first `upto` letters of a block at its (centered) left edge, so the
    // typewriter fills left-to-right and ends centered.
    let draw = |scr: &mut Screen, text: &str, upto: usize, x: usize, y: usize, color: u8| {
        for (r, line) in art_prefix(text, upto, scale, gap).iter().enumerate() {
            for (i, ch) in line.chars().enumerate() {
                if ch != ' ' {
                    scr.put(
                        x + i,
                        y + r,
                        styled_glyph(boot_char_style, ch),
                        tint_to_color(color, palette),
                    );
                }
            }
        }
    };

    let title_len = title.chars().count();
    let by_len = by.chars().count();
    let tag_len = tag.chars().count();

    // Helper: wait for the next audio beat (with timeout fallback if no music).
    let wait_for_beat = |audio: &AudioState, timeout_ms: u64| -> bool {
        let start = std::time::Instant::now();
        loop {
            if audio.beat() {
                return true;
            }
            if start.elapsed() >= Duration::from_millis(timeout_ms) {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
    };

    if intro_beat_sync && audio.volume() > 0.02 {
        // Efectos internos de ttfx que NO usan colores de tema; usar palett...[truncated]
        for step in 0..=title_len {
            scr.clear_dirty();
            draw(scr, &title, step, tx_title, ty, 1);
            scr.fps_overlay(70.0);
            scr.present()?;
            let beat_sync = wait_for_beat(audio, 1500);
            // If we got a beat, great; otherwise fall back to volume-paced delay.
            if !beat_sync {
                let vol = audio.volume().clamp(0.0, 1.0);
                let speed = 1.0 + vol * 2.5;
                thread::sleep(Duration::from_millis((70.0 / speed).max(12.0) as u64));
            }
        }
        // Phase 2: type the byline under the complete title.
        for step in 0..=by_len {
            scr.clear_dirty();
            draw(scr, &title, title_len, tx_title, ty, 1);
            draw(scr, &by, step, tx_by, by_y, 2);
            scr.fps_overlay(70.0);
            scr.present()?;
            let beat_sync = wait_for_beat(audio, 1500);
            if !beat_sync {
                let vol = audio.volume().clamp(0.0, 1.0);
                let speed = 1.0 + vol * 2.5;
                thread::sleep(Duration::from_millis((45.0 / speed).max(10.0) as u64));
            }
        }
    } else {
        // Phase 1: type the title, one ASCII-art letter at a time.
        for step in 0..=title_len {
            scr.clear_dirty();
            draw(scr, &title, step, tx_title, ty, 1);
            scr.fps_overlay(70.0);
            scr.present()?;
            let vol = audio.volume().clamp(0.0, 1.0);
            let speed = 1.0 + vol * 2.5 + if audio.beat() { 1.0 } else { 0.0 };
            thread::sleep(Duration::from_millis((70.0 / speed).max(12.0) as u64));
        }
        // Phase 2: type the byline under the complete title.
        for step in 0..=by_len {
            scr.clear_dirty();
            draw(scr, &title, title_len, tx_title, ty, 1);
            draw(scr, &by, step, tx_by, by_y, 2);
            scr.fps_overlay(45.0);
            scr.present()?;
            let vol = audio.volume().clamp(0.0, 1.0);
            let speed = 1.0 + vol * 2.5 + if audio.beat() { 1.0 } else { 0.0 };
            thread::sleep(Duration::from_millis((45.0 / speed).max(10.0) as u64));
        }
    }

    // Phase 3: stamp the effect tag, then hold the finished splash.
    scr.clear_dirty();
    draw(scr, &title, title_len, tx_title, ty, 1);
    draw(scr, &by, by_len, tx_by, by_y, 2);
    draw(scr, &tag, tag_len, tx_tag, tag_y, 3);
    scr.fps_overlay(2600.0);
    scr.present()?;
    // Hold pulses with audio: beat shortens hold, quiet holds full 2600ms.
    let hold_base = 2600.0;
    // If audio active, let the hold be slightly shorter on loud sections so boot feels synced.
    let vol = audio.volume().clamp(0.0, 1.0);
    let hold_speed = 1.0 + vol * 0.8 + if audio.beat() { 0.5 } else { 0.0 };
    thread::sleep(Duration::from_millis(
        (hold_base / hold_speed).max(900.0) as u64
    ));
    Ok(())
}

// ttfx effects we route to the vendored engine (everything in the catalog EXCEPT
// matrix/rain, which we keep hand-rolled because ours are audio-reactive).
const TTFX_EFFECTS: [&str; 35] = [
    "beams",
    "binarypath",
    "blackhole",
    "bouncyballs",
    "bubbles",
    "burn",
    "colorshift",
    "crumble",
    "decrypt",
    "errorcorrect",
    "expand",
    "fireworks",
    "highlight",
    "laseretch",
    "middleout",
    "orbittingvolley",
    "overflow",
    "pour",
    "print",
    "randomsequence",
    "rings",
    "scattered",
    "slice",
    "slide",
    "smoke",
    "spotlights",
    "spray",
    "swarm",
    "sweep",
    "synthgrid",
    "thunderstorm",
    "unstable",
    "vhstape",
    "waves",
    "wipe",
];

fn is_ttfx_effect(name: &str) -> bool {
    TTFX_EFFECTS.contains(&name)
}

fn build_ttfx_effect(name: &str) -> Option<Box<dyn ttfx::engine::effect::Effect>> {
    use clap::Parser;
    match ttfx::cli::Cli::try_parse_from(["ttfx", name]) {
        Ok(ttfx::cli::Cli {
            effect: Some(effect),
            ..
        }) => Some(effect.build_effect()),
        _ => None,
    }
}

fn use_ttfx_backend(name: &str, seed: Option<u64>) -> bool {
    is_ttfx_effect(name) || (seed.is_some() && matches!(name, "matrix" | "rain"))
}

// Any effect the renderer can actually run: our hand-rolled set or the ttfx catalog.
fn is_valid_effect(name: &str) -> bool {
    DEFAULT_EFFECTS.contains(&name) || is_ttfx_effect(name)
}

// Drive a vendored ttfx effect on our Vte PTY, looping so it runs as a continuous
// background (ttfx effects settle when done; rebuild and replay). Handles PTY resize
// by rebuilding at the new size. Runs after our ASCII intro (same stdout).
// The official wordmark is a bitmap; every other input remains ordinary text.
#[cfg(test)]
fn ttfx_canvas_input(text: &str, cols: usize, rows: usize, char_style: &str) -> String {
    wordmark::canvas_input(text, cols, rows, char_style, 2.0)
}

type ThemeRgb = (u8, u8, u8);

fn gradient_rgb_set(stops: &[&str], steps: i64) -> std::collections::HashSet<ThemeRgb> {
    use ttfx::utils::graphics::{Color, Gradient};
    let colors = stops
        .iter()
        .map(|hex| Color::from_hex(hex).expect("valid effect color"))
        .collect::<Vec<_>>();
    Gradient::with_steps(&colors, steps, false)
        .expect("valid effect gradient")
        .spectrum
        .iter()
        .filter_map(|color| parse_hex_color(&color.rgb_color.to_string()))
        .collect()
}

fn theme_palette_mapper(theme: &[(String, String)]) -> impl Fn(u8, u8, u8) -> ThemeRgb + use<> {
    let get = |key: &str, fallback: ThemeRgb| {
        theme
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| parse_hex_color(value))
            .unwrap_or(fallback)
    };
    let accent = get("accent", (128, 160, 255));
    let foreground = get("foreground", accent);
    let wheel = [
        get("red", accent),
        get("yellow", accent),
        get("green", accent),
        get("cyan", accent),
        get("blue", accent),
        get("magenta", accent),
        get("red", accent),
    ];
    move |r, g, b| {
        if (r, g, b) == (0, 0, 0) {
            return (0, 0, 0);
        }
        let rf = r as f32 / 255.0;
        let gf = g as f32 / 255.0;
        let bf = b as f32 / 255.0;
        let max = rf.max(gf).max(bf);
        let min = rf.min(gf).min(bf);
        let delta = max - min;
        let target = if delta / max.max(0.001) < 0.12 {
            foreground
        } else {
            let hue = if max == rf {
                ((gf - bf) / delta).rem_euclid(6.0)
            } else if max == gf {
                (bf - rf) / delta + 2.0
            } else {
                (rf - gf) / delta + 4.0
            };
            let index = hue.floor() as usize;
            let fraction = hue - index as f32;
            let a = wheel[index];
            let z = wheel[index + 1];
            (
                ((a.0 as f32 + (z.0 as f32 - a.0 as f32) * fraction).round()) as u8,
                ((a.1 as f32 + (z.1 as f32 - a.1 as f32) * fraction).round()) as u8,
                ((a.2 as f32 + (z.2 as f32 - a.2 as f32) * fraction).round()) as u8,
            )
        };
        (
            (target.0 as f32 * max).round() as u8,
            (target.1 as f32 * max).round() as u8,
            (target.2 as f32 * max).round() as u8,
        )
    }
}

fn final_accent_set(effect: &str) -> std::collections::HashSet<ThemeRgb> {
    let (stops, steps): (&[&str], i64) = match effect {
        "binarypath" => (&["00d500", "007500"], 12),
        "blackhole" => (&["8A008A", "00D1FF", "ffffff"], 9),
        "bubbles" => (&["d33aff", "02ff7f"], 12),
        "crumble" => (&["5CE1FF", "FF8C00"], 12),
        "decrypt" => (&["eda000"], 12),
        "errorcorrect" | "fireworks" | "smoke" | "thunderstorm" | "unstable" => {
            (&["8A008A", "00D1FF", "FFFFFF"], 12)
        }
        "laseretch" | "sweep" => (&["8A008A", "00D1FF", "ffffff"], 8),
        "swarm" => (&["31b900", "f0ff65"], 12),
        "synthgrid" => (&["8a008a", "00d1ff", "ffffff"], 12),
        "vhstape" => (&["ab48ff", "e7b2b2", "fffebd"], 12),
        _ => return std::collections::HashSet::new(),
    };
    gradient_rgb_set(stops, steps)
}

fn ttfx_theme_transform(
    effect_name: &str,
    theme: &[(String, String)],
) -> Option<ttfx::utils::ansi::ColorTransform> {
    // A missing theme file is not a theme. Leaving the transform disabled keeps
    // TTFX's built-in palette intact instead of applying fallback accent colors.
    if theme.is_empty() {
        return None;
    }
    let map = theme_palette_mapper(theme);
    match effect_name {
        // These were deliberately left unthemed when the initial catalog landed,
        // which made Colorshift and Waves ignore the user's theme entirely. They
        // are abstract effects, so their colors are decorative rather than semantic:
        // map their full spectrum like the rest of the abstract catalog.
        "colorshift" | "waves" => Some(std::rc::Rc::new(map)),
        // Physical/semantic effects: map only the unambiguous final-text spectrum.
        "binarypath" | "blackhole" | "bubbles" | "crumble" | "decrypt" | "errorcorrect"
        | "fireworks" | "laseretch" | "smoke" | "swarm" | "sweep" | "synthgrid"
        | "thunderstorm" | "unstable" | "vhstape" => {
            let accents = final_accent_set(effect_name);
            Some(std::rc::Rc::new(move |r, g, b| {
                if accents.contains(&(r, g, b)) {
                    map(r, g, b)
                } else {
                    (r, g, b)
                }
            }))
        }
        "burn" => {
            // Product decision: preserve the complete fire/ember spectrum. Starting,
            // final and smoke accents may follow the host palette.
            let fire = gradient_rgb_set(&["ffffff", "fff75d", "fe650d", "8A003C", "510100"], 10);
            Some(std::rc::Rc::new(move |r, g, b| {
                if fire.contains(&(r, g, b)) {
                    (r, g, b)
                } else {
                    map(r, g, b)
                }
            }))
        }
        // Abstract/motion effects have no physically meaningful hue: all authored
        // colors are decorative accents, mapped across the full theme color wheel.
        _ if is_ttfx_effect(effect_name) => Some(std::rc::Rc::new(map)),
        _ => None,
    }
}

// Drive a vendored ttfx effect on our Vte PTY, audio-reactively. The effect runs on
// a VIRTUAL clock and we pace the frames by the live audio level — loud music
// advances the effect faster, quiet slows it — so the whole ttfx catalog reacts to
// the music like the hand-rolled effects do. Loops so it runs as a continuous
// background; rebuilds on PTY resize. Runs after our ASCII intro (same stdout).
fn ttfx_rng(seed: Option<u64>) -> ttfx::utils::rng::Rng {
    match seed {
        Some(seed) => ttfx::utils::rng::Rng::seeded(seed),
        None => ttfx::utils::rng::Rng::from_entropy(),
    }
}

fn run_ttfx(
    effect_name: &str,
    cols: usize,
    rows: usize,
    ttfx_text: &str,
    audio: &AudioState,
    audio_enabled: bool,
    intensity: i64,
    speed: i64,
    reactivity: i64,
    use_theme_colors: bool,
    char_style: &str,
    show_fps: bool,
    cell_aspect: f32,
    seed: Option<u64>,
    paint_socket: Option<PathBuf>,
) -> Result<()> {
    log_dbg(&ttfx_runtime_diagnostic(
        effect_name,
        cols,
        rows,
        reactivity,
        audio_enabled,
    ));
    use std::io::Write;
    use ttfx::engine::ctx::{Clock, EngineCtx};
    use ttfx::engine::terminal::TerminalConfig;

    let (mut cols, mut rows) = settle_pty_size(cols, rows);
    let fps = 60i64;
    let frame_secs = 1.0 / fps as f64;
    // reactivity is live-adjustable: the frame loop re-reads state.json so a slider
    // change applies WITHOUT restarting the background (no respawn, no intro replay).
    let mut reactivity = reactivity;
    let mut frames = 0u32;
    let mut fps_frames = 0u32;
    let mut fps_since = std::time::Instant::now();
    let mut fps_value = 0.0f32;
    let mut theme_watcher = ThemeWatcher::new();
    let mut paint_handshake = paint_socket.map(PreparedFrameHandshake::new);
    loop {
        // Build the effect with default config via the clap parser (like upstream's
        // --random-effect), then drive it ourselves so we can pace it by audio.
        let mut effect = match build_ttfx_effect(effect_name) {
            Some(effect) => effect,
            None => {
                let m = format!("unknown ttfx effect: {effect_name}");
                eprintln!("{m}");
                log_dbg(&m);
                return Ok(());
            }
        };
        // Match the official pixel proportions to the actual terminal cells.
        let input = wordmark::canvas_input(ttfx_text, cols, rows, char_style, cell_aspect);
        let mut config = TerminalConfig::default();
        config.canvas_width = cols as i64;
        config.canvas_height = rows as i64;
        config.frame_rate = fps;
        let mut ctx = match EngineCtx::new(
            &input,
            config,
            ttfx_rng(seed),
            Clock::virtual_with_frame_rate(fps),
        ) {
            Ok(c) => c,
            Err(e) => {
                let m = format!("ttfx ctx error for {effect_name}: {e:?}");
                eprintln!("{m}");
                log_dbg(&m);
                return Ok(());
            }
        };
        ctx.final_text_bands = true;
        if let Err(e) = effect.build(&mut ctx) {
            let m = format!("ttfx build error for {effect_name}: {e:?}");
            eprintln!("{m}");
            log_dbg(&m);
            return Ok(());
        }

        // Audio-paced frame loop: with a virtual clock each next_frame() advances the
        // animation by one tick, so pacing the calls by the live audio level makes the
        // effect speed up on loud passages / beats and slow down when quiet.
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        if ctx.terminal.prep_canvas(&mut out).is_err() {
            return Ok(());
        }
        loop {
            // Audio-reactive color: bass -> hue shift, volume -> brightness. Same
            // reactivity slider that drives speed: 0=off, 3=triple. Applied in the
            // global sgr_color hook so every ttfx effect recolors without per-effect
            // patches. This is the bridge (vendored engine) - no upstream PR needed.
            {
                let vol = audio.volume().clamp(0.0, 1.0);
                let bass =
                    (audio.band_at(0, NBANDS) + audio.band_at(1, NBANDS) * 0.5).clamp(0.0, 1.0);
                let active = audio_enabled && reactivity > 0 && (vol > 0.02 || bass > 0.05);
                if active {
                    // Brightness 1.0..1.5 (reactivity 3 = stronger), hue up to ~60 deg from bass + vol.
                    let bright = 1.0 + vol * 0.45 * (reactivity as f32 / 2.0);
                    let hue_deg = if effect_name == "burn" {
                        0.0 // fire hue is semantic; audio may brighten it, never recolor it
                    } else {
                        (bass * 45.0 + vol * 18.0) * (reactivity as f32 / 2.0)
                            + if audio.beat() { 10.0 } else { 0.0 }
                    };
                    let rad = hue_deg.to_radians();
                    let (c, s) = (rad.cos(), rad.sin());
                    let t = 1.0 - c;
                    let w1 = 0.57735026; // 1/sqrt(3) for hue rotation around gray axis
                    let m = [
                        c + t / 3.0,
                        t / 3.0 - w1 * s,
                        t / 3.0 + w1 * s,
                        t / 3.0 + w1 * s,
                        c + t / 3.0,
                        t / 3.0 - w1 * s,
                        t / 3.0 - w1 * s,
                        t / 3.0 + w1 * s,
                        c + t / 3.0,
                    ];
                    ttfx::utils::ansi::set_audio_color(true, bright, m);
                } else {
                    ttfx::utils::ansi::set_audio_color(
                        false,
                        1.0,
                        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    );
                }
            }
            // Effect-aware live theme mapping: semantic colors stay intact;
            // only accents selected for this specific effect follow the theme.
            if use_theme_colors {
                if let Some(new_colors) = theme_watcher.changed() {
                    let accent = new_colors
                        .iter()
                        .find(|(key, _)| key == "accent")
                        .map(|(_, value)| value.as_str())
                        .unwrap_or("unknown");
                    ttfx::utils::ansi::set_color_transform(ttfx_theme_transform(
                        effect_name,
                        new_colors,
                    ));
                    log_dbg(&format!(
                        "ttfx theme colors changed: {effect_name} accent={accent}"
                    ));
                }
            } else {
                ttfx::utils::ansi::set_color_transform(None);
            }
            // Audio-reactive per-effect hook (e.g. thunderstorm lightning on loud beats).
            // Runs before next_frame so the effect can inject a strike this tick.
            if audio_enabled && reactivity > 0 {
                let vol = audio.volume().clamp(0.0, 1.0);
                let bass =
                    (audio.band_at(0, NBANDS) + audio.band_at(1, NBANDS) * 0.5).clamp(0.0, 1.0);
                let beat = audio.beat();
                effect.on_audio(&mut ctx, vol, bass, beat);
            }
            // Stop if the PTY went away (effect settled / terminal gone).
            let frame =
                match gate_prepared_frame(effect.next_frame(&mut ctx), &mut paint_handshake)? {
                    Some(frame) => frame,
                    None => break,
                };
            if ctx.terminal.print_frame(&mut out, &frame).is_err() {
                let _ = ctx.terminal.restore_cursor(&mut out, "");
                return Ok(());
            }
            // ttfx renders directly through its own terminal, bypassing Screen. Keep
            // its overlay here so the FPS switch works for every catalog effect too.
            fps_frames += 1;
            let fps_elapsed = fps_since.elapsed().as_secs_f32();
            if fps_elapsed >= 0.5 {
                fps_value = fps_frames as f32 / fps_elapsed;
                fps_frames = 0;
                fps_since = std::time::Instant::now();
            }
            if show_fps {
                if write!(out, "\x1b[{};1H\x1b[97mFPS {:.0}\x1b[0m", rows, fps_value).is_err() {
                    let _ = ctx.terminal.restore_cursor(&mut out, "");
                    return Ok(());
                }
            }
            if out.flush().is_err() {
                let _ = ctx.terminal.restore_cursor(&mut out, "");
                return Ok(());
            }
            // Live config poll (~every 45 frames): apply reactivity changes without a
            // restart. Do not compare `live.effect` here: it is the user's selected
            // effect, while the controller can temporarily render another enabled
            // effect during rotation. That comparison killed every rotated ttfx child
            // after 45 frames, leaving a blank/restarting background.
            frames += 1;
            if seed.is_none() && frames % 45 == 0 {
                let live = read_config();
                reactivity = live.reactivity;
                if !live.running {
                    let _ = ctx.terminal.restore_cursor(&mut out, "");
                    return Ok(());
                }
            }
            // Pace by the live audio level, scaled by the user's reactivity setting.
            // Louder => smaller delay => faster animation, capped at 3x (triple).
            // reactivity 0 disables it; higher reactivity reaches the cap more easily.
            let vol = audio.volume().clamp(0.0, 1.0);
            let boost = vol * reactivity as f32 + if audio.beat() { 0.5 } else { 0.0 };
            let audio_speed = if audio_enabled && reactivity > 0 {
                (1.0 + boost).clamp(1.0, 3.0)
            } else {
                1.0
            };
            // Apply intensity as a live baseline speed control. It is sampled
            // independently of reactivity, so moving this slider never resets
            // a running ttfx scene.
            let intensity_speed = 0.70 + live_intensity(intensity) as f32 * 0.06;
            let baseline =
                Duration::from_secs_f64(frame_secs / (audio_speed * intensity_speed) as f64);
            let intended = speed_delay(baseline, live_speed(speed));
            thread::sleep(intended);
            AUTO_DEGRADE.with(|a| a.borrow_mut().tick(intended.as_secs_f64() * 1000.0));
        }
        let _ = ctx.terminal.restore_cursor(&mut out, "");
        // Continuous background: loop all ttfx effects for the full rotate_secs
        // without a gap. The 400ms pause caused the visible "para y vuelve".
        // A selected effect is different from the controller's current rotation
        // item, so only the running flag is safe to inspect here. The controller
        // terminates and replaces this child for a real effect change.
        if seed.is_none() {
            let live = read_config();
            if !live.running {
                let _ = ctx.terminal.restore_cursor(&mut out, "\n");
                return Ok(());
            }
        }
        // Tiny settle without blanking — next pass rebuilds immediately.
        let (c2, r2) = settle_pty_size(cols, rows);
        if (c2, r2) != (cols, rows) {
            cols = c2;
            rows = r2;
        }
    }
}

fn run_render(options: RenderOptions) -> Result<()> {
    let RenderOptions {
        effect,
        cols,
        rows,
        intensity,
        speed,
        audio,
        byline,
        ttfx_text,
        reactivity,
        intro_size,
        cell_aspect,
        show_fps,
        show_intro: with_intro,
        intro_beat_sync,
        use_theme_colors,
        transparent_background,
        char_style,
        boot_char_style,
        seed,
        paint_socket,
    } = options;
    log_dbg(&render_runtime_diagnostic(
        &effect, cols, rows, audio, with_intro,
    ));
    let intensity = intensity.clamp(0, 10);
    let state = AudioState::start(audio);
    // Use the REAL PTY size, but WAIT for it to settle first (Vte spawns at 80x24).
    let (cols, rows) = settle_pty_size(cols, rows);
    let mut scr = Screen::new(cols, rows, show_fps);

    let palette: Vec<String> = if use_theme_colors {
        // Use theme colors when user opts in; fall back gracefully if theme unavailable
        let tc = read_theme_colors();
        if !tc.is_empty() {
            theme_palette(&tc, &effect).into_iter().collect()
        } else {
            hardcoded_palette(&effect)
        }
    } else {
        hardcoded_palette(&effect)
    };
    let palette_refs: Vec<&str> = palette.iter().map(|s| s.as_str()).collect();

    // Initialize the thread-local palette so effects can read live theme colors
    if use_theme_colors {
        set_theme_palette(palette.clone());
    }

    set_auto_degrade_enabled(false);
    if with_intro && paint_socket.is_none() {
        show_intro(
            &mut scr,
            &palette,
            &byline,
            &effect,
            intro_size,
            &state,
            intro_beat_sync,
            &boot_char_style,
        )?;
    }

    // From here on, sustained FPS drops auto-escalate the grid resolution and
    // shrink the boot text (the intro replaying at the new size confirms it).
    set_auto_degrade_enabled(true);

    // ttfx effects drive the vendored engine on this same PTY (after our intro).
    if use_ttfx_backend(&effect, seed) {
        return run_ttfx(
            &effect,
            cols,
            rows,
            &ttfx_text,
            &state,
            audio,
            intensity,
            speed,
            reactivity,
            use_theme_colors,
            &char_style,
            show_fps,
            cell_aspect,
            seed,
            paint_socket,
        );
    }

    scr.set_paint_handshake(paint_socket);

    // Each effect returns Ok(()) when it detects a PTY resize; re-dispatch so
    // it restarts with fresh state at the new size (no jump, no stale grid).
    loop {
        let pal: &[String] = &palette;
        let res = match effect.as_str() {
            "donut" => fx_donut(
                &mut scr,
                pal,
                intensity,
                &state,
                cell_aspect,
                use_theme_colors,
                &effect,
            ),
            "fire" => fx_fire(&mut scr, pal, intensity, &state, use_theme_colors, &effect),
            "starfield" => {
                fx_starfield(&mut scr, pal, intensity, &state, use_theme_colors, &effect)
            }
            "life" => fx_life(&mut scr, pal, intensity, &state, use_theme_colors, &effect),
            "wave" => fx_wave(&mut scr, pal, intensity, &state, use_theme_colors, &effect),
            "bars" => fx_bars(&mut scr, pal, intensity, &state, use_theme_colors, &effect),
            _ => fx_matrix(
                &mut scr,
                pal,
                intensity,
                &state,
                effect == "rain",
                use_theme_colors,
                &effect,
            ),
        };
        if let Err(e) = res {
            eprintln!("render error: {e:?}");
            break;
        }
        // Ok(()) => resize happened; loop and restart the effect.
    }
    Ok(())
}

// Audio-reactive pacing: more sound => faster flow (lower delay), smoothly.
fn frame_delay(base_ms: i64, intensity: i64, speed: i64, audio: &AudioState) -> Duration {
    let base = (base_ms - intensity * 3).clamp(8, 120) as f32;
    let audio_speed = 1.0 + audio.volume() * 2.5;
    let baseline = Duration::from_millis((base / audio_speed).max(6.0) as u64);
    speed_delay(baseline, speed)
}

// --- matrix / rain: column rain where EACH COLUMN follows its frequency band
// (rain equalizer, like the node implementation): band energy raises that
// column's fall speed and brightness. Global spawn density follows volume. ---
fn fx_matrix(
    scr: &mut Screen,
    palette: &[String],
    intensity: i64,
    audio: &AudioState,
    rain: bool,
    use_theme_colors: bool,
    effect: &str,
) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = if rain {
        "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿ0123456789".chars().collect()
    } else {
        "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789ABCDEF"
            .chars()
            .collect()
    };
    let speed_base: i32 = 1 + (intensity / 4) as i32;
    let trail_base: usize = 8 + (intensity as usize) * 2;
    let (cols, rows) = (scr.cols, scr.rows);
    let mut head: Vec<i32> = (0..cols).map(|_| rng.gen_range(0..rows as i32)).collect();
    let mut prev_head: Vec<i32> = head.clone();
    let mut speed: Vec<i32> = (0..cols)
        .map(|_| rng.gen_range(speed_base..speed_base + 2).max(1))
        .collect();
    let mut trail: Vec<usize> = (0..cols)
        .map(|_| rng.gen_range(trail_base..trail_base + 12))
        .collect();
    // When the theme changes, only drops that RESPAWN after the change adopt the
    // new palette; drops already on screen keep their born color until they
    // scroll away. Each column's trail is painted entirely from its born palette
    // so every cell always has a real hue (never the white default foreground).
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut col_pal: Vec<Vec<String>> = (0..cols).map(|_| palette.to_vec()).collect();
    let mut theme_watcher = ThemeWatcher::new();
    loop {
        if scr.maybe_resize() {
            return Ok(());
        }
        // Detect theme changes live and update the palette future drops use.
        if use_theme_colors {
            if let Some(new_colors) = theme_watcher.changed() {
                cur_pal = theme_palette(new_colors, effect);
                set_theme_palette(cur_pal.clone());
                log_dbg(&format!(
                    "theme colors changed: {effect} palette updated ({})",
                    cur_pal.len()
                ));
            }
        }
        let vol = audio.volume();
        let spawn_chance = 0.35 + vol * 0.65;
        // A) Erase cells this column's trail vacated since the previous frame.
        for c in 0..cols {
            let h_prev = prev_head[c];
            let h = head[c];
            let vac_top = h_prev - trail[c] as i32 + 1;
            let vac_end = h - trail[c] as i32 + 1; // exclusive
            if vac_end > vac_top {
                for y in vac_top..vac_end.min(rows as i32) {
                    if y >= 0 && y < rows as i32 {
                        scr.erase(c, y as usize);
                    }
                }
            }
            // Column was reset/rested (head negative): clear its entire old trail
            if h < 0 && vac_top >= 0 {
                for y in vac_top..h_prev.min(rows as i32) {
                    if y >= 0 && y < rows as i32 {
                        scr.erase(c, y as usize);
                    }
                }
            }
        }
        let band = |c: usize| audio.band_at(c, cols);
        for c in 0..cols {
            let h = head[c];
            // Paint this column's drop entirely with ITS born palette.
            let pal = &col_pal[c];
            for t in 0..trail[c] {
                let y = h - t as i32;
                if y >= 0 && y < rows as i32 {
                    let ch = chars[rng.gen_range(0..chars.len())];
                    let idx = if t == 0 {
                        0
                    } else if t < 4 {
                        1
                    } else {
                        2
                    };
                    let idx = idx.min(pal.len().saturating_sub(1));
                    // fall back to a neutral theme hue if palette is empty
                    let color = pal
                        .get(idx)
                        .cloned()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "\x1b[32m".to_string());
                    scr.put(c, y as usize, ch, color);
                }
            }
            // Advance drop; on respawn adopt the current theme palette.
            prev_head[c] = h;
            head[c] = h + speed[c] + (band(c) * 2.0) as i32;
            if head[c] - trail[c] as i32 > rows as i32 {
                if rng.gen::<f32>() < spawn_chance {
                    head[c] = -(rng.gen_range(0..30));
                    speed[c] = rng.gen_range(speed_base..speed_base + 2).max(1);
                    trail[c] = rng.gen_range(trail_base..trail_base + 12);
                    col_pal[c] = cur_pal.clone();
                } else {
                    head[c] = -(rows as i32 + 10);
                }
            }
        }
        let frame_dur = frame_delay(40, live_intensity(intensity), live_speed(5), audio);
        scr.fps_overlay(frame_dur.as_secs_f64() * 1000.0);
        scr.present()?;
        thread::sleep(frame_dur);
    }
}

// --- wave: layered sine waves scrolling horizontally ---
fn fx_wave(
    scr: &mut Screen,
    palette: &[String],
    intensity: i64,
    audio: &AudioState,
    use_theme_colors: bool,
    effect: &str,
) -> Result<()> {
    let mut t = 0f32;
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut theme_watcher = ThemeWatcher::new();
    loop {
        if scr.maybe_resize() {
            return Ok(());
        }
        if use_theme_colors {
            if let Some(new_colors) = theme_watcher.changed() {
                cur_pal = theme_palette(new_colors, effect);
                set_theme_palette(cur_pal.clone());
                log_dbg(&format!(
                    "theme colors changed: {effect} palette updated ({})",
                    cur_pal.len()
                ));
            }
        }
        scr.clear_dirty();
        let lv = audio.volume();
        let (cols, rows) = (scr.cols, scr.rows);
        let cy = rows as f32 / 2.0;
        for layer in 0..3u8 {
            let amp = (rows as f32 * 0.18) * (0.5 + lv) * (1.0 - layer as f32 * 0.25);
            let freq = 0.045 + layer as f32 * 0.02;
            let phase = t * (1.0 + layer as f32 * 0.6);
            for x in 0..cols {
                let y = cy + amp * (x as f32 * freq + phase).sin();
                if y >= 0.0 && (y as usize) < rows {
                    scr.put(
                        x,
                        y as usize,
                        if layer == 0 { '~' } else { '-' },
                        tint_to_color(layer + 1, &cur_pal),
                    );
                }
            }
        }
        let frame_dur = frame_delay(45, live_intensity(intensity), live_speed(5), audio);
        scr.fps_overlay(frame_dur.as_secs_f64() * 1000.0);
        scr.present()?;
        t += 0.12 + lv * 0.25;
        thread::sleep(frame_dur);
    }
}

// --- bars: equalizer driven by the REAL per-band spectrum (each bar = one
// frequency band's energy), smoothed so it follows without flicker. ---
fn fx_bars(
    scr: &mut Screen,
    palette: &[String],
    intensity: i64,
    audio: &AudioState,
    use_theme_colors: bool,
    effect: &str,
) -> Result<()> {
    let bands = NBANDS;
    let mut heights = vec![0f32; bands];
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut theme_watcher = ThemeWatcher::new();
    loop {
        if scr.maybe_resize() {
            return Ok(());
        }
        if use_theme_colors {
            if let Some(new_colors) = theme_watcher.changed() {
                cur_pal = theme_palette(new_colors, effect);
                set_theme_palette(cur_pal.clone());
                log_dbg(&format!(
                    "theme colors changed: {effect} palette updated ({})",
                    cur_pal.len()
                ));
            }
        }
        scr.clear_dirty();
        let (cols, rows) = (scr.cols, scr.rows);
        let maxh = rows as f32 * 0.9;
        let bw = cols / bands.max(1);
        for b in 0..bands {
            let energy = audio.band_at(b * (cols / bands.max(1)), cols);
            let target = (energy * 1.1).min(1.0) * maxh;
            heights[b] += (target - heights[b]) * 0.4;
            let h = heights[b] as usize;
            for y in 0..h.min(rows) {
                let tint = if y as f32 > h as f32 * 0.7 {
                    1
                } else if y as f32 > h as f32 * 0.4 {
                    2
                } else {
                    3
                };
                for x in 0..bw.saturating_sub(1) {
                    scr.put(b * bw + x, rows - 1 - y, '#', tint_to_color(tint, &cur_pal));
                }
            }
        }
        let frame_dur = frame_delay(50, live_intensity(intensity), live_speed(5), audio);
        scr.fps_overlay(frame_dur.as_secs_f64() * 1000.0);
        scr.present()?;
        thread::sleep(frame_dur);
    }
}

// --- donut: classic 3D torus, scaled to the grid. Spin speed follows audio. ---
// Scale follows the original donut.c proportions (30/80 horizontal, 15/22
// vertical), which already bake in the terminal cell aspect. That keeps the
// torus round on any screen; compressing by cell_aspect over-flattened it.
fn fx_donut(
    scr: &mut Screen,
    palette: &[String],
    intensity: i64,
    audio: &AudioState,
    cell_aspect: f32,
    use_theme_colors: bool,
    effect: &str,
) -> Result<()> {
    let _ = cell_aspect;
    let mut a = 0f32;
    let mut e = 1f32;
    let (cols, rows) = (scr.cols, scr.rows);
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    let sx = cols as f32 * (30.0 / 80.0);
    let sy = rows as f32 * (15.0 / 22.0);
    let mut zbuf = vec![0f32; cols * rows];
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut theme_watcher = ThemeWatcher::new();
    loop {
        if scr.maybe_resize() {
            return Ok(());
        }
        if use_theme_colors {
            if let Some(new_colors) = theme_watcher.changed() {
                cur_pal = theme_palette(new_colors, effect);
                set_theme_palette(cur_pal.clone());
                log_dbg(&format!(
                    "theme colors changed: {effect} palette updated ({})",
                    cur_pal.len()
                ));
            }
        }
        scr.clear_dirty();
        for b in zbuf.iter_mut() {
            *b = 0.0;
        }
        let lv = audio.volume();
        let mut j = 0f32;
        while j < 6.28 {
            let mut i = 0f32;
            while i < 6.28 {
                let (sj, cj) = (j.sin(), j.cos());
                let (si, ci) = (i.sin(), i.cos());
                let (sa, ca) = (a.sin(), a.cos());
                let (se, ce) = (e.sin(), e.cos());
                let h = cj + 2.0;
                let d = 1.0 / (si * h * sa + sj * ca + 5.0);
                let t = si * h * ca - sj * sa;
                let x = (cx + sx * d * (ci * h * ce - t * se)) as i32;
                let y = (cy + sy * d * (ci * h * se + t * ce)) as i32;
                let lum = ((sj * sa - si * ca) * ce - ci * h * se - sj * ca - ci * h * sa) * 8.0;
                if y >= 0
                    && y < rows as i32
                    && x >= 0
                    && x < cols as i32
                    && d > zbuf[y as usize * cols + x as usize]
                {
                    zbuf[y as usize * cols + x as usize] = d;
                    let chars = b".,-~:;=!*#$@";
                    let ci2 = lum.max(0.0) as usize;
                    let ch = chars[ci2.min(chars.len() - 1)] as char;
                    let tint = if ci2 > 8 {
                        1
                    } else if ci2 > 4 {
                        2
                    } else {
                        3
                    };
                    scr.put(x as usize, y as usize, ch, tint_to_color(tint, &cur_pal));
                }
                i += 0.02;
            }
            j += 0.07;
        }
        let frame_dur = frame_delay(45, live_intensity(intensity), live_speed(5), audio);
        scr.fps_overlay(frame_dur.as_secs_f64() * 1000.0);
        scr.present()?;
        let spin = 1.0 + lv * 2.0;
        a += 0.04 * spin;
        e += 0.02 * spin;
        thread::sleep(frame_dur);
    }
}

// --- fire: classic doom fire from the bottom row. Height licks with audio. ---
fn fx_fire(
    scr: &mut Screen,
    palette: &[String],
    intensity: i64,
    audio: &AudioState,
    use_theme_colors: bool,
    effect: &str,
) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let palette_chars: Vec<char> = " .:-=+*#%@".chars().collect();
    let mut heat = vec![vec![0u8; cols]; rows];
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut theme_watcher = ThemeWatcher::new();
    loop {
        if scr.maybe_resize() {
            return Ok(());
        }
        if use_theme_colors {
            if let Some(new_colors) = theme_watcher.changed() {
                cur_pal = theme_palette(new_colors, effect);
                set_theme_palette(cur_pal.clone());
                log_dbg(&format!(
                    "theme colors changed: {effect} palette updated ({})",
                    cur_pal.len()
                ));
            }
        }
        let lv = audio.volume();
        let fuel = (28.0 + lv * 8.0 + live_intensity(intensity) as f32 * 0.4) as u8;
        for x in 0..cols {
            heat[rows - 1][x] = fuel.min(36);
        }
        for y in 0..rows - 1 {
            for x in 0..cols {
                let src_x = (x as i32 + rng.gen_range(-1..=1)).clamp(0, cols as i32 - 1) as usize;
                let decay = rng.gen_range(0..=2);
                heat[y][x] = heat[y + 1][src_x].saturating_sub(decay);
            }
        }
        scr.clear_dirty();
        for y in 0..rows {
            for x in 0..cols {
                let h = heat[y][x] as usize;
                if h > 0 {
                    let ci = (h * palette_chars.len() / 37).min(palette_chars.len() - 1);
                    let tint = if h > 24 {
                        1
                    } else if h > 12 {
                        2
                    } else {
                        3
                    };
                    scr.put(x, y, palette_chars[ci], tint_to_color(tint, &cur_pal));
                }
            }
        }
        let frame_dur = frame_delay(45, live_intensity(intensity), live_speed(5), audio);
        scr.fps_overlay(frame_dur.as_secs_f64() * 1000.0);
        scr.present()?;
        thread::sleep(frame_dur);
    }
}

// --- starfield: stars flying outward from the center. Speed follows audio. ---
fn fx_starfield(
    scr: &mut Screen,
    palette: &[String],
    intensity: i64,
    audio: &AudioState,
    use_theme_colors: bool,
    effect: &str,
) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    let n = 160usize;
    let mut stars: Vec<(f32, f32, f32)> = (0..n)
        .map(|_| {
            (
                rng.gen_range(-1.0..1.0),
                rng.gen_range(-1.0..1.0),
                rng.gen_range(0.05..1.0),
            )
        })
        .collect();
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut theme_watcher = ThemeWatcher::new();
    loop {
        if scr.maybe_resize() {
            return Ok(());
        }
        if use_theme_colors {
            if let Some(new_colors) = theme_watcher.changed() {
                cur_pal = theme_palette(new_colors, effect);
                set_theme_palette(cur_pal.clone());
                log_dbg(&format!(
                    "theme colors changed: {effect} palette updated ({})",
                    cur_pal.len()
                ));
            }
        }
        scr.clear_dirty();
        let lv = audio.volume();
        let speed = (0.006 + live_intensity(intensity) as f32 * 0.0012) * (1.0 + lv * 2.2);
        for s in stars.iter_mut() {
            s.2 -= speed;
            if s.2 <= 0.02 {
                *s = (rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0), 1.0);
            }
            let px = cx + s.0 / s.2 * cx * 0.5;
            let py = cy + s.1 / s.2 * cy * 0.5;
            if px >= 0.0 && px < cols as f32 && py >= 0.0 && py < rows as f32 {
                let depth = 1.0 - s.2;
                let (ch, tint) = if depth > 0.75 {
                    ('@', 1)
                } else if depth > 0.45 {
                    ('*', 2)
                } else {
                    ('.', 3)
                };
                scr.put(px as usize, py as usize, ch, tint_to_color(tint, &cur_pal));
            }
        }
        let frame_dur = frame_delay(40, live_intensity(intensity), live_speed(5), audio);
        scr.fps_overlay(frame_dur.as_secs_f64() * 1000.0);
        scr.present()?;
        thread::sleep(frame_dur);
    }
}

// --- life: Conway's Game of Life, reseeded on stagnation ---
fn fx_life(
    scr: &mut Screen,
    palette: &[String],
    intensity: i64,
    audio: &AudioState,
    use_theme_colors: bool,
    effect: &str,
) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let mut grid = vec![vec![false; cols]; rows];
    let mut next = vec![vec![false; cols]; rows];
    let mut age = vec![vec![0u8; cols]; rows];
    let density = 0.18 + audio.volume() * 0.1;
    for y in 0..rows {
        for x in 0..cols {
            grid[y][x] = rng.gen::<f32>() < density;
        }
    }
    let mut stagnant = 0u32;
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut theme_watcher = ThemeWatcher::new();
    loop {
        if scr.maybe_resize() {
            return Ok(());
        }
        if use_theme_colors {
            if let Some(new_colors) = theme_watcher.changed() {
                cur_pal = theme_palette(new_colors, effect);
                set_theme_palette(cur_pal.clone());
                log_dbg(&format!(
                    "theme colors changed: {effect} palette updated ({})",
                    cur_pal.len()
                ));
            }
        }
        scr.clear_dirty();
        for y in 0..rows {
            for x in 0..cols {
                if grid[y][x] {
                    age[y][x] = age[y][x].saturating_add(1);
                    let tint = if age[y][x] > 30 {
                        1
                    } else if age[y][x] > 8 {
                        2
                    } else {
                        3
                    };
                    scr.put(
                        x,
                        y,
                        if age[y][x] > 8 { 'O' } else { 'o' },
                        tint_to_color(tint, &cur_pal),
                    );
                } else {
                    age[y][x] = 0;
                }
            }
        }
        let frame_dur = frame_delay(70, live_intensity(intensity), live_speed(5), audio);
        scr.fps_overlay(frame_dur.as_secs_f64() * 1000.0);
        scr.present()?;
        let mut changed = 0u32;
        for y in 0..rows {
            for x in 0..cols {
                let mut n = 0;
                for dy in [-1i32, 0, 1] {
                    for dx in [-1i32, 0, 1] {
                        if dy == 0 && dx == 0 {
                            continue;
                        }
                        let yy = (y as i32 + dy).rem_euclid(rows as i32) as usize;
                        let xx = (x as i32 + dx).rem_euclid(cols as i32) as usize;
                        if grid[yy][xx] {
                            n += 1;
                        }
                    }
                }
                next[y][x] = if grid[y][x] { n == 2 || n == 3 } else { n == 3 };
                if next[y][x] != grid[y][x] {
                    changed += 1;
                }
            }
        }
        std::mem::swap(&mut grid, &mut next);
        if changed < cols as u32 / 8 {
            stagnant += 1;
            if stagnant > 30 {
                stagnant = 0;
                let density = 0.18 + audio.volume() * 0.1;
                for y in 0..rows {
                    for x in 0..cols {
                        if rng.gen::<f32>() < density * 0.25 {
                            grid[y][x] = true;
                        }
                    }
                }
            }
        } else {
            stagnant = 0;
        }
        thread::sleep(frame_dur);
    }
}

#[cfg(test)]
mod speed_tests {
    use super::*;

    #[test]
    fn speed_panel_displays_multiplier_and_reaches_twenty() {
        let panel = include_str!("../Panel.qml");
        assert!(panel.contains("(root.speed / 5).toFixed(1) + \"x\""));
        assert!(panel.contains("minimum: 1; maximum: 100; step: 1; integer: true"));
        assert!(panel.contains("Math.min(100, s.speed)"));
    }

    #[test]
    fn speed_writer_round_trips_and_bounds_values() {
        let home = std::env::temp_dir().join(format!(
            "ttfx-speed-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&home).unwrap();
        let state = home.join(".local/state/omarchy/audio-background/state.json");
        let write = |args: &[&str]| {
            let result = std::process::Command::new("sh")
                .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../write_state.sh"))
                .args(args)
                .env("HOME", &home)
                .output()
                .unwrap();
            assert!(result.status.success(), "{:?}", result);
            std::fs::read_to_string(&state).unwrap()
        };
        assert_eq!(json_num(&write(&[]), "speed"), Some(5));
        for (input, expected) in [
            ("1", 1),
            ("5", 5),
            ("20", 20),
            ("100", 100),
            ("101", 100),
            ("999999999999999999999", 100),
            ("0", 1),
            ("-1", 1),
            ("oops", 5),
        ] {
            let text = write(&[&format!("speed={input}"), "effect=rain"]);
            assert_eq!(json_num(&text, "speed"), Some(expected), "input={input}");
            assert_eq!(json_str(&text, "effect").as_deref(), Some("rain"));
        }
        write(&["speed=100"]);
        assert_eq!(json_num(&write(&["audio=0"]), "speed"), Some(100));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn speed_pacing_is_shared_and_clamped_without_a_six_ms_ceiling() {
        let audio = AudioState::start(false);
        for value in [1, 5, 20, 100] {
            let expected = Duration::from_millis(8).div_f64(value as f64 / 5.0);
            assert_eq!(speed_delay(Duration::from_millis(8), value), expected);
            assert_eq!(frame_delay(8, 0, value, &audio), expected);
        }
        assert_eq!(speed_multiplier(i64::MIN), 0.2);
        assert_eq!(speed_multiplier(i64::MAX), 20.0);
    }

    #[test]
    fn speed_default_is_unchanged_and_max_is_twenty_times_faster() {
        let audio = AudioState::start(false);
        assert_eq!(Config::default().speed, 5);
        let baseline = frame_delay(40, 5, 5, &audio);
        assert_eq!(baseline, Duration::from_millis(25));
        assert_eq!(frame_delay(40, 5, 100, &audio), baseline.div_f64(20.0));
    }
}

#[cfg(test)]
mod selective_theme_tests {
    use super::*;
    use ttfx::utils::graphics::{Color, Gradient};

    #[test]
    fn burn_preserves_fire_and_themes_only_accents() {
        let theme = vec![("accent".to_string(), "#204080".to_string())];
        let transform = ttfx_theme_transform("burn", &theme).unwrap();
        let stops = ["ffffff", "fff75d", "fe650d", "8A003C", "510100"]
            .iter()
            .map(|hex| Color::from_hex(hex).unwrap())
            .collect::<Vec<_>>();
        for color in &Gradient::with_steps(&stops, 10, false).unwrap().spectrum {
            let rgb = parse_hex_color(&color.rgb_color.to_string()).unwrap();
            assert_eq!(
                transform(rgb.0, rgb.1, rgb.2),
                rgb,
                "fire color changed: {rgb:?}"
            );
        }
        assert_eq!(transform(0, 0, 0), (0, 0, 0));
        assert_eq!(transform(0, 195, 255), (32, 64, 128));
        assert_eq!(transform(128, 128, 128), (16, 32, 64));
    }

    #[test]
    fn every_effect_has_a_safe_policy() {
        let theme = vec![("accent".to_string(), "#204080".to_string())];
        for effect in TTFX_EFFECTS {
            let transform = ttfx_theme_transform(effect, &theme);
            assert!(transform.is_some(), "{effect} has no theme policy");
        }
    }

    #[test]
    fn missing_theme_configuration_keeps_every_ttfx_effect_native() {
        for effect in TTFX_EFFECTS {
            assert!(
                ttfx_theme_transform(effect, &[]).is_none(),
                "{effect} should retain its built-in palette without theme colors"
            );
        }
    }

    #[test]
    fn semantic_effects_map_only_final_accents() {
        let theme = vec![("accent".to_string(), "#204080".to_string())];
        let blackhole = ttfx_theme_transform("blackhole", &theme).unwrap();
        assert_eq!(blackhole(255, 204, 13), (255, 204, 13)); // explosion yellow
        assert_eq!(blackhole(138, 0, 138), (17, 35, 69)); // final magenta, value preserved
        let thunder = ttfx_theme_transform("thunderstorm", &theme).unwrap();
        assert_eq!(thunder(104, 163, 232), (104, 163, 232)); // lightning blue
        assert_eq!(thunder(138, 0, 138), (17, 35, 69)); // final magenta, value preserved
    }

    #[test]
    fn auto_degrade_requires_full_sustained_sub_10_fps_window() {
        assert!(!should_auto_degrade(90, 9.0)); // 10 FPS, but only 9 seconds
        assert!(!should_auto_degrade(149, 14.99));
        assert!(!should_auto_degrade(150, 15.0)); // exactly 10 FPS is healthy
        assert!(should_auto_degrade(149, 15.0)); // 9.93 FPS for full window
        assert!(should_auto_degrade(100, 20.0)); // 5 FPS sustained
    }

    #[test]
    fn embedded_catalog_matches_every_upstream_effect_except_local_matrix_and_rain() {
        use clap::CommandFactory;
        let mut upstream: Vec<String> = ttfx::cli::Cli::command()
            .get_subcommands()
            .map(|command| command.get_name().to_string())
            .filter(|name| name != "matrix" && name != "rain")
            .collect();
        let mut embedded: Vec<String> = TTFX_EFFECTS.iter().map(|name| name.to_string()).collect();
        upstream.sort();
        embedded.sort();
        assert_eq!(embedded, upstream);
    }

    #[test]
    fn panel_catalog_matches_embedded_catalog() {
        let panel = include_str!("../Panel.qml");
        let block = panel
            .split("readonly property var ttfxEffects: [")
            .nth(1)
            .unwrap()
            .split(']')
            .next()
            .unwrap();
        let mut panel_names: Vec<String> = block
            .split('"')
            .enumerate()
            .filter_map(|(index, part)| {
                if index % 2 == 1 {
                    Some(part.to_string())
                } else {
                    None
                }
            })
            .collect();
        let mut embedded: Vec<String> = TTFX_EFFECTS.iter().map(|name| name.to_string()).collect();
        panel_names.sort();
        embedded.sort();
        assert_eq!(panel_names, embedded);
    }

    #[test]
    fn every_safe_policy_changes_an_accent_between_two_themes() {
        let warm = vec![("accent".to_string(), "#ff8040".to_string())];
        let cool = vec![("accent".to_string(), "#4080ff".to_string())];
        for effect in TTFX_EFFECTS {
            let sample = if effect == "burn" {
                (0, 195, 255)
            } else {
                final_accent_set(effect)
                    .iter()
                    .copied()
                    .next()
                    .unwrap_or((138, 0, 138))
            };
            let a = ttfx_theme_transform(effect, &warm).unwrap()(sample.0, sample.1, sample.2);
            let b = ttfx_theme_transform(effect, &cool).unwrap()(sample.0, sample.1, sample.2);
            assert_ne!(
                a, b,
                "{effect} did not react to theme change for {sample:?}"
            );
        }
    }

    #[test]
    fn glyph_style_changes_only_the_ttfx_input_letters() {
        let native = ttfx_canvas_input("OMARCHY", 200, 64, "native");
        let block = ttfx_canvas_input("OMARCHY", 200, 64, "block");
        let lower_o = ttfx_canvas_input("OMARCHY", 200, 64, "lower_o");
        let upper_o = ttfx_canvas_input("OMARCHY", 200, 64, "upper_o");
        assert!(!native.contains('#'));
        assert!(native.contains('█'));
        assert!(block.contains('█'));
        assert!(!block.contains('#'));
        assert!(lower_o.contains('o'));
        assert!(!lower_o.contains('#'));
        assert!(upper_o.contains('O'));
        assert!(!upper_o.contains('#'));
    }
}

#[cfg(test)]
mod boot_handoff_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn saved_off_bridge_stays_after_deadline_without_mapped_background() {
        let cfg = Config {
            running: false,
            ..Config::default()
        };
        LAST_GOOD_CONFIG.with(|saved| *saved.borrow_mut() = Some(cfg.clone()));
        let state = Rc::new(RefCell::new(ControllerHandoff::new(
            Some(BootHandoff {
                effect: "rain".into(),
                seed: 7,
                ready_file: "/unused".into(),
                unhide_cursor_on_ready: false,
                scene_settings: BootSceneSettings::default(),
                phase: None,
            }),
            1,
        )));
        state
            .borrow()
            .coordinator
            .as_ref()
            .unwrap()
            .borrow_mut()
            .ready = true;
        state.borrow_mut().cursor_released = true;
        let windows = Rc::new(RefCell::new(Vec::new()));
        let last = Rc::new(RefCell::new(cfg.clone()));
        let effect = Rc::new(RefCell::new("rain".into()));
        assert!(!normalize_handoff(
            &state,
            HANDOFF_NORMALIZATION_DEADLINE - Duration::from_nanos(1),
            false,
            &windows,
            "unused",
            &cfg,
            &last,
            &effect
        ));
        assert!(state.borrow().is_active());
        assert_eq!(*effect.borrow(), "rain");
        assert!(!normalize_handoff(
            &state,
            HANDOFF_NORMALIZATION_DEADLINE,
            false,
            &windows,
            "unused",
            &cfg,
            &last,
            &effect
        ));
        assert!(state.borrow().is_active());
        assert!(!last.borrow().running);
        LAST_GOOD_CONFIG.with(|saved| *saved.borrow_mut() = None);
    }

    #[test]
    fn missing_config_retains_selected_bridge_even_at_recovery_deadline() {
        LAST_GOOD_CONFIG.with(|saved| *saved.borrow_mut() = None);
        let cfg = Config::default();
        let state = Rc::new(RefCell::new(ControllerHandoff::new(
            Some(BootHandoff {
                effect: "rain".into(),
                seed: 7,
                ready_file: "/unused".into(),
                unhide_cursor_on_ready: false,
                scene_settings: BootSceneSettings::default(),
                phase: None,
            }),
            1,
        )));
        state.borrow_mut().cursor_released = true;
        let windows = Rc::new(RefCell::new(Vec::new()));
        let last = Rc::new(RefCell::new(cfg.clone()));
        let effect = Rc::new(RefCell::new("rain".into()));
        assert!(!normalize_handoff(
            &state,
            HANDOFF_NORMALIZATION_DEADLINE,
            false,
            &windows,
            "unused",
            &cfg,
            &last,
            &effect
        ));
        assert_eq!(*effect.borrow(), "rain");
        assert!(state.borrow().is_active());
        assert!(state.borrow().successor.is_none());
    }

    #[test]
    fn animated_successor_only_releases_bridge_after_its_own_paint() {
        for ready in [false, true] {
            let cfg = Config {
                running: true,
                ..Config::default()
            };
            LAST_GOOD_CONFIG.with(|saved| *saved.borrow_mut() = Some(cfg.clone()));
            let state = Rc::new(RefCell::new(ControllerHandoff::new(
                Some(BootHandoff {
                    effect: "rain".into(),
                    seed: 7,
                    ready_file: "/unused".into(),
                    unhide_cursor_on_ready: false,
                    scene_settings: BootSceneSettings::default(),
                    phase: None,
                }),
                1,
            )));
            state
                .borrow()
                .coordinator
                .as_ref()
                .unwrap()
                .borrow_mut()
                .ready = true;
            state.borrow_mut().cursor_released = true;
            state.borrow_mut().successor = Some(PendingSuccessor {
                windows: Rc::new(RefCell::new(Vec::new())),
                coordinator: Rc::new(RefCell::new(PaintCoordinator {
                    expected: 1,
                    painted: Default::default(),
                    marker: None,
                    unhide_cursor: false,
                    ready,
                })),
                config: cfg.clone(),
                failed: false,
                started: std::time::Instant::now(),
            });
            let windows = Rc::new(RefCell::new(Vec::new()));
            let last = Rc::new(RefCell::new(cfg.clone()));
            let effect = Rc::new(RefCell::new("rain".into()));
            assert_eq!(
                normalize_handoff(
                    &state,
                    Duration::from_secs(1),
                    false,
                    &windows,
                    "unused",
                    &cfg,
                    &last,
                    &effect
                ),
                ready
            );
            assert_eq!(state.borrow().is_active(), !ready);
            if !ready {
                state.borrow_mut().successor.as_mut().unwrap().started =
                    std::time::Instant::now() - HANDOFF_NORMALIZATION_DEADLINE;
                assert!(!normalize_handoff(
                    &state,
                    HANDOFF_NORMALIZATION_DEADLINE,
                    false,
                    &windows,
                    "unused",
                    &cfg,
                    &last,
                    &effect
                ));
                assert!(state.borrow().is_active());
                assert!(state.borrow().successor.as_ref().unwrap().failed);
                assert_eq!(*effect.borrow(), "rain");
            }
        }
        LAST_GOOD_CONFIG.with(|saved| *saved.borrow_mut() = None);
    }

    #[test]
    fn successor_wait_is_bounded_and_requires_its_own_paint() {
        assert!(!successor_wait_finished(false, Duration::ZERO));
        assert!(!successor_wait_finished(
            false,
            HANDOFF_NORMALIZATION_DEADLINE - Duration::from_nanos(1)
        ));
        assert!(successor_wait_finished(
            false,
            HANDOFF_NORMALIZATION_DEADLINE
        ));
        assert!(successor_wait_finished(true, Duration::ZERO));
        let mut successor = PaintCoordinator {
            expected: 2,
            painted: Default::default(),
            marker: None,
            unhide_cursor: false,
            ready: false,
        };
        assert!(!successor.after_paint(0).unwrap());
        assert!(!successor.is_ready());
        assert!(successor.after_paint(1).unwrap());
        assert!(successor.is_ready());
    }

    #[test]
    fn controller_handoff_is_complete_validated_and_seeded() {
        let args = strings(&[
            "ttfx-bg-rs",
            "--boot-effect",
            "rain",
            "--boot-seed",
            "18446744073709551615",
            "--ready-file",
            "/run/user/1000/ttfx.ready",
            "--unhide-cursor-on-ready",
        ]);
        let handoff = parse_controller_handoff(&args).unwrap().unwrap();
        assert_eq!(handoff.effect, "rain");
        assert_eq!(handoff.seed, u64::MAX);
        assert_eq!(
            handoff.ready_file,
            PathBuf::from("/run/user/1000/ttfx.ready")
        );
        assert!(handoff.unhide_cursor_on_ready);

        for bad in [
            strings(&["ttfx-bg-rs", "--boot-effect"]),
            strings(&[
                "ttfx-bg-rs",
                "--boot-effect",
                "burn",
                "--boot-seed",
                "-1",
                "--ready-file",
                "/tmp/r",
            ]),
            strings(&[
                "ttfx-bg-rs",
                "--boot-effect",
                "unknown",
                "--boot-seed",
                "1",
                "--ready-file",
                "/tmp/r",
            ]),
            strings(&["ttfx-bg-rs", "--boot-effect", "burn", "--boot-seed", "1"]),
            strings(&["ttfx-bg-rs", "--unknown", "value"]),
        ] {
            assert!(parse_controller_handoff(&bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn renderer_consumes_byline_text_and_theme_values_that_look_like_controls() {
        let args = strings(&[
            "ttfx-bg-rs",
            "--render",
            "--byline",
            "--seed",
            "--ttfx-text",
            "--paint-socket",
            "--char-style",
            "--effect",
            "--effect",
            "rain",
            "--seed",
            "7",
            "--paint-socket",
            "/tmp/paint.sock",
        ]);
        let parsed = parse_render_options(&args).unwrap();
        assert_eq!(parsed.effect, "rain");
        assert_eq!(parsed.byline, "--seed");
        assert_eq!(parsed.ttfx_text, "--paint-socket");
        assert_eq!(parsed.char_style, "native");
        assert_eq!(parsed.seed, Some(7));
        assert_eq!(parsed.paint_socket, Some(PathBuf::from("/tmp/paint.sock")));
    }

    #[test]
    fn renderer_parser_populates_every_child_option() {
        let parsed = parse_render_options(&strings(&[
            "ttfx-bg-rs",
            "--render",
            "--effect",
            "rain",
            "--cols",
            "101",
            "--rows",
            "42",
            "--intensity",
            "8",
            "--speed",
            "17",
            "--audio",
            "1",
            "--byline",
            "credits",
            "--ttfx-text",
            "HELLO",
            "--reactivity",
            "4",
            "--intro-size",
            "3",
            "--cell-aspect",
            "1.75",
            "--show-fps",
            "1",
            "--intro-beat-sync",
            "0",
            "--use-theme-colors",
            "0",
            "--transparent-background",
            "1",
            "--char-style",
            "block",
            "--boot-char-style",
            "dot",
            "--no-intro",
            "--seed",
            "99",
            "--paint-socket",
            "/tmp/paint.sock",
        ]))
        .unwrap();
        assert_eq!(parsed.effect, "rain");
        assert_eq!(parsed.cols, 101);
        assert_eq!(parsed.rows, 42);
        assert_eq!(parsed.intensity, 8);
        assert_eq!(parsed.speed, 17);
        assert!(parsed.audio);
        assert_eq!(parsed.byline, "credits");
        assert_eq!(parsed.ttfx_text, "HELLO");
        assert_eq!(parsed.reactivity, 4);
        assert_eq!(parsed.intro_size, 3);
        assert_eq!(parsed.cell_aspect, 1.75);
        assert!(parsed.show_fps);
        assert!(!parsed.intro_beat_sync);
        assert!(!parsed.use_theme_colors);
        assert!(parsed.transparent_background);
        assert_eq!(parsed.char_style, "block");
        assert_eq!(parsed.boot_char_style, "dot");
        assert!(!parsed.show_intro);
        assert_eq!(parsed.seed, Some(99));
        assert_eq!(parsed.paint_socket, Some(PathBuf::from("/tmp/paint.sock")));
    }

    #[test]
    fn renderer_rejects_duplicate_missing_and_unknown_controls() {
        for bad in [
            strings(&["ttfx-bg-rs", "--render", "--cols", "1", "--cols", "2"]),
            strings(&["ttfx-bg-rs", "--render", "--no-intro", "--no-intro"]),
            strings(&["ttfx-bg-rs", "--render", "--rows"]),
            strings(&["ttfx-bg-rs", "--render", "--unknown"]),
        ] {
            assert!(parse_render_options(&bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn renderer_structurally_accepts_one_paint_socket() {
        let args = strings(&[
            "ttfx-bg-rs",
            "--render",
            "--effect",
            "burn",
            "--paint-socket",
            "/run/user/1000/ttfx-paint.sock",
        ]);
        let parsed = parse_render_options(&args).unwrap();
        assert_eq!(
            parsed.paint_socket,
            Some(PathBuf::from("/run/user/1000/ttfx-paint.sock"))
        );
        assert!(parse_render_options(&strings(&[
            "ttfx-bg-rs",
            "--render",
            "--paint-socket",
            "/tmp/a",
            "--paint-socket=/tmp/b",
        ]))
        .is_err());
        assert!(
            parse_render_options(&strings(&["ttfx-bg-rs", "--render", "--paint-socket",])).is_err()
        );
    }

    #[test]
    fn renderer_accepts_only_seed_from_handoff_parent() {
        let handoff = BootHandoff {
            effect: "burn".into(),
            seed: 42,
            ready_file: PathBuf::from("/tmp/boot.ready"),
            unhide_cursor_on_ready: true,
            scene_settings: BootSceneSettings::default(),
            phase: None,
        };
        let mut argv = Vec::new();
        append_renderer_handoff_args(
            &mut argv,
            Some(&handoff),
            Some(Path::new("/tmp/paint.sock")),
        );
        assert_eq!(
            argv,
            strings(&["--seed", "42", "--paint-socket", "/tmp/paint.sock"])
        );
        assert!(!argv.iter().any(|arg| arg.starts_with("--ready-file")));
    }

    #[test]
    fn seeded_matrix_and_rain_use_the_vendored_engine_only_for_handoff() {
        assert!(use_ttfx_backend("matrix", Some(7)));
        assert!(use_ttfx_backend("rain", Some(7)));
        assert!(!use_ttfx_backend("matrix", None));
        assert!(!use_ttfx_backend("rain", None));
        assert!(use_ttfx_backend("burn", None));
        for effect in ["matrix", "rain"] {
            let built = build_ttfx_effect(effect);
            assert!(built.is_some(), "vendored registry cannot build {effect}");
        }
    }

    #[test]
    fn supplied_ttfx_seed_uses_repeatable_rng_stream() {
        let mut first = ttfx_rng(Some(42));
        let mut second = ttfx_rng(Some(42));
        let a: Vec<f64> = (0..8).map(|_| first.random()).collect();
        let b: Vec<f64> = (0..8).map(|_| second.random()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn handoff_forces_only_the_initial_render_when_saved_running_is_false() {
        assert!(should_spawn_layers(false, true));
        assert!(!should_spawn_layers(false, false));
        assert!(should_spawn_layers(true, false));
    }

    #[test]
    fn validated_monitor_items_receive_compact_ids() {
        let raw = [Some("left"), None, Some("right")];
        let monitors = collect_valid_items(raw.len() as u32, |index| raw[index as usize]);
        let identified: Vec<(u32, &str)> = monitors
            .into_iter()
            .enumerate()
            .map(|(id, monitor)| (id as u32, monitor))
            .collect();
        assert_eq!(identified, vec![(0, "left"), (1, "right")]);
    }

    #[test]
    fn handoff_normalization_deadline_is_inclusive() {
        let mut before = HandoffNormalization::default();
        assert!(!before.resolve(
            false,
            HANDOFF_NORMALIZATION_DEADLINE - Duration::from_nanos(1),
            false
        ));
        let mut at = HandoffNormalization::default();
        assert!(at.resolve(false, HANDOFF_NORMALIZATION_DEADLINE, false));
    }

    #[test]
    fn handoff_normalizes_exactly_once_on_readiness_or_deadline() {
        let mut ready = HandoffNormalization::default();
        assert!(ready.resolve(true, Duration::ZERO, false));
        assert!(!ready.resolve(true, HANDOFF_NORMALIZATION_DEADLINE, false));

        let mut deadline = HandoffNormalization::default();
        assert!(deadline.resolve(false, HANDOFF_NORMALIZATION_DEADLINE, false));
        assert!(!deadline.resolve(false, HANDOFF_NORMALIZATION_DEADLINE * 2, false));
    }

    #[test]
    fn missing_readiness_cannot_force_saved_running_forever() {
        let mut normalization = HandoffNormalization::default();
        assert!(should_spawn_layers(false, normalization.handoff_active()));
        assert!(normalization.resolve(false, HANDOFF_NORMALIZATION_DEADLINE, false));
        assert!(!normalization.handoff_active());
        assert!(!should_spawn_layers(false, normalization.handoff_active()));
    }

    #[test]
    fn monitor_change_aborts_handoff_into_safe_normalization() {
        let mut normalization = HandoffNormalization::default();
        assert!(normalization.resolve(false, Duration::ZERO, true));
        assert!(!normalization.handoff_active());
        assert!(!normalization.resolve(true, HANDOFF_NORMALIZATION_DEADLINE, false));
    }

    #[test]
    fn pre_handshake_content_is_ignored_then_content_and_afterpaint_report_once() {
        let mut surface = SurfaceReadiness::default();
        surface.contents_changed();
        assert!(!surface.after_paint());
        surface.arm();
        assert!(!surface.after_paint());
        surface.contents_changed();
        assert!(surface.after_paint());
        assert!(!surface.after_paint(), "surface reported more than once");
    }

    #[test]
    fn paint_socket_paths_are_unique_private_and_removed_on_drop() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-paint-paths-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let first = ParentPaintSocket::bind_in(&dir, 0, Duration::from_secs(1)).unwrap();
        let second = ParentPaintSocket::bind_in(&dir, 0, Duration::from_secs(1)).unwrap();
        assert_ne!(first.path(), second.path());
        assert_eq!(
            std::fs::metadata(first.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let first_path = first.path().to_path_buf();
        drop(first);
        assert!(!first_path.exists());
        drop(second);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn child_does_not_announce_without_an_actual_effect_frame() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-no-frame-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("paint.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut handshake = Some(PreparedFrameHandshake::with_timeout(
            path,
            Duration::from_millis(50),
        ));
        assert!(gate_prepared_frame(None::<u8>, &mut handshake)
            .unwrap()
            .is_none());
        assert!(
            listener.accept().is_err(),
            "prep/build failure announced readiness"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn child_ack_wait_is_bounded() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-ack-timeout-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("paint.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut prepared = [0u8; 9];
            stream.read_exact(&mut prepared).unwrap();
            assert_eq!(&prepared, b"PREPARED\n");
            thread::sleep(Duration::from_millis(250));
        });
        let start = std::time::Instant::now();
        let mut handshake = PreparedFrameHandshake::with_timeout(path, Duration::from_millis(60));
        assert!(handshake.before_first_frame().is_err());
        assert!(start.elapsed() < Duration::from_millis(200));
        server.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spawn_callback_records_expected_renderer_pid() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-spawn-pid-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let parent = Rc::new(RefCell::new(
            ParentPaintSocket::bind_in(&dir, 0, Duration::from_secs(1)).unwrap(),
        ));
        handle_renderer_spawn_result(Some(&parent), Ok(std::process::id() as i32)).unwrap();
        assert_eq!(parent.borrow().expected_pid, Some(std::process::id()));
        drop(parent);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spawn_diagnostics_never_include_free_form_text() {
        let messages = [
            renderer_spawn_diagnostic("decrypt", 81, 10),
            ttfx_runtime_diagnostic("decrypt", 81, 10, 2, true),
            render_runtime_diagnostic("decrypt", 81, 10, true, false),
        ];
        for message in messages {
            assert!(message.contains("decrypt"));
            assert!(!message.contains("private byline"));
            assert!(!message.contains("private ttfx text"));
        }
    }

    #[test]
    fn debug_log_is_private_and_rejects_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = std::env::temp_dir().join(format!(
            "ttfx-log-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let private = dir.join("debug.log");
        append_private_log(&private, b"safe\n").unwrap();
        let metadata = std::fs::metadata(&private).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.permissions().mode() & 0o077, 0);
        let target = dir.join("target");
        std::fs::write(&target, b"unchanged").unwrap();
        let link = dir.join("link");
        symlink(&target, &link).unwrap();
        assert!(append_private_log(&link, b"injected").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"unchanged");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn paint_socket_subprocess_client() {
        let Ok(path) = std::env::var("TTFX_TEST_PAINT_SOCKET") else {
            return;
        };
        let expect_ack = std::env::var_os("TTFX_TEST_EXPECT_ACK").is_some();
        let mut stream = UnixStream::connect(path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream.write_all(b"PREPARED\n").unwrap();
        let mut ack = [0u8; 4];
        let result = stream.read_exact(&mut ack);
        if expect_ack {
            result.unwrap();
            assert_eq!(&ack, b"ACK\n");
        } else {
            assert!(result.is_err(), "unauthorized client received an ACK");
        }
    }

    fn spawn_paint_client(path: &Path, expect_ack: bool) -> std::process::Child {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "boot_handoff_tests::paint_socket_subprocess_client",
            ])
            .env("TTFX_TEST_PAINT_SOCKET", path);
        if expect_ack {
            command.env("TTFX_TEST_EXPECT_ACK", "1");
        }
        command.spawn().unwrap()
    }

    #[test]
    fn unset_expected_pid_does_not_ack_or_arm() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-unset-pid-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut parent = ParentPaintSocket::bind_in(&dir, 0, Duration::from_secs(1)).unwrap();
        let mut stream = UnixStream::connect(parent.path()).unwrap();
        stream.set_nonblocking(true).unwrap();
        stream.write_all(b"PREPARED\n").unwrap();
        let mut surface = SurfaceReadiness::default();
        for _ in 0..50 {
            parent.poll(&mut surface).unwrap();
            if parent.received == b"PREPARED\n" {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(parent.received, b"PREPARED\n");
        assert!(!surface.is_armed());
        let mut ack = [0u8; 4];
        assert!(
            matches!(stream.read(&mut ack), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        drop(parent);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wrong_same_uid_pid_is_rejected_without_arming() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-wrong-pid-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut parent = ParentPaintSocket::bind_in(&dir, 0, Duration::from_secs(2)).unwrap();
        parent.set_expected_pid(std::process::id());
        let mut child = spawn_paint_client(parent.path(), false);
        let mut surface = SurfaceReadiness::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            parent.poll(&mut surface).unwrap();
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            child.try_wait().unwrap().is_some(),
            "unauthorized child remained connected"
        );
        assert!(!surface.is_armed());
        drop(parent);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn actual_renderer_child_waits_for_expected_pid_then_is_acked() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-child-pid-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut parent = ParentPaintSocket::bind_in(&dir, 0, Duration::from_secs(2)).unwrap();
        let mut child = spawn_paint_client(parent.path(), true);
        let mut surface = SurfaceReadiness::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while parent.received != b"PREPARED\n" && std::time::Instant::now() < deadline {
            parent.poll(&mut surface).unwrap();
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(parent.received, b"PREPARED\n");
        assert!(!surface.is_armed());
        assert!(
            child.try_wait().unwrap().is_none(),
            "child continued before PID was authenticated"
        );
        parent.set_expected_pid(child.id());
        let ack_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !surface.is_armed() && std::time::Instant::now() < ack_deadline {
            parent.poll(&mut surface).unwrap();
            thread::sleep(Duration::from_millis(2));
        }
        assert!(surface.is_armed());
        assert!(child.wait().unwrap().success());
        drop(parent);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn correct_renderer_pid_is_acked_and_arms_surface() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-prepared-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut parent = ParentPaintSocket::bind_in(&dir, 4, Duration::from_secs(1)).unwrap();
        parent.set_expected_pid(std::process::id());
        let path = parent.path().to_path_buf();
        let child = thread::spawn(move || {
            let mut handshake = PreparedFrameHandshake::with_timeout(path, Duration::from_secs(1));
            handshake.before_first_frame().unwrap();
        });
        let mut surface = SurfaceReadiness::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !surface.is_armed() && std::time::Instant::now() < deadline {
            parent.poll(&mut surface).unwrap();
            thread::sleep(Duration::from_millis(2));
        }
        assert!(surface.is_armed());
        child.join().unwrap();
        surface.contents_changed();
        assert!(surface.after_paint());
        assert!(!surface.after_paint());
        drop(parent);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn first_monitor_cannot_publish_until_all_distinct_monitors_paint() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-ready-all-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("desktop.ready");
        let mut coordinator =
            PaintCoordinator::new(3, ReadyMarker::new(path.clone(), "rain", 42), false);

        assert!(!coordinator.after_paint(0).unwrap());
        assert!(!path.exists(), "first child won readiness race");
        assert!(
            !coordinator.after_paint(0).unwrap(),
            "duplicate monitor counted twice"
        );
        assert!(!coordinator.after_paint(2).unwrap());
        assert!(!path.exists());
        assert!(coordinator.after_paint(1).unwrap());
        assert_eq!(coordinator.painted_count(), 3);
        let marker = std::fs::read_to_string(&path).unwrap();
        assert!(marker.starts_with("status=frame-ready\neffect=rain\nseed=42\nready_ns="));
        let ready_ns = marker
            .strip_prefix("status=frame-ready\neffect=rain\nseed=42\nready_ns=")
            .unwrap()
            .strip_suffix('\n')
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert!(ready_ns > 0);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_ready_marker_is_atomically_replaced_by_this_handoff() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-ready-existing-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("desktop.ready");
        std::fs::write(
            &path,
            "status=frame-ready\neffect=burn\nseed=9\nready_ns=1\n",
        )
        .unwrap();
        let mut coordinator =
            PaintCoordinator::new(1, ReadyMarker::new(path.clone(), "burn", 9), false);
        assert!(coordinator.after_paint(0).unwrap());
        assert!(coordinator.is_ready());
        let marker = std::fs::read_to_string(&path).unwrap();
        assert!(marker.starts_with("status=frame-ready\neffect=burn\nseed=9\nready_ns="));
        assert_ne!(
            marker,
            "status=frame-ready\neffect=burn\nseed=9\nready_ns=1\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn zero_monitors_and_out_of_range_monitor_never_publish() {
        let dir = std::env::temp_dir().join(format!(
            "ttfx-ready-none-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("desktop.ready");
        let mut none = PaintCoordinator::new(0, ReadyMarker::new(path.clone(), "burn", 1), false);
        assert!(!none.after_paint(0).unwrap());
        assert!(!path.exists());
        let mut two = PaintCoordinator::new(2, ReadyMarker::new(path.clone(), "burn", 1), false);
        assert!(!two.after_paint(9).unwrap());
        assert_eq!(two.painted_count(), 0);
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn service_guard_uses_fixed_shell_program_and_quoted_argv() {
        let qml = include_str!("../Service.qml");
        assert!(qml.contains("omarchy-plymouth-handoff.active"));
        assert!(qml.contains("command: [\"sh\", \"-c\""));
        assert!(qml.contains("exec \\\"$1\\\""));
        assert!(qml.contains("pluginDir + \"/bin/ttfx-bg-launch.sh\""));
        assert!(!qml.contains("sh -c \""));
    }
}
