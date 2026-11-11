# Candidate patches

These patches are kept separate from the installer. They are not applied by the one-liner and are intended for later upstream Omarchy or Aquamarine review.

- `limine-quiet-wallpaper.patch` (Limine 12.9.4 and 12.9.6): adds the
  `quiet_wallpaper: <URI>` key. With `quiet: yes` and a timeout, Limine shows
  only that image (the boot theme's mark and "Press any key for boot options")
  until the timeout ends; a key press reveals the normal menu over `wallpaper`.
  Without the key Limine behaves as stock. Proposed as an Omarchy build of the
  `limine` package in omacom/omarchy-pkgs. The installer writes the key, which
  a stock Limine ignores.
