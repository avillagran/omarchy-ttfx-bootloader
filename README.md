# Omarchy TTFX Bootloader

Native TTFX animation for Omarchy's Plymouth boot and LUKS unlock screen, with a seamless handoff to `omarchy-audio-background` when that plugin is installed.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/avillagran/omarchy-ttfx-bootloader/main/install.sh | bash
```

The installer downloads a temporary source archive, installs required Arch packages, builds the Plymouth 26 module and optional desktop bridge locally, activates the `omarchy` theme, rebuilds the UKI/initramfs, and verifies the resulting boot image.

It also adds these entries to `Menu → Style → Bootloader`:

- `Input Only` — default; plays once, accelerates after a valid password, and holds the final frame until the desktop handoff.
- `Full Animated` — legacy continuous loop from boot through handoff.
- `Effect` — selects and persists any supported TTFX effect. `Decrypt` is the initial/fallback effect; `Off` is not exposed.

Animation runs at 2× while preserving the effect and playback selections across reinstall/update runs. Runtime helpers are installed consistently under `/usr/share/omarchy/bin`, `/usr/local/bin`, and `/usr/bin` because Omarchy Menu invokes its `/usr/share/omarchy/bin` copy directly.

Reboot after installation to validate the complete boot path.

## Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/avillagran/omarchy-ttfx-bootloader/main/uninstall.sh | bash
```

Uninstall restores the files, menu extension, previous Plymouth theme, and boot image state captured by the first standalone installation.

## Local verification

```bash
bash tests/installer-static.sh
bash tests/kbd-order-test.sh
bash tests/shell.d/plymouth-effect-switcher-test.sh
bash tests/shell.d/plymouth-native-install-test.sh
bash tests/shell.d/plymouth-native-package-test.sh
native/plymouth-ttfx-plugin/tests/run.sh
(cd native/plymouth-ttfx-engine && cargo test --locked)
(cd bridge && cargo test --locked)
```
