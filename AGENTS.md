# AGENTS.md

Omarchy TTFX Bootloader: native TTFX animation for Omarchy's Plymouth boot and
LUKS unlock screen, installed on Omarchy 4.5 (x86_64 and aarch64) by a one-liner.

## Layout

- `native/plymouth-ttfx-engine/` — Rust staticlib TTFX engine with C FFI
  (`include/ttfx_plymouth.h`) and determinism tests (`tests/engine.rs`, `tests/ffi.rs`).
  Vendors TTFX 0.5.1 plus a downstream patch in `vendor/`; `UPSTREAM.md` and
  `tests/vendor-provenance.py` record and check the import.
- `native/plymouth-ttfx-plugin/` — Plymouth C plugin (`plugin.c`) plus contract
  and lifecycle tests under `tests/` (run via `tests/run.sh`).
- `bridge/` — the 2026-09 desktop bridge (`ttfx-bg-rs`) for omarchy-audio-background,
  with its own older engine copy in `bridge/ttfx-src/`. The installer no longer
  builds or installs it: that plugin ships its own renderer.
- `bin/` — runtime helpers installed under `/usr/share/omarchy/bin` (Omarchy Menu
  invokes that copy directly).
- `scripts/` — real installer/uninstaller used by the one-liner bootstrap;
  `common.sh` holds the managed-file list and hash-guarded restore,
  `menu-extension.py` edits the user's menu extension inside its markers.
- `default/` — pristine copies of system files the installer overrides.
- `tests/` — rootless verification of the installer and Omarchy shell integration.
- `patches/` — candidate patches for upstream review; never applied by the installer.

## Build and install

- `bin/omarchy-plymouth-ttfx-build` builds the engine (cargo, offline,
  `--profile plymouth`) and compiles the plugin with gcc.
- `scripts/install.sh` installs packages, builds the module, activates the
  `omarchy` Plymouth theme, rebuilds UKI/initramfs, verifies the boot image, and
  records the hash of every file it wrote for `scripts/uninstall.sh`.
- One-liner: `curl -fsSL .../main/install.sh | bash`; the root bootstrap pins an
  immutable commit archive and verifies its sha256 before installing. By default
  it installs the prebuilt module pinned in `prebuilt/manifest` (GitHub release
  asset plus sha256); `bash -s -- --build` compiles on the machine instead.
- `scripts/build-prebuilt.sh` builds the release modules for both architectures
  in rootless Arch containers (aarch64 through qemu-user) from a clean commit.

## Test

Run `bash tests/verify.sh` on the target architecture before committing native
changes. The individual checks include:

    bash tests/installer-static.sh
    bash tests/kbd-order-test.sh
    bash tests/menu-extension-test.sh
    bash tests/shell.d/plymouth-effect-switcher-test.sh
    bash tests/shell.d/plymouth-native-install-test.sh
    bash tests/shell.d/plymouth-native-package-test.sh
    native/plymouth-ttfx-plugin/tests/run.sh
    (cd native/plymouth-ttfx-engine && cargo test --locked --offline)
    python3 native/plymouth-ttfx-engine/tests/vendor-provenance.py

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
- The sha256 values in `prebuilt/manifest`, except when publishing modules built
  by `scripts/build-prebuilt.sh`: download each release asset and verify it
  matches before committing. Never commit the `.so` files themselves.
- `native/plymouth-ttfx-plugin/tests/contract.sh` invariants: no simulation
  advance from `on_draw`; required FFI symbols stay literal.
- `native/` and the runtime helpers in `bin/` are synced from the Omarchy branch
  `feature/ttfx-plymouth-quattro`; change them there first. The deliberate
  difference: `bin/omarchy-plymouth-ttfx-install` here turns Off and corrupt
  selections into Decrypt, and the effect switcher keeps a literal prompt
  because stock Omarchy has no translation key for it, and
  `bin/omarchy-plymouth-effect-list` falls back to `/usr/share/omarchy` when
  `OMARCHY_PATH` is unset (the installer runs it outside a session).
- Never write into the user's home as root: menu edits run through `runuser`.
- No credentials, build artifacts, or machine-specific lab files in commits.

## Test safety

- Keep build output and retained fixtures on disk under `${TMPDIR:-/var/tmp}`.
- Do not add recursive fixture cleanup; retain directories for inspection.
- Symlink fixtures must use targets inside their own temporary directory.
- Rootless staging verifies the module without activating it on the host.
