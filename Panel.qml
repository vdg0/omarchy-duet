import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import qs.Commons
import qs.Commons as Commons
import qs.Ui as Ui

// Duet panel. Loaded from BarWidget.qml with a Loader and receives
// `bar`, `anchorItem`, `service`, and `config` injected, so the widget keeps
// the quick click and the panel is a KeyboardPanel anchored to the bar.
Ui.Panel {
  id: root
  moduleName: "io.github.vdg0.duet"
  ipcTarget: "io.github.vdg0.duet"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  property var service: null
  property var config: null
  property var strings: null
  property var models: null
  property string tab: "settings" // settings | activity
  property bool showAdvanced: false
  property bool showDownloads: false

  readonly property bool ready: service !== null && config !== null && config.loaded === true && strings !== null
  readonly property var turn: service ? service.lastTurn : null
  readonly property bool hasTurn: turn !== null && turn !== undefined
  readonly property string fontFamily: Style.font.family
  readonly property color foreground: Commons.Color.popups.text
  readonly property color muted: Qt.darker(foreground, 1.4)
  readonly property color statusColor: {
    if (!service) return muted
    if (service.state === "running") return "#a6e3a1"
    if (service.busy) return "#f9e2af"
    return "#f38ba8"
  }
  readonly property string statusLabel: {
    if (!service) return t("status.loading")
    if (service.state === "running") return t("status.running")
    if (service.state === "starting") return t("status.starting")
    if (service.state === "stopping") return t("status.stopping")
    return t("status.stopped")
  }
  readonly property bool locked: service !== null && (service.running || service.busy)
  readonly property var langOptions: [
    { value: "es", label: t("lang.es") },
    { value: "en", label: t("lang.en") }
  ]
  readonly property var modeOptions: [
    { value: "meet", label: t("mode.meet") },
    { value: "live", label: t("mode.live") }
  ]
  readonly property var sensitivityOptions: [
    { value: "200", label: t("sensitivity.200") },
    { value: "300", label: t("sensitivity.300") },
    { value: "500", label: t("sensitivity.500") },
    { value: "800", label: t("sensitivity.800") }
  ]
  readonly property var modelOptions: {
    var out = []
    var list = models ? (models.localModels || []) : []
    for (var i = 0; i < list.length; i++)
      out.push({ value: list[i].path, label: modelLabel(list[i]) })

    var current = config ? config.modelPath : ""
    if (current !== "") {
      var found = false
      for (var j = 0; j < out.length; j++)
        if (out[j].value === current) { found = true; break }
      if (!found) out.unshift({ value: current, label: t("models.custom") })
    }
    return out
  }
  readonly property var downloadEntry: models && models.downloadId !== ""
    ? models.entryForId(models.downloadId) : null
  readonly property real downloadProgress: models
    ? (models.phase === "download" ? models.progress : 1) : 0
  property var micOptions: []
  property var outputOptions: []
  readonly property string logText: {
    if (!service) return t("log.noService")
    var lines = service.fullLog || []
    return lines.length > 0 ? lines.join("\n") : t("log.empty")
  }

  function t(key) {
    return root.strings ? root.strings.t(key) : key
  }

  function deviceOptions(kind, current) {
    var out = []
    var list = service && service.devices && service.devices[kind] ? service.devices[kind] : []
    for (var i = 0; i < list.length; i++) {
      var d = list[i] || ({})
      var name = String(d.name || "")
      if (name === "") continue
      out.push({
        value: name,
        label: String(d.description || name) + (d.default ? t("devices.defaultSuffix") : "")
      })
    }
    // The saved value may be "Built-in" or a node that no longer exists. Show it
    // anyway so the selector does not go blank.
    if (current && current !== "") {
      var found = false
      for (var j = 0; j < out.length; j++) if (out[j].value === current) { found = true; break }
      if (!found) out.unshift({
        value: current,
        label: current === "Built-in" ? t("devices.default") : current
      })
    }
    return out
  }

  function rebuildDeviceOptions() {
    micOptions = deviceOptions("inputs", config ? config.micDevice : "")
    outputOptions = deviceOptions("outputs", config ? config.speakerDevice : "")
  }

  function swapLanguages() {
    if (!config) return
    var src = config.srcLang
    config.srcLang = config.dstLang
    config.dstLang = src
  }

  function timeLabel(ts) {
    if (!ts) return ""
    return Qt.formatTime(new Date(Number(ts)), "HH:mm:ss")
  }

  function turnLabel() {
    if (!hasTurn) return ""
    var dir = turn.dir === "peer_to_user" ? t("turn.meetToUser") : t("turn.userToMeet")
    return dir + " · " + String(turn.src_lang || "").toUpperCase()
      + " -> " + String(turn.dst_lang || "").toUpperCase()
      + (turn.ts ? " · " + timeLabel(turn.ts) : "")
  }

  function formatSize(bytes) {
    var n = Number(bytes) || 0
    if (n >= 1024 * 1024 * 1024) return (n / (1024 * 1024 * 1024)).toFixed(1) + " GB"
    if (n >= 1024 * 1024) return Math.round(n / (1024 * 1024)) + " MB"
    if (n >= 1024) return Math.round(n / 1024) + " KB"
    return n + " B"
  }

  function modelLabel(model) {
    var info = models ? models.infoFor(model.file) : null
    var title = info && info.titleKey ? t(info.titleKey) : String(model.file || "")
    var size = model.size > 0 ? " · " + formatSize(model.size) : ""
    var note = info && info.noteKey ? " · " + t(info.noteKey) : ""
    return title + size + note
  }

  function modelPhaseText() {
    if (!models) return ""
    if (models.phase === "verify") return t("models.verifying")
    if (models.phase === "install") return t("models.installing")
    return t("models.downloading")
  }

  function modelErrorText() {
    if (!models || models.errorCode === "") return ""
    var key = "models.error." + models.errorCode
    var text = t(key)
    return text === key ? t("models.error.download") : text
  }

  function catalogButtonText(entry) {
    if (!models || !entry) return ""
    if (models.downloadId === entry.id && models.phase !== "") return modelPhaseText()
    if (models.isInstalled(entry.id)) {
      if (config && config.modelPath === models.pathFor(entry.id)) return t("models.inUse")
      return t("models.use")
    }
    return t("models.download")
  }

  function catalogButtonEnabled(entry) {
    if (!models || !entry || models.phase !== "") return false
    if (!models.isInstalled(entry.id)) return true
    if (root.locked) return false
    return !(config && config.modelPath === models.pathFor(entry.id))
  }

  function handleCatalogAction(entry) {
    if (!models || !entry) return
    if (models.isInstalled(entry.id)) {
      if (!root.locked && config) config.modelPath = models.pathFor(entry.id)
      return
    }
    models.startDownload(entry.id)
  }

  onOpenedChanged: {
    if (models) {
      models.refreshLocal()
      if (opened && models.downloading) {
        showAdvanced = true
        showDownloads = true
      }
    }
    if (!service) return
    service.panelOpen = opened
    if (opened) {
      service.refresh()
      service.refreshFileLog()
      tab = service.running ? "activity" : "settings"
    }
  }
  onServiceChanged: {
    rebuildDeviceOptions()
    if (service) service.panelOpen = opened
  }
  onConfigChanged: rebuildDeviceOptions()
  onStringsChanged: rebuildDeviceOptions()
  Component.onCompleted: rebuildDeviceOptions()
  Component.onDestruction: if (service) service.panelOpen = false

  Connections {
    target: root.strings
    function onLangChanged() { root.rebuildDeviceOptions() }
  }

  Connections {
    target: root.service
    function onDevicesChanged() { root.rebuildDeviceOptions() }
    function onTurnSerialChanged() {
      if (root.opened) root.tab = "activity"
    }
    function onStateChanged() {
      if (root.opened && root.service.state === "running") root.tab = "activity"
    }
  }

  Ui.KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: panel.loadedContent
    contentWidth: panel.fittedContentWidth(
      Style.space(root.tab === "activity" ? 820 : 430),
      Style.space(root.tab === "activity" ? 1000 : 520))
    contentHeight: panel.fittedContentHeight(
      Style.space(root.tab === "activity" ? 740 : 700),
      Style.space(820))
    content: Component {
      Item {
        id: contentRoot
        implicitWidth: Style.space(410)
        implicitHeight: Style.space(700)
        focus: true
        Keys.onEscapePressed: root.close()

        ScrollView {
          id: scroll
          anchors.fill: parent
          clip: true
          ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

          ColumnLayout {
            width: scroll.availableWidth
            height: Math.max(scroll.availableHeight, implicitHeight)
            spacing: Style.spacing.panelGap

            // -- header -------------------------------------------------
            RowLayout {
              Layout.fillWidth: true
              spacing: Style.spacing.lg

              Rectangle {
                Layout.alignment: Qt.AlignVCenter
                width: 10
                height: 10
                radius: 5
                color: root.statusColor
              }

              ColumnLayout {
                Layout.fillWidth: true
                spacing: 1

                Text {
                  text: root.t("panel.title")
                  color: root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.title
                  font.bold: true
                }
                Text {
                  text: root.statusLabel
                  color: root.statusColor
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                }
              }

              RowLayout {
                Layout.alignment: Qt.AlignVCenter
                spacing: Style.spacing.xs

                Ui.Button {
                  Layout.preferredWidth: Style.space(36)
                  text: "ES"
                  selected: root.config ? root.config.uiLang === "es" : true
                  bordered: true
                  focusable: true
                  fontSize: Style.font.caption
                  horizontalPadding: Style.spacing.sm
                  tooltipText: root.t("ui.toSpanish")
                  onClicked: if (root.config) root.config.uiLang = "es"
                }

                Ui.Button {
                  Layout.preferredWidth: Style.space(36)
                  text: "EN"
                  selected: root.config ? root.config.uiLang === "en" : false
                  bordered: true
                  focusable: true
                  fontSize: Style.font.caption
                  horizontalPadding: Style.spacing.sm
                  tooltipText: root.t("ui.toEnglish")
                  onClicked: if (root.config) root.config.uiLang = "en"
                }
              }

              Ui.ToggleSwitch {
                id: powerSwitch
                Layout.alignment: Qt.AlignVCenter
                checked: root.service ? root.service.running : false
                busy: root.service ? root.service.busy : true
                enabled: root.ready
                onToggled: if (root.ready && root.service) root.service.toggle()
              }

              Ui.PanelActionButton {
                Layout.alignment: Qt.AlignTop
                iconText: "󰅖"
                tooltipText: root.t("action.close")
                onClicked: root.close()
              }
            }

            RowLayout {
              Layout.fillWidth: true
              spacing: Style.spacing.md

              Ui.Button {
                Layout.fillWidth: true
                text: root.service && root.service.running ? root.t("action.stop")
                    : root.service && root.service.busy ? root.t("action.wait")
                    : root.t("action.start")
                iconText: root.service && root.service.running ? "󰓛" : "󰐊"
                selected: root.service ? root.service.running : false
                focusable: true
                enabled: root.ready && !(root.service && root.service.busy)
                onClicked: if (root.service) root.service.toggle()
              }

              Ui.Button {
                text: root.t("action.refresh")
                iconText: "󰑐"
                bordered: true
                focusable: true
                enabled: root.ready
                onClicked: if (root.service) root.service.refresh()
              }
            }

            Text {
              Layout.fillWidth: true
              visible: root.service && root.service.lastError !== ""
              text: root.service ? root.service.lastError : ""
              color: Commons.Color.urgent
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              wrapMode: Text.Wrap
              textFormat: Text.PlainText
            }

            // -- tabs ---------------------------------------------------
            RowLayout {
              Layout.fillWidth: true
              spacing: Style.spacing.md

              Ui.Button {
                Layout.fillWidth: true
                text: root.t("tab.settings")
                iconText: "󰒓"
                selected: root.tab === "settings"
                focusable: true
                onClicked: root.tab = "settings"
              }

              Ui.Button {
                Layout.fillWidth: true
                text: root.t("tab.activity") + (root.hasTurn && root.tab !== "activity" ? "  •" : "")
                iconText: "󰋼"
                selected: root.tab === "activity"
                focusable: true
                onClicked: root.tab = "activity"
              }
            }

            // -- settings -----------------------------------------------
            ColumnLayout {
              Layout.fillWidth: true
              visible: root.tab === "settings"
              spacing: Style.spacing.panelGap

              Ui.PanelSeparator { Layout.fillWidth: true }

              Ui.PanelSectionHeader { text: root.t("mode.header"); Layout.fillWidth: true }

              Ui.Dropdown {
                id: modeDrop
                Layout.fillWidth: true
                label: root.t("mode.label")
                options: root.modeOptions
                value: root.config ? root.config.mode : "meet"
                enabled: root.ready && !root.locked
                onChanged: function(value) { if (root.config) root.config.mode = value }
              }

              Text {
                Layout.fillWidth: true
                visible: root.locked
                text: root.t("mode.locked")
                color: root.muted
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                wrapMode: Text.Wrap
              }

              Ui.PanelSeparator { Layout.fillWidth: true }
              Ui.PanelSectionHeader { text: root.t("languages.header"); Layout.fillWidth: true }

              RowLayout {
                Layout.fillWidth: true
                spacing: Style.spacing.md

                Ui.Dropdown {
                  id: srcDrop
                  Layout.fillWidth: true
                  label: root.t("lang.you")
                  options: root.langOptions
                  value: root.config ? root.config.srcLang : "es"
                  enabled: root.ready && !root.locked
                  onChanged: function(value) { if (root.config) root.config.srcLang = value }
                }

                Ui.Button {
                  Layout.alignment: Qt.AlignBottom
                  Layout.preferredWidth: Style.space(36)
                  text: "<->"
                  tooltipText: root.t("lang.swap")
                  bordered: true
                  focusable: true
                  enabled: root.ready && !root.locked
                  onClicked: root.swapLanguages()
                }

                Ui.Dropdown {
                  id: dstDrop
                  Layout.fillWidth: true
                  label: root.t("lang.other")
                  options: root.langOptions
                  value: root.config ? root.config.dstLang : "en"
                  enabled: root.ready && !root.locked
                  onChanged: function(value) { if (root.config) root.config.dstLang = value }
                }
              }

              Ui.PanelSeparator { Layout.fillWidth: true }

              RowLayout {
                Layout.fillWidth: true
                Ui.PanelSectionHeader { text: root.t("devices.header"); Layout.fillWidth: true }
                Ui.PanelActionButton {
                  iconText: "󰑐"
                  tooltipText: root.t("devices.refresh")
                  enabled: root.ready
                  onClicked: if (root.service) root.service.listDevices()
                }
              }

              Ui.Dropdown {
                id: micDrop
                Layout.fillWidth: true
                label: root.t("devices.mic")
                options: root.micOptions
                value: root.config ? root.config.micDevice : ""
                enabled: root.ready && !root.locked
                onChanged: function(value) { if (root.config) root.config.micDevice = value }
              }

              Ui.Dropdown {
                id: spkDrop
                Layout.fillWidth: true
                label: root.t("devices.speaker")
                options: root.outputOptions
                value: root.config ? root.config.speakerDevice : ""
                enabled: root.ready && !root.locked
                onChanged: function(value) { if (root.config) root.config.speakerDevice = value }
              }

              Text {
                Layout.fillWidth: true
                text: root.config && root.config.mode === "live"
                  ? root.t("devices.hint.live")
                  : root.t("devices.hint.meet")
                color: root.muted
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                wrapMode: Text.Wrap
              }

              Ui.Dropdown {
                id: sensitivityDrop
                Layout.fillWidth: true
                label: root.t("sensitivity.label")
                options: root.sensitivityOptions
                value: root.config ? String(root.config.minSpeechMs) : "300"
                enabled: root.ready && !root.locked
                onChanged: function(value) { if (root.config) root.config.minSpeechMs = parseInt(value, 10) }
              }

              Ui.Button {
                Layout.fillWidth: true
                text: root.showAdvanced ? root.t("advanced.hide") : root.t("advanced.show")
                iconText: root.showAdvanced ? "󰅃" : "󰅀"
                bordered: true
                focusable: true
                onClicked: root.showAdvanced = !root.showAdvanced
              }

              ColumnLayout {
                Layout.fillWidth: true
                spacing: Style.spacing.md
                visible: root.showAdvanced

                Ui.PanelSectionHeader {
                  Layout.fillWidth: true
                  text: root.t("models.header")
                }

                Ui.Dropdown {
                  id: modelDrop
                  Layout.fillWidth: true
                  label: root.t("models.local")
                  options: root.modelOptions
                  value: root.config ? root.config.modelPath : ""
                  enabled: root.ready && !root.locked
                  onChanged: function(value) {
                    if (root.config) root.config.modelPath = value
                  }
                }

                Text {
                  Layout.fillWidth: true
                  text: root.t("models.applyHint")
                  color: root.muted
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  wrapMode: Text.Wrap
                }

                Ui.Button {
                  Layout.fillWidth: true
                  visible: root.models !== null
                  text: root.showDownloads ? root.t("models.hideDownloads") : root.t("models.downloads")
                  iconText: root.showDownloads ? "󰅃" : "󰇚"
                  bordered: true
                  focusable: true
                  onClicked: root.showDownloads = !root.showDownloads
                }

                ColumnLayout {
                  Layout.fillWidth: true
                  spacing: Style.spacing.md
                  visible: root.showDownloads && root.models !== null

                  Text {
                    Layout.fillWidth: true
                    text: root.t("models.downloadHint")
                    color: root.muted
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    wrapMode: Text.Wrap
                  }

                  Repeater {
                    model: root.models ? root.models.catalog : []

                    delegate: RowLayout {
                      Layout.fillWidth: true
                      spacing: Style.spacing.md

                      ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 1

                        Text {
                          Layout.fillWidth: true
                          text: root.t(modelData.titleKey)
                          color: root.foreground
                          font.family: root.fontFamily
                          font.pixelSize: Style.font.body
                          font.bold: true
                          wrapMode: Text.Wrap
                        }

                        Text {
                          Layout.fillWidth: true
                          text: root.formatSize(modelData.size) + " · " + root.t(modelData.noteKey)
                          color: root.muted
                          font.family: root.fontFamily
                          font.pixelSize: Style.font.caption
                          wrapMode: Text.Wrap
                          textFormat: Text.PlainText
                        }
                      }

                      Ui.Button {
                        Layout.alignment: Qt.AlignVCenter
                        text: root.catalogButtonText(modelData)
                        bordered: true
                        focusable: true
                        fontSize: Style.font.caption
                        horizontalPadding: Style.spacing.md
                        enabled: root.catalogButtonEnabled(modelData)
                        opacity: enabled ? 1.0 : 0.55
                        onClicked: root.handleCatalogAction(modelData)
                      }
                    }
                  }

                  ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Style.spacing.sm
                    visible: root.models && root.models.phase !== ""

                    Text {
                      Layout.fillWidth: true
                      text: root.downloadEntry ? root.t(root.downloadEntry.titleKey) : ""
                      color: root.foreground
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.body
                      font.bold: true
                      wrapMode: Text.Wrap
                    }

                    Text {
                      Layout.fillWidth: true
                      text: root.modelPhaseText()
                      color: root.muted
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.caption
                      wrapMode: Text.Wrap
                    }

                    Rectangle {
                      Layout.fillWidth: true
                      implicitHeight: Style.space(6)
                      radius: height / 2
                      color: Qt.rgba(1, 1, 1, 0.12)
                      clip: true

                      Rectangle {
                        width: parent.width * root.downloadProgress
                        height: parent.height
                        radius: parent.radius
                        color: Commons.Color.accent
                      }
                    }

                    RowLayout {
                      Layout.fillWidth: true
                      spacing: Style.spacing.md

                      Text {
                        Layout.fillWidth: true
                        text: root.formatSize(root.models ? root.models.downloadedBytes : 0)
                          + " / " + root.formatSize(root.models ? root.models.expectedBytes : 0)
                        color: root.muted
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.caption
                      }

                      Ui.Button {
                        visible: root.models && root.models.phase === "download"
                        text: root.t("models.cancel")
                        bordered: true
                        focusable: true
                        fontSize: Style.font.caption
                        horizontalPadding: Style.spacing.md
                        onClicked: if (root.models) root.models.cancelDownload()
                      }
                    }
                  }

                  ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    visible: root.models && root.models.errorCode !== ""

                    Text {
                      Layout.fillWidth: true
                      text: root.modelErrorText()
                      color: Commons.Color.urgent
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.caption
                      wrapMode: Text.Wrap
                    }

                    Text {
                      Layout.fillWidth: true
                      visible: root.models && root.models.errorDetail !== ""
                      text: root.models ? root.models.errorDetail : ""
                      color: root.muted
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.caption
                      wrapMode: Text.Wrap
                      textFormat: Text.PlainText
                    }
                  }

                  Text {
                    Layout.fillWidth: true
                    visible: root.models && root.models.statusCode === "downloaded"
                      && root.models.phase === "" && root.models.errorCode === ""
                    text: root.t("models.info.downloaded")
                    color: "#a6e3a1"
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    wrapMode: Text.Wrap
                  }
                }

                Ui.PanelSeparator { Layout.fillWidth: true }

                Text {
                  Layout.fillWidth: true
                  text: root.t("models.custom")
                  color: root.muted
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  font.bold: true
                }

                Ui.TextField {
                  Layout.fillWidth: true
                  text: root.config ? root.config.modelPath : ""
                  enabled: root.ready && !root.locked
                  onEditingFinished: if (root.config) root.config.modelPath = text
                }

                Text {
                  Layout.fillWidth: true
                  text: root.t("models.customHint")
                  color: root.muted
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  wrapMode: Text.Wrap
                }
              }

              Text {
                Layout.fillWidth: true
                text: root.t("privacy")
                color: root.muted
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                wrapMode: Text.Wrap
              }
            }

            // -- activity -----------------------------------------------
            ColumnLayout {
              Layout.fillWidth: true
              Layout.fillHeight: true
              visible: root.tab === "activity"
              spacing: Style.spacing.panelGap

              Ui.PanelSeparator { Layout.fillWidth: true }
              Ui.PanelSectionHeader { text: root.t("activity.last"); Layout.fillWidth: true }

              Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: turnColumn.implicitHeight + Style.spacing.lg * 2
                visible: root.hasTurn
                radius: Style.cornerRadius
                color: Qt.rgba(1, 1, 1, 0.035)
                border.width: 1
                border.color: Qt.rgba(1, 1, 1, 0.10)

                ColumnLayout {
                  id: turnColumn
                  anchors.fill: parent
                  anchors.margins: Style.spacing.lg
                  spacing: Style.spacing.sm

                  Text {
                    Layout.fillWidth: true
                    text: root.turnLabel()
                    color: root.muted
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    font.bold: true
                    textFormat: Text.PlainText
                  }
                  Text {
                    Layout.fillWidth: true
                    text: root.hasTurn ? String(root.turn.text_src || "") : ""
                    color: root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.body
                    wrapMode: Text.Wrap
                    maximumLineCount: 4
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                  }
                  Text {
                    Layout.fillWidth: true
                    text: root.hasTurn ? String(root.turn.text_dst || "") : ""
                    color: "#a6e3a1"
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.body
                    wrapMode: Text.Wrap
                    maximumLineCount: 4
                    elide: Text.ElideRight
                    textFormat: Text.PlainText
                  }
                }
              }

              Text {
                Layout.fillWidth: true
                visible: !root.hasTurn
                text: root.t("activity.empty")
                color: root.muted
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                wrapMode: Text.Wrap
              }

              Ui.PanelSeparator { Layout.fillWidth: true }

              RowLayout {
                Layout.fillWidth: true
                Ui.PanelSectionHeader { text: root.t("activity.header"); Layout.fillWidth: true }
                Ui.PanelActionButton {
                  iconText: "󰏌"
                  tooltipText: root.t("activity.openLog")
                  focusable: true
                  enabled: root.ready && root.service && root.service.logPath !== ""
                  onClicked: {
                    if (!root.service || !root.service.logPath) return
                    Quickshell.execDetached(["omarchy-launch-terminal",
                      "tail", "-n", "+1", "-F", root.service.logPath])
                  }
                }
                Ui.PanelActionButton {
                  iconText: "󰑐"
                  tooltipText: root.t("activity.refresh")
                  focusable: true
                  enabled: root.ready
                  onClicked: {
                    if (!root.service) return
                    root.service.refreshFileLog()
                    root.service.requestStatus()
                  }
                }
              }

              Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.minimumHeight: Style.space(220)
                radius: Style.cornerRadius
                color: Qt.rgba(0, 0, 0, 0.22)
                border.width: 1
                border.color: Qt.rgba(1, 1, 1, 0.08)

                ScrollView {
                  anchors.fill: parent
                  anchors.margins: Style.spacing.sm
                  clip: true
                  ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

                  TextArea {
                    id: logArea
                    text: root.logText
                    readOnly: true
                    selectByMouse: true
                    wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                    textFormat: TextEdit.PlainText
                    color: root.muted
                    font.family: "monospace"
                    font.pixelSize: Style.font.caption
                    leftPadding: 0
                    rightPadding: 0
                    topPadding: 0
                    bottomPadding: 0
                    background: null
                  }
                }
              }
            }

            Item { Layout.preferredHeight: Style.spacing.sm }
          }
        }

      }
    }
  }
}
