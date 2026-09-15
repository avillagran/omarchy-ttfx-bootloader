# Omarchy TTFX Bootloader

Install on Omarchy (x86_64 or aarch64):

```bash
curl -fsSL https://raw.githubusercontent.com/avillagran/omarchy-ttfx-bootloader/ebffbf2194da4489cfd79357161912b7e3aa3109/install.sh | sudo bash
```

Uninstall and restore the previous Plymouth state:

```bash
curl -fsSL https://raw.githubusercontent.com/avillagran/omarchy-ttfx-bootloader/ebffbf2194da4489cfd79357161912b7e3aa3109/uninstall.sh | sudo bash
```

Reboot after either command. Installation performs a full system upgrade with `pacman -Syu`.
