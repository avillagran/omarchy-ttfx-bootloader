import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "io.github.avillagran.omarchy-audio-background"
  ipcTarget: "io.github.avillagran.omarchy-audio-background"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  readonly property var barIdentity: hostWidget || root

  readonly property string stateFile: Quickshell.env("HOME") +
    "/.local/state/omarchy/audio-background/state.json"
  readonly property string writeState: Qt.resolvedUrl("bin/write_state.sh").toString().replace("file://", "")

  property bool running: true
  property bool audio: true
  property bool showFps: false
  property string effect: "matrix"
  property var effects: ["matrix", "rain", "wave", "bars", "donut", "fire", "starfield", "life"]
  property int intensity: 5
  property int introSize: 2
  property bool bootBetween: true
  property int rotateSecs: 20
  property string ttfxText: "OMARCHY"
  property int resolution: 1
  property int reactivity: 2
  property string byline: ""
  property string label: "♪"
  property bool collapsed: false
  property real panelOpacity: 0.6
  property point dragOffset: Qt.point(0, 0)
  property bool introBeatSync: true
  property string charStyle: "native"
  property string bootCharStyle: "native"
  // State uses fifths of normal speed: 5 = 1x, 100 = 20x.
  property int speed: 5

  readonly property var allEffects: ["matrix", "rain", "wave", "bars", "donut", "fire", "starfield", "life"]
  // Vendored ttfx effects (rendered by the ttfx engine). matrix/rain stay hand-rolled
  // (ours are audio-reactive); these are the extra ttfx catalog worth offering.
  readonly property var ttfxEffects: [
    "beams", "binarypath", "blackhole", "bouncyballs", "bubbles", "burn", "colorshift",
    "crumble", "decrypt", "errorcorrect", "expand", "fireworks", "highlight", "laseretch",
    "middleout", "orbittingvolley", "overflow", "pour", "print", "randomsequence", "rings",
    "scattered", "slice", "slide", "smoke", "spotlights", "spray", "swarm", "sweep",
    "synthgrid", "thunderstorm", "unstable", "vhstape", "waves", "wipe"
  ]

  // NOTE: opened / open / close / toggle / closeForPopoutSwitch are provided
  // by the qs.Ui.Panel base type — do NOT redeclare them (QML forbids
  // redefining base properties and the panel then fails to compile).
  function openFromHotkey() { root.open() }

  function refresh() { statusProc.running = true }

  function write(arg) { Quickshell.execDetached(["sh", root.writeState, arg]) }
  // Coupled updates must use one writer process. Two detached read-modify-write
  // calls race: selecting a disabled effect could enable it but restore the old
  // active effect, making the first click appear to do nothing.
  function writeMany(args) { Quickshell.execDetached(["sh", root.writeState].concat(args)) }
  function setRunning(v)   { root.running = v;   write("running="   + (v ? "1" : "0")); }
  function setAudio(v)     { root.audio = v;     write("audio="     + (v ? "1" : "0")); }
  function setShowFps(v)   { root.showFps = v;   write("show_fps="  + (v ? "1" : "0")); }
  function setBootBetween(v) { root.bootBetween = v; write("boot_between=" + (v ? "1" : "0")); }
  function setRotateSecs(v) { root.rotateSecs = v;  write("rotate_secs=" + v); }
  function setResolution(v) { root.resolution = v;  write("resolution=" + v); }
  function setReactivity(v) { root.reactivity = v;  write("reactivity=" + v); }
  function setTtfxText(t)  { root.ttfxText = t;    write("ttfx_text=" + t); }
  function pickEffect(e)   { root.effect = e;
    // Keep the picked effect in the rotation set and select it atomically. This
    // prevents a detached effect+ write from overwriting the new selection.
    if (root.effects.indexOf(e) < 0) {
      root.effects = root.effects.concat([e])
      writeMany(["effect=" + e, "effect+" + e])
    } else {
      write("effect=" + e)
    }
  }
  function setIntensity(v) { root.intensity = v; write("intensity=" + v); }
  function setSpeed(v) { root.speed = Math.max(1, Math.min(100, Math.round(v))); write("speed=" + root.speed); }
  function setIntroSize(v) {
    root.introSize = v
    write("intro_size=" + v)
    introReplay.restart()   // the intro renders once at startup; replay it at the new size (debounced)
  }
  function setIntroBeatSync(v) { root.introBeatSync = v; write("intro_beat_sync=" + (v ? "1" : "0")) }
  function setByline(t)    { root.byline = t;    write("byline="    + t); }
  function setPanelOpacity(v) { root.panelOpacity = v; write("panel_opacity=" + v); }
  // When true, use the active Omarchy theme colors for the effect palettes instead
  // of the built-in hardcoded colors. Requires omarchy-theme-current to be installed.
  property bool useThemeColors: true
  property bool transparentBackground: false
  function setUseThemeColors(v) { root.useThemeColors = v; write("use_theme_colors=" + (v ? "1" : "0")) }
  function setTransparentBackground(v) { root.transparentBackground = v; write("transparent_background=" + (v ? "1" : "0")) }
  function setCharStyle(v) { root.charStyle = v; write("char_style=" + v) }
  function bootCharGlyph() {
    switch (root.bootCharStyle) {
      case "block": return "█"
      case "dark": return "▓"
      case "medium": return "▒"
      case "light": return "░"
      case "hash": return "#"
      case "dot": return "·"
      case "lower_o": return "o"
      case "upper_o": return "O"
      default: return "#"
    }
  }
  function cycleBootCharStyle() {
    var styles = ["native", "block", "dark", "medium", "light", "hash", "dot", "lower_o", "upper_o"]
    var index = styles.indexOf(root.bootCharStyle)
    root.bootCharStyle = styles[(index + 1 + styles.length) % styles.length]
    // This is a boot-only visual setting, so atomically replay the intro as a preview.
    writeMany(["boot_char_style=" + root.bootCharStyle, "restart"])
  }
  function toggleCollapsed() { root.collapsed = !root.collapsed; if (!root.collapsed) root.dragOffset = Qt.point(0,0) }
  // Cycle the active effect across the ENABLED set only (the user's selection).
  // Built-ins + ttfx combined catalog is for the grid UI, NOT for cycling.
  function cycleEffect(dir) {
    var list = root.effects
    if (!list || list.length === 0) return // nothing enabled
    var i = list.indexOf(root.effect)
    if (i < 0) { i = 0; root.pickEffect(list[0]); return }
    i = (i + dir + list.length) % list.length
    root.pickEffect(list[i])
  }
  function toggleEffect(e, on) {
    var list = root.effects.slice()
    var i = list.indexOf(e)
    if (on && i < 0) list.push(e)
    if (!on && i >= 0) list.splice(i, 1)
    if (list.length === 0) return // never empty
    root.effects = list
    // If the currently displayed effect was just disabled, switch to first enabled
    if (!on && root.effect === e) {
      var next = list[0]
      root.effect = next
      writeMany(["effect-" + e, "effect=" + next])
    } else {
      write("effect" + (on ? "+" : "-") + e)
    }
  }

  Component.onCompleted: root.refresh()

  // The intro (boot text) renders once at renderer startup, so a new intro_size only
  // shows on a restart. Debounce a restart until the size slider stops moving.
  Timer {
    id: introReplay
    interval: 700
    repeat: false
    onTriggered: root.write("restart")
  }

  Process {
    id: statusProc
    command: ["cat", root.stateFile]
    running: false
    stdout: StdioCollector {
      onStreamFinished: function() {
        try {
          var s = JSON.parse(text || "{}")
          if (typeof s.running === "boolean") root.running = s.running
          if (typeof s.audio === "boolean")   root.audio = s.audio
          if (typeof s.show_fps === "boolean") root.showFps = s.show_fps
          // Load the persisted enabled set BEFORE validating the active effect.
          // Otherwise a shell/theme reload validates against the 8-item default
          // and silently rewrites a valid ttfx selection back to matrix.
          if (Array.isArray(s.effects) && s.effects.length) root.effects = s.effects
          if (typeof s.effect === "string" && s.effect.trim() !== "") {
            root.effect = s.effect
            if (root.effects.indexOf(root.effect) < 0 && root.effects.length > 0) {
              root.effect = root.effects[0]
              write("effect=" + root.effect)
            }
          } else if (root.effects && root.effects.length > 0 && root.effects.indexOf(root.effect) < 0) {
            root.effect = root.effects[0]
            write("effect=" + root.effect)
          }
          if (typeof s.intensity === "number") root.intensity = s.intensity
          if (typeof s.speed === "number") root.speed = Math.max(1, Math.min(100, s.speed))
          if (typeof s.intro_size === "number") root.introSize = s.intro_size
          if (typeof s.boot_between === "boolean") root.bootBetween = s.boot_between
          if (typeof s.rotate_secs === "number") root.rotateSecs = s.rotate_secs
          if (typeof s.resolution === "number") root.resolution = s.resolution
          if (typeof s.reactivity === "number") root.reactivity = s.reactivity
          if (typeof s.ttfx_text === "string" && s.ttfx_text.trim() !== "") root.ttfxText = s.ttfx_text
          if (typeof s.byline === "string")   root.byline = s.byline
          if (typeof s.panel_opacity === "number") root.panelOpacity = Math.max(0.15, Math.min(1.0, s.panel_opacity))
          if (typeof s.intro_beat_sync === "boolean") root.introBeatSync = s.intro_beat_sync
          if (typeof s.use_theme_colors === "boolean") root.useThemeColors = s.use_theme_colors
          if (typeof s.transparent_background === "boolean") root.transparentBackground = s.transparent_background
          if (typeof s.char_style === "string") root.charStyle = s.char_style
          if (typeof s.boot_char_style === "string") root.bootCharStyle = s.boot_char_style
        } catch (e) {}
      }
    }
  }

  IpcHandler {
    target: root.ipcTarget
    function open(): void  { root.open() }
    function close(): void { root.close() }
    function toggle(): void { root.toggle() }
  }

  onOpenedChanged: { if (root.opened) root.dragOffset = Qt.point(0, 0) }

  CardWindow {
    id: panel
    anchorItem: root.anchorItem
    owner: root.barIdentity
    bar: root.bar
    open: root.opened
    centerOnBar: false  // anchor under the bar widget (like KeyboardPanel's default), not centered
    dragOffset: root.dragOffset
    contentWidth: root.collapsed ? panel.fittedContentWidth(Style.space(420)) : panel.fittedContentWidth(Style.space(560))
    // Keep the card compact and put the long effect catalog in a real scroll view.
    // The old content-sized card exceeded the screen and hid bottom settings.
    contentHeight: root.collapsed ? panel.fittedContentHeight(collapsedRow.implicitHeight) : Math.round(panel.availableCardHeight * 0.86)
    focusTarget: keyCatcher

    // Translucent card so the animated background shows through. Opacity via slider.
    Rectangle {
      id: cardSurface
      anchors.fill: parent
      radius: Style.cornerRadius
      border.color: Color.popups.border
      border.width: 1
      color: Qt.rgba(Color.popups.background.r, Color.popups.background.g, Color.popups.background.b, root.panelOpacity)
    }

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      anchors.margins: panel.padding
      onCloseRequested: root.close()
      onTabRequested: function(d) { if (root.bar) root.bar.switchPanelFrom(root.barIdentity, d) }

      Flickable {
        id: panelFlick
        anchors.fill: parent
        clip: true
        contentWidth: width
        contentHeight: contentColumn.implicitHeight
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        visible: !root.collapsed

        ColumnLayout {
          id: contentColumn
          width: panelFlick.width
          spacing: Style.space(12)

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(8)
          // Drag handle — drag the header to move the panel (resets on reopen)
          Item {
            Layout.fillWidth: true
            height: headerText.implicitHeight
            PanelSectionHeader { id: headerText; text: "AUDIO BACKGROUND"; anchors.verticalCenter: parent.verticalCenter }
            MouseArea {
              anchors.fill: parent
              acceptedButtons: Qt.LeftButton
              cursorShape: Qt.SizeAllCursor
              property point startPos
              property point startOffset
              onPressed: (mouse) => { startPos = Qt.point(mouse.x, mouse.y); startOffset = root.dragOffset }
              onPositionChanged: (mouse) => { if (pressed) root.dragOffset = Qt.point(startOffset.x + mouse.x - startPos.x, startOffset.y + mouse.y - startPos.y) }
            }
          }
          Button { text: "—"; onClicked: root.toggleCollapsed() } // collapse to mini
        }

        // Main toggles in one compact row (short labels so three fit across).
        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(6)
          Text { text: "On"; color: root.barForeground; font.family: root.bar ? root.bar.fontFamily : "sans"; font.pixelSize: Style.font.body }
          // ToggleSwitch is controlled: toggled() passes no value, so flip the real state.
          ToggleSwitch { checked: root.running; onToggled: root.setRunning(!root.running) }
          Text { text: "Audio"; color: root.barForeground; font.family: root.bar ? root.bar.fontFamily : "sans"; font.pixelSize: Style.font.body; Layout.leftMargin: Style.space(8) }
          ToggleSwitch { checked: root.audio; onToggled: root.setAudio(!root.audio) }
          Text { text: "FPS"; color: root.barForeground; font.family: root.bar ? root.bar.fontFamily : "sans"; font.pixelSize: Style.font.body; Layout.leftMargin: Style.space(8) }
          ToggleSwitch { checked: root.showFps; onToggled: root.setShowFps(!root.showFps) }
          Item { Layout.fillWidth: true }
        }

        PanelSectionHeader { text: "BACKGROUNDS" }

        // Prev / next effect across the whole catalog.
        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(8)
          Button { text: "◀"; onClicked: root.cycleEffect(-1) }
          Text {
            text: root.effect
            color: root.barForeground
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
            Layout.fillWidth: true
          }
          Button { text: "▶"; onClicked: root.cycleEffect(1) }
        }

        // Enabled set with per-effect toggle, 2 per row so the whole catalog fits
        // without making the panel too tall; >1 enabled => rotates (see ROTATION).
        GridLayout {
          columns: 3
          Layout.fillWidth: true
          columnSpacing: Style.space(10)
          rowSpacing: Style.space(6)
          Repeater {
            model: root.allEffects
            RowLayout {
              Layout.fillWidth: true
              spacing: Style.space(6)
              Button {
                text: modelData
                selected: root.effect === modelData
                onClicked: root.pickEffect(modelData)
                Layout.fillWidth: true
              }
              ToggleSwitch {
                checked: root.effects.indexOf(modelData) >= 0
                onToggled: root.toggleEffect(modelData, !(root.effects.indexOf(modelData) >= 0))
              }
            }
          }
        }

        PanelSectionHeader {
          visible: root.ttfxEffects.indexOf(root.effect) >= 0
          text: "CARÁCTER · TEXTO TTFX"
        }
        RowLayout {
          visible: root.ttfxEffects.indexOf(root.effect) >= 0
          Layout.fillWidth: true
          spacing: Style.space(6)
          Button { text: "Original"; selected: root.charStyle === "native"; onClicked: root.setCharStyle("native") }
          Button { text: "█"; selected: root.charStyle === "block"; onClicked: root.setCharStyle("block") }
          Button { text: "▓"; selected: root.charStyle === "dark"; onClicked: root.setCharStyle("dark") }
          Button { text: "▒"; selected: root.charStyle === "medium"; onClicked: root.setCharStyle("medium") }
          Button { text: "░"; selected: root.charStyle === "light"; onClicked: root.setCharStyle("light") }
          Button { text: "#"; selected: root.charStyle === "hash"; onClicked: root.setCharStyle("hash") }
          Button { text: "·"; selected: root.charStyle === "dot"; onClicked: root.setCharStyle("dot") }
          Button { text: "o"; selected: root.charStyle === "lower_o"; onClicked: root.setCharStyle("lower_o") }
          Button { text: "O"; selected: root.charStyle === "upper_o"; onClicked: root.setCharStyle("upper_o") }
          Item { Layout.fillWidth: true }
        }

        PanelSectionHeader { text: "TTFX EFFECTS" }

        // Vendored ttfx effects; same pick + rotation-toggle as the built-ins.
        GridLayout {
          columns: 3
          Layout.fillWidth: true
          columnSpacing: Style.space(10)
          rowSpacing: Style.space(6)
          Repeater {
            model: root.ttfxEffects
            RowLayout {
              Layout.fillWidth: true
              spacing: Style.space(6)
              Button {
                text: modelData
                selected: root.effect === modelData
                onClicked: root.pickEffect(modelData)
                Layout.fillWidth: true
              }
              ToggleSwitch {
                checked: root.effects.indexOf(modelData) >= 0
                onToggled: root.toggleEffect(modelData, !(root.effects.indexOf(modelData) >= 0))
              }
            }
          }
        }

        PanelSectionHeader { text: "TTFX TEXT" }
        TextField {
          Layout.fillWidth: true
          // Default text the ttfx engine animates; emptying reverts to OMARCHY.
          text: root.ttfxText
          placeholderText: "OMARCHY"
          onEditingFinished: root.setTtfxText(text.trim() === "" ? "OMARCHY" : text)
        }

        PanelSectionHeader { text: "ROTATION" }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(10)
          Text {
            text: "Boot between backgrounds"
            color: root.barForeground
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
            Layout.fillWidth: true
          }
          ToggleSwitch {
            checked: root.bootBetween
            onToggled: root.setBootBetween(!root.bootBetween)
          }
          Button {
            text: "CHAR " + root.bootCharGlyph()
            onClicked: root.cycleBootCharStyle()
          }
        }

        // Sliders organized 3 per row for a compact layout
        GridLayout {
          columns: 3
          Layout.fillWidth: true
          columnSpacing: Style.space(10)
          rowSpacing: Style.space(6)

          // Row 1: Seconds | Intensity | Speed
          ColumnLayout {
            Layout.fillWidth: true
            PanelSectionHeader { text: "SECONDS  ·  " + root.rotateSecs }
            PanelSlider {
              Layout.fillWidth: true
              bar: root.bar
              minimum: 3; maximum: 120; step: 1; integer: true
              value: root.rotateSecs
              onMoved: function(v) { root.setRotateSecs(v) }
            }
          }
          ColumnLayout {
            Layout.fillWidth: true
            PanelSectionHeader { text: "INTENSITY  ·  " + root.intensity }
            PanelSlider {
              Layout.fillWidth: true
              bar: root.bar
              minimum: 0; maximum: 10; step: 1; integer: true
              value: root.intensity
              onMoved: function(v) { root.setIntensity(v) }
            }
          }
          ColumnLayout {
            Layout.fillWidth: true
            PanelSectionHeader { text: "SPEED  ·  " + (root.speed / 5).toFixed(1) + "x" }
            PanelSlider {
              Layout.fillWidth: true
              bar: root.bar
              minimum: 1; maximum: 100; step: 1; integer: true
              value: root.speed
              onMoved: function(v) { root.setSpeed(v) }
            }
          }

          // Row 2: Resolution | Audio Reactivity | Intro Text Size
          ColumnLayout {
            Layout.fillWidth: true
            PanelSectionHeader { text: "RESOLUTION  ·  " + root.resolution }
            PanelSlider {
              Layout.fillWidth: true
              bar: root.bar
              minimum: 1; maximum: 4; step: 1; integer: true
              value: root.resolution
              onMoved: function(v) { root.setResolution(v) }
            }
          }

          // Row 2: Resolution | Audio Reactivity | Intro Text Size
          ColumnLayout {
            Layout.fillWidth: true
            PanelSectionHeader { text: "AUDIO REACT  ·  " + root.reactivity }
            PanelSlider {
              Layout.fillWidth: true
              bar: root.bar
              minimum: 0; maximum: 5; step: 1; integer: true
              value: root.reactivity
              onMoved: function(v) { root.setReactivity(v) }
            }
          }
          ColumnLayout {
            Layout.fillWidth: true
            PanelSectionHeader { text: "INTRO SIZE  ·  " + root.introSize }
            PanelSlider {
              Layout.fillWidth: true
              bar: root.bar
              minimum: 1; maximum: 3; step: 1; integer: true
              value: root.introSize
              onMoved: function(v) { root.setIntroSize(v) }
            }
          }
        }

        PanelSectionHeader { text: "INTRO BYLINE" }
        TextField {
          Layout.fillWidth: true
          // Default signature; emptying the field reverts to it.
          property string defaultByline: "By x.com/avillagran"
          text: root.byline.trim() === "" ? defaultByline : root.byline
          placeholderText: defaultByline
          onEditingFinished: {
            if (text.trim() === "") text = defaultByline
            // Empty means "use the built-in default" (Rust also defaults to it).
            root.setByline(text === defaultByline ? "" : text)
          }
        }

        PanelSectionHeader { text: "INTRO AL RITMO  ·  " + (root.introBeatSync ? "ON" : "OFF") }
        ToggleSwitch {
          checked: root.introBeatSync
          onToggled: root.setIntroBeatSync(!root.introBeatSync)
        }

        PanelSectionHeader { text: "USAR COLORES DEL TEMA  ·  " + (root.useThemeColors ? "ON" : "OFF") }
        ToggleSwitch {
          checked: root.useThemeColors
          onToggled: root.setUseThemeColors(!root.useThemeColors)
        }

        PanelSectionHeader { text: "FONDO TRANSPARENTE  ·  " + (root.transparentBackground ? "ON" : "OFF") }
        ToggleSwitch {
          checked: root.transparentBackground
          onToggled: root.setTransparentBackground(!root.transparentBackground)
        }

        PanelSectionHeader { text: "PANEL TRANSPARENCY  ·  " + Math.round(root.panelOpacity*100) + "%" }
        PanelSlider {
          Layout.fillWidth: true
          bar: root.bar
          minimum: 15; maximum: 100; step: 5; integer: true
          value: Math.round(root.panelOpacity*100)
          onMoved: function(v) { root.setPanelOpacity(v/100) }
        }
        }
      }

      // The catalog is intentionally longer than the card. Keep a permanent
      // thumb visible while it overflows so scrolling is discoverable.
      Rectangle {
        id: scrollThumb
        visible: !root.collapsed && panelFlick.contentHeight > panelFlick.height + 1
        z: 2
        width: Style.space(3)
        height: Math.max(Style.space(18), panelFlick.height * panelFlick.height / panelFlick.contentHeight)
        x: parent.width - width
        y: panelFlick.y + panelFlick.contentY * (panelFlick.height - height) /
           Math.max(1, panelFlick.contentHeight - panelFlick.height)
        radius: width / 2
        color: Color.accent
        opacity: 0.75
        Behavior on y { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
      }

      RowLayout {
        id: collapsedRow
        width: parent.width
        spacing: Style.space(8)
        visible: root.collapsed
        Button { text: "◀"; onClicked: root.cycleEffect(-1) }
        Item {
          Layout.fillWidth: true
          height: effectText.implicitHeight
          Text {
            id: effectText
            anchors.centerIn: parent
            text: root.effect
            color: root.barForeground
            font.family: root.bar ? root.bar.fontFamily : "sans"
            font.pixelSize: Style.font.body
          }
          MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton
            cursorShape: Qt.SizeAllCursor
            property point startPos
            property point startOffset
            onPressed: (mouse) => { startPos = Qt.point(mouse.x, mouse.y); startOffset = root.dragOffset }
            onPositionChanged: (mouse) => { if (pressed) root.dragOffset = Qt.point(startOffset.x + mouse.x - startPos.x, startOffset.y + mouse.y - startPos.y) }
          }
        }
        Button { text: "▶"; onClicked: root.cycleEffect(1) }
        Button { text: "⛶"; onClicked: root.toggleCollapsed() }
      }
    }
  }
}
