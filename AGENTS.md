# AGENTS.md

Omarchy TTFX Bootloader: native TTFX animation for Omarchy's Plymouth boot and
LUKS unlock screen, with a desktop handoff bridge to omarchy-audio-background.

## Layout

- `native/plymouth-ttfx-engine/` — Rust cdylib TTFX engine with C FFI
  (`include/ttfx_plymouth.h`) and determinism tests (`tests/engine.rs`, `tests/ffi.rs`).
- `native/plymouth-ttfx-plugin/` — Plymouth C plugin (`plugin.c`) plus contract
  and lifecycle tests under `tests/` (run via `tests/run.sh`).
- `bridge/` — Rust desktop bridge (`ttfx-bg-rs`) that freezes the last boot frame
  and crossfades into the desktop; QML UI files at its root.
- `bridge/ttfx-src/` — vendored engine sources for the bridge; edit in parallel
  with `native/plymouth-ttfx-engine/` (manual sync, never a live path).
- `bin/` — runtime helpers installed under `/usr/share/omarchy/bin` (Omarchy Menu
  invokes that copy directly).
- `scripts/` — real installer/uninstaller used by the one-liner bootstrap.
- `default/` — pristine copies of system files the installer overrides.
- `tests/` — rootless verification of the installer and Omarchy shell integration.
- `patches/` — candidate patches for upstream review; never applied by the installer.

## Build and install

- `bin/omarchy-plymouth-ttfx-build` builds the engine (cargo, offline,
  `--profile plymouth`) and compiles the plugin with gcc.
- `scripts/install.sh` installs packages, builds module and bridge, activates the
  `omarchy` Plymouth theme, rebuilds UKI/initramfs, and verifies the boot image.
- One-liner: `curl -fsSL .../main/install.sh | bash`; the root bootstrap pins an
  immutable commit archive and verifies its sha256 before installing.

## Test

Run all of these before committing native changes:

    bash tests/installer-static.sh
    bash tests/shell.d/plymouth-effect-switcher-test.sh
    bash tests/shell.d/plymouth-native-install-test.sh
    bash tests/shell.d/plymouth-native-package-test.sh
    native/plymouth-ttfx-plugin/tests/run.sh
    (cd native/plymouth-ttfx-engine && cargo test --locked)
    (cd bridge && cargo test --locked)

## Conventions

- Conventional commit messages; keep build output (`bridge/target/`,
  `native/*/target/`) out of commits.
- The boot contract is deterministic: fps=240, 2x speed,
  `TTFX_PLAYBACK_STEPS_PER_TICK=2U`; all logo drawing goes through
  `ttfx_engine_draw_logo` with loop/one_time modes.
- Menu entries keep `Decrypt` as initial/fallback effect, never expose `Off`,
  and never remove `Random`.

## Never touch

- The archive URL and sha256 pin in root `install.sh`, except when re-pinning to
  a newly pushed, tested commit (download the GitHub archive and verify its
  sha256 first).
- `native/plymouth-ttfx-plugin/tests/contract.sh` invariants: no simulation
  advance from `on_draw`; required FFI symbols stay literal.
- `bridge/ttfx-src/` stays vendored: mirror engine changes there manually.
- No credentials, build artifacts, or machine-specific lab files in commits.
