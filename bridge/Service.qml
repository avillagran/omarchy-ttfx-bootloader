import Quickshell
import QtQuick
import Quickshell.Io

// Audio-reactive desktop background — plugin launcher.
//
// This plugin does NOT render anything itself. It starts the prebuilt Rust
// binary (bin/ttfx-bg-launch.sh -> bin/ttfx-bg-rs-<arch>), which opens one
// gtk4-layer-shell window per monitor and draws the effect inside an embedded
// Vte terminal. The binary owns the layer-shell surface; the plugin only keeps
// it alive for the lifetime of the shell.
//
// No build step: the binaries are committed, so `omarchy plugin add` is enough.
Item {
  id: root
  // Plugin directory: resolve relative to this QML file. `Quickshell.pluginDir`
  // is NOT available in the `service` kind context (it was `undefined`), so we
  // use Qt.resolvedUrl(".") like BarWidget/Panel do.
  readonly property string pluginDir: Qt.resolvedUrl(".").toString().replace(/^file:\/\//, "").replace(/\/$/, "")

  Process {
    id: bgProc
    running: true
    // The early boot helper creates this marker before Quickshell starts. Pass
    // the launcher as an argv value rather than interpolating it into shell
    // source, so spaces or shell metacharacters in pluginDir cannot execute.
    command: ["sh", "-c", "test ! -e \"${XDG_RUNTIME_DIR}/omarchy-plymouth-handoff.active\" && exec \"$1\"", "sh", pluginDir + "/bin/ttfx-bg-launch.sh"]
    // Inherits WAYLAND_DISPLAY / HYPRLAND_INSTANCE_SIGNATURE from the session.
  }
}
