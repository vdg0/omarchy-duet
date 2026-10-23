import QtQuick
import Quickshell
import qs.Commons
import qs.Ui

// The widget keeps the quick click (on/off) and adds the panel:
// right-click or `omarchy-shell shell summon io.github.vdg0.duet` opens it.
BarWidget {
  id: root
  moduleName: "io.github.vdg0.duet"

  property bool panelPending: false
  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  readonly property string state: service.state
  readonly property bool busy: service.busy

  Config { id: config }
  Strings { id: strings; lang: config.uiLang }
  Models { id: models; config: config }
  Service { id: service; config: config }

  function injectPanel() {
    var target = panelLoader.item
    if (!target) return
    target.bar = root.bar
    target.settings = root.settings
    target.anchorItem = button
    target.hostWidget = root
    target.service = service
    target.config = config
    target.strings = strings
    target.models = models
    if (root.panelPending) {
      root.panelPending = false
      target.open()
    }
  }

  function open() {
    service.refresh()
    if (panelLoader.item) panelLoader.item.open()
    else panelPending = true
  }

  function close() {
    if (panelLoader.item) panelLoader.item.close()
    panelPending = false
  }

  function toggle() {
    if (root.opened) root.close()
    else root.open()
  }

  onBarChanged: injectPanel()
  onSettingsChanged: injectPanel()
  onBusyChanged: if (!busy) dot.opacity = 1.0

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: Qt.callLater(root.injectPanel)
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "DU"
    active: root.opened
    tooltipText: {
      var key = root.state === "running" ? "widget.live"
        : root.state === "starting" ? "widget.starting"
        : root.state === "stopping" ? "widget.stopping"
        : "widget.off"
      return strings.t(key) + strings.t("widget.hint")
    }

    onPressed: function(b) {
      if (b === Qt.RightButton) root.open()
      else if (b === Qt.MiddleButton) service.refresh()
      else if (b === Qt.LeftButton) service.toggle()
    }

    // Status dot over the "R": red when off, blinking amber during a
    // transition, green only when the pipeline is ready.
    Rectangle {
      id: dot
      width: 5
      height: 5
      radius: 2.5
      color: root.state === "running" ? "#a6e3a1"
           : root.busy ? "#f9e2af"
           : "#f38ba8"
      anchors.right: parent.right
      anchors.rightMargin: -1
      anchors.top: parent.top
      anchors.topMargin: 1

      SequentialAnimation on opacity {
        running: root.busy
        loops: Animation.Infinite
        NumberAnimation { from: 1.0; to: 0.25; duration: 350; easing.type: Easing.InOutQuad }
        NumberAnimation { from: 0.25; to: 1.0; duration: 350; easing.type: Easing.InOutQuad }
      }
    }
  }
}
