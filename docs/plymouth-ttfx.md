> This reference describes the complete quattro source port. In this standalone repository, use `bash tests/verify.sh`; paths under `test/shell.d` refer to the Omarchy source tree.

# Native TTFX boot animation

This integration is based on the current `quattro` boot/provisioning and shell interfaces. It adds a Plymouth 26 native pixel plugin backed by vendored TTFX 0.5.1, effect/playback helpers, an Appearance > Bootloader submenu beside the existing Unlock picker, and transactional activation with classic-initramfs/UKI verification.

## Safe package defaults

`default/plymouth/omarchy.plymouth` selects `ModuleName=script` and also carries the initial TTFX selector values. The normal script therefore remains usable when a package does not include the native renderer. A package may ship a compiled payload at `/usr/lib/omarchy/plymouth/ttfx-plymouth.so`; target finalization, deferred owner provisioning, and the one-time migration activate it through `omarchy-plymouth-ttfx-install`. They never compile or replace live boot state merely because the source package was unpacked into an ISO.

The installer verifies the exact Plymouth ABI and module exports/hardening, stages static recovery, serializes mutations on `/run/lock/omarchy-plymouth.lock`, rebuilds the active boot artifacts, and rolls back on failure. `omarchy-plymouth-ttfx-build stage` explicitly selects the native renderer only after validating the supplied module. Packaging a runtime payload and publishing it are separate release steps.

Branding changes retain the existing validated effect, seed, colors, and playback mode. They preserve a selected native renderer only while a trusted native module is installed. Selecting an effect without that module fails before the descriptor is changed. The current `quattro` script, including late-display prompt recentering, is retained as the fallback.

## Runtime and handoff

The default is decrypt at 240 Hz with two engine frames per tick. Input Only plays once and holds completion, speeding up when the password is submitted; Full Animated loops. All playback uses `ttfx_engine_draw_logo`. The effect picker omits Off and keeps Random. The Unlock theme picker remains available.

While only a firmware framebuffer drives the panel (on Apple Silicon the device-tree simple-framebuffer that the native display driver replaces a few seconds into boot), the plugin can hold on a static frame: the theme background plus `boot-mark.png` from the theme's `ImageDir`, centred at 128 logical pixels. The animation starts once a native display appears, or after 8 seconds. The `[ttfx]` key `FirmwareHold=true|false|auto` controls it; `auto`, also used when the key is absent or invalid, holds only when the device tree lists a `framebuffer@` node under `/chosen`. The default descriptor omits the key, and reinstalls and branding refreshes rebuild the descriptor from its template without carrying a hand-set value over. `boot-mark.png` is optional and theme-independent: without it the hold shows the background alone, branding changes never recolour or replace it, and the native helper requires it inside the boot image only when the installed theme ships it. This standalone installer ships Omarchy's `icon.png` as `default/plymouth/boot-mark.png`, installs it into both the live theme and `/usr/share/omarchy/default/plymouth`, and restores the original state of both paths on uninstall.

Right before the handover the boot splash plays its effect backwards until only the theme background is left, so the compositor starts on the same colour. `Outro=reverse|none` and `OutroMs` (wall-clock budget, default 700, clamped to 100..1500) control it; descriptors without the keys get the outro with the default budget. The engine keeps a bounded, evenly decimated record of the forward playback (at most 128 keyframes and 4 MiB), and `become_idle` defers the idle trigger until the reverse has reached the blank frame and that frame has been drawn (or 250 ms later), well inside the 3 s `plymouth deactivate` timeout. The raw handoff frame and the console-restore frame are then the plain background, and the phase file is removed. Shutdown, reboot and update splashes, `Outro=none`, a missing record, a firmware hold, and static mode keep the previous idle behaviour; a hide during the outro ends it at once on the plain background.

Plymouth deactivates before quitting, retains its last splash through compositor startup, and has a bounded reap timer. A desktop renderer may opt in with `OMARCHY_TTFX_BG_LAUNCHER`; without one, normal shell startup proceeds immediately. There is no dependency on a personal desktop plugin. The background layer emits a readiness marker once after a settled frame, without idle polling.

## Validation

Run the focused shell tests `test/shell.d/plymouth-*-test.sh` and `test/shell.d/menu-test.sh`, then the native Rust and C suites described in `native/plymouth-ttfx-engine/UPSTREAM.md` and `native/plymouth-ttfx-plugin/README.md`, on the target architecture. Keep fixtures and build output on disk under `/var/tmp`; fixtures are retained and symlink targets stay inside their own fixture directory.

These checks establish module/API, pixelbuffer, selector, theme refresh, rollback, provisioning, and menu contracts. They do not prove visible LUKS input or the physical boot-to-desktop transition. Those require a coordinated installation and reboot or an independent disposable boot environment.
