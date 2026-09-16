# Native Plymouth embedded TTFX tracer

This standalone module integrates the real Rust TTFX static engine into the Plymouth 26.134.222 pixel-plugin seam. Its native default is the exact persisted selector `Enabled=true`, `Mode=fixed`, `Effect=decrypt`, `Seed=22321466108495961`, and `PlaybackMode=submit-to-finish`, matching `ttfx --frame-rate 240 decrypt` with the established deterministic seed. `submit-to-finish` plays once and holds its terminal frame; if successful unlock is confirmed before natural completion, it fast-forwards once to that frame. `continuous` preserves full start-to-finish looping. It uses generated, checked-in bytes of `../../logo.txt` on an 81×10 canvas, configures the engine for 240 fps, schedules Plymouth at 1/240 second, and advances exactly one engine frame per event-loop timeout. Every display draws the same immutable structured-cell snapshot. Run `./generate-embedded-logo.py` after changing the canonical logo; tests and builds use `--check` so stale embedded bytes fail.

The draw handler paints the configured loader background, an always-available static Omarchy mark, TTFX foreground/background cell colors, and Plymouth label/entry widgets. The loader's `BackgroundColor` is also passed into the embedded TTFX terminal, so effects that sample or fade toward the terminal background stay seamless when the Omarchy theme changes instead of rendering a mismatched rectangle. Spaces and hidden cells do not paint foreground. For password prompts, the plugin loads `lock.png` once from `ImageDir`, scales it once to 80% of the 48-pixel entry height (34×38 for the original 84×96 asset), composites it 15 pixels left of the centered entry, and leaves bullet rendering to Plymouth's `entry.png`/`bullet.png` widget. Missing or unusable lock assets fall back to the built-in vector marker without failing the splash. Password prompt labels and ordinary messages are not drawn while password mode is active; question prompt text remains available, and retained messages can reappear after password mode ends. Wrong-password reaction and candidate windows remain approximately 700 ms and 5 seconds at 240 Hz (168 and 1200 ticks), and each horizontal offset is held for 24 ticks (100 ms); only X changes, while the entry Y coordinate remains fixed. The interface intentionally omits `set_keyboard`, `unset_keyboard`, and `display_prompt`; the module registers no keyboard callback and never receives the plaintext LUKS passphrase.

Engine creation and mutable calls stay on the Plymouth event-loop thread. A pre-activation create or first-snapshot failure rejects `show_splash_screen` for Plymouth fallback. A post-activation step, snapshot, or label failure clears the borrowed snapshot, frees the engine once, stops animation, and establishes persistent static-degraded mode. After hide, show can bind a different event loop, restores retained prompt/message state and password bullets, and schedules no animation retry. A normal hide/show preserves a healthy engine and resumes its timeout deterministically; destroy releases it once.

Run local sanitizer-backed lifecycle, state, real-engine activation, and contract tests with `./tests/run.sh`. The Rust bridge intentionally retains the broad upstream effect registry rather than specializing it to `decrypt`: native effect selection is expected to permit every validated effect, while this tracer chooses `decrypt`.

## Reproducible package build and installation

`bin/omarchy-plymouth-ttfx-build` is the production package-build interface. It accepts only Plymouth ABI `26.134.222` (a distribution package-release suffix such as `-2` is allowed), independently gates both `ply-splash-core` and `ply-splash-graphics`, builds the vendored Rust tree with `cargo build --locked --offline --profile plymouth`, links against the builder's Plymouth headers and libraries, strips the result, and rejects the artifact unless RELRO/NOW, the single plugin export, no leaked engine exports, no debug sections, and a fully resolved runtime dependency set are present. Publication to the requested output uses a unique temporary in the destination directory and an atomic rename.

A package recipe can invoke the interfaces exactly as follows on an Arch build host with the matching Plymouth development files and Rust toolchain:

```bash
build() {
  "$srcdir/omarchy/bin/omarchy-plymouth-ttfx-build" build --output "$srcdir/ttfx-plymouth.so"
}

package() {
  "$srcdir/omarchy/bin/omarchy-plymouth-ttfx-build" stage --module "$srcdir/ttfx-plymouth.so" --destdir "$pkgdir"
  install -Dm755 "$srcdir/omarchy/bin/omarchy-plymouth-ttfx-install" "$pkgdir/usr/bin/omarchy-plymouth-ttfx-install"
}
```

