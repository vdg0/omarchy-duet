import QtQuick
import Quickshell
import Quickshell.Io

// Duet preferences. This is a regular component (not a singleton) so the
// path-loaded plugin does not depend on a qmldir: BarWidget instantiates it and
// passes it to the panel.
QtObject {
  id: root

  readonly property string home: Quickshell.env("HOME") || ""
  readonly property string xdgConfigHome: Quickshell.env("XDG_CONFIG_HOME") || (home + "/.config")
  readonly property string xdgDataHome: Quickshell.env("XDG_DATA_HOME") || (home + "/.local/share")
  readonly property string xdgStateHome: Quickshell.env("XDG_STATE_HOME") || (home + "/.local/state")
  readonly property string runtimeDir: xdgDataHome + "/duet"
  readonly property string configDir: xdgConfigHome + "/duet"
  readonly property string path: configDir + "/config.json"
  readonly property string socketPath: (Quickshell.env("XDG_RUNTIME_DIR") || "/tmp") + "/duet.sock"
  readonly property string binaryPath: runtimeDir + "/bin/duet"
  readonly property string togglePath: runtimeDir + "/bin/duet-toggle"
  readonly property string statusPath: runtimeDir + "/bin/duet-status"
  readonly property string logPath: xdgStateHome + "/duet.log"
  readonly property string modelsDir: runtimeDir + "/models"

  property string mode: "meet" // meet | live
  property string uiLang: "en" // es | en (interface language)
  property string micDevice: "Built-in"
  property string speakerDevice: "Built-in"
  property string modelPath: modelsDir + "/ggml-base-q5_1.bin"
  property string srcLang: "es"
  property string dstLang: "en"
  property int minSpeechMs: 300

  property bool loaded: false
  property bool applying: false

  function scheduleSave() {
    if (root.loaded && !root.applying) saveTimer.restart()
  }

  function apply(raw) {
    var v = ({})
    try { v = JSON.parse(String(raw || "{}")) || ({}) } catch (e) { v = ({}) }
    root.applying = true
    root.mode = v.mode === "live" ? "live" : "meet"
    if (v.ui_lang === "en" || v.ui_lang === "es") root.uiLang = v.ui_lang
    if (typeof v.mic === "string" && v.mic.length > 0) root.micDevice = v.mic
    if (typeof v.speaker === "string" && v.speaker.length > 0) root.speakerDevice = v.speaker
    if (typeof v.model === "string" && v.model.length > 0) root.modelPath = v.model
    if (v.src_lang === "es" || v.src_lang === "en") root.srcLang = v.src_lang
    if (v.dst_lang === "es" || v.dst_lang === "en") root.dstLang = v.dst_lang
    var ms = parseInt(String(v.min_speech_ms), 10)
    if (isFinite(ms) && ms >= 100 && ms <= 2000) root.minSpeechMs = ms
    root.applying = false
    root.loaded = true
  }

  function save() {
    if (!root.loaded) return
    var data = {
      mode: root.mode,
      ui_lang: root.uiLang,
      mic: root.micDevice,
      speaker: root.speakerDevice,
      model: root.modelPath,
      src_lang: root.srcLang,
      dst_lang: root.dstLang,
      min_speech_ms: root.minSpeechMs
    }
    file.setText(JSON.stringify(data, null, 2) + "\n")
  }

  onModeChanged: scheduleSave()
  onUiLangChanged: scheduleSave()
  onMicDeviceChanged: scheduleSave()
  onSpeakerDeviceChanged: scheduleSave()
  onModelPathChanged: scheduleSave()
  onSrcLangChanged: scheduleSave()
  onDstLangChanged: scheduleSave()
  onMinSpeechMsChanged: scheduleSave()

  property Timer saveTimer: Timer {
    interval: 250
    repeat: false
    onTriggered: root.save()
  }

  property Process mkdirProcess: Process {
    command: ["mkdir", "-p", root.configDir]
    running: true
  }

  property FileView file: FileView {
    path: root.path
    watchChanges: true
    printErrors: false
    onLoaded: root.apply(text())
    onLoadFailed: {
      // First run: write a configuration file with the defaults.
      root.loaded = true
      root.save()
    }
    onFileChanged: reload()
  }
}