The staged package contains `/usr/lib/plymouth/ttfx-plymouth.so`, the native `omarchy` descriptor/theme/state, and the complete `omarchy-static` script recovery theme. End-user systems do not need Cargo or Rust. For a transactional manual or package-script activation, invoke `omarchy-plymouth-ttfx-install --module /path/to/root-owned/ttfx-plymouth.so` before allowing a package manager to replace the live module. It serializes on `/run/lock/omarchy-plymouth.lock`, activates static recovery first, atomically publishes the mode-0644 root-owned module and descriptor, rebuilds the UKI, verifies the module, theme, and `ldd` dependency closure through `lsinitcpio`, and restores the previous module, descriptor, recovery theme, active theme, and UKI before rebuilding the restored state on failure. The package repository's PKGBUILD and install-hook wiring live outside this repository and remain a separate integration step.

`./build-on-guest.sh` remains a development convenience for exact-header validation against the lab guest; it is not the package interface and does not install or activate anything.

## Frozen framebuffer handoff

The phase handoff protocol is version 2. It publishes `playback=hold-final` for
Input Only and `playback=continuous` for Full Animated. A compatible desktop
bridge must retain a hold-final snapshot without advancing or looping the TTFX
engine; this prevents a second animation cycle while the compositor reaches the
desktop. Normal playback advances two engine steps per 240 Hz tick (2x).

At `become_idle`, the optional `/run/omarchy-plymouth-handoff.raw` publisher uses the same trusted root-owned tmpfs directory fd as the phase exporter. It atomically renames a newly created regular file to mode 0644. Initialization, invalid publication, and phase-publication failure remove stale raw state. No runtime path or environment override is supported.

The 48-byte little-endian header matches `BootSnapshot`: `OMBFRAW1`, width/height/packed stride at offsets 8/12/16, XRGB8888 `0x34325258` at 20, payload length at 24, capture `CLOCK_BOOTTIME` nanoseconds at 32, and zero reserved at 40. Pixels are packed BGRX with zero X bytes. The timestamp belongs to the draw, not the later publication; captures older than 30 seconds decline.

The plugin copies actual Plymouth ARGB pixelbuffer data during a complete draw callback, immediately after painting the background and TTFX grid but before painting any lock, entry, bullets, question, or message. This animation-only base is tied to the last drawn cells/phase rather than pending engine state, so a retained password box does not prevent safe RAW handoff and its pixels cannot enter the snapshot. Raw publication additionally requires that exact phase and exactly one upright scale-1 output no larger than 4096×2160. Partial draws invalidate raw eligibility until another complete base draw. Error-reaction draws never replace the clean cache because their red/offset composition is not represented by phase metadata; an older clean cache is published only if its phase still matches at idle. The cache owns its pixels (at most 35,389,440 bytes); each eligible draw performs one bounded copy, and idle publication uses 16 KiB scratch and at most 2160 row writes without sleeps, retries, or disk synchronization. Unsupported rotation, scaling, multiple outputs, stale captures, and allocation/I/O errors decline raw export rather than fabricate pixels. Plymouth provides no scanout acknowledgment: the guarantee is the last fully drawn pixelbuffer, not a confirmed physical KMS presentation.

`tests/run.sh` covers freeze/phase ownership, pre-overlay prompt and message isolation with sentinel pixels, reaction-phase safety, unsupported displays, and failed atomic writes. `build-on-guest.sh` additionally paints a real 1280×720 Plymouth pixelbuffer and verifies every exported pixel and header field by file readback before building the module. These are non-root tests in temporary directories; live compositor takeover still needs the separate QEMU proof.

## Verdict: PACKAGING COMPLETE, BOOT E2E PENDING

The real embedded-engine code path, lifecycle/error behavior, exact-header build, hardening, ABI-gated staging, static recovery, transactional activation/rollback, and simulated stock-mkinitcpio inclusion contract are covered here. No deployment or activation was performed. QEMU boot/runtime E2E, graphical password/unlock behavior during LUKS, and live pixels on the boot path remain deliberately unclaimed. Cross-repository PKGBUILD/install-hook provisioning is also not claimed.
