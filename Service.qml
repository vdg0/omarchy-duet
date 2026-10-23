import QtQuick
import Quickshell
import Quickshell.Io

// Bridge between the panel and the daemon.
//
// - `duet-status` is the source of truth for the state (it works even when
//   the daemon is stopped and there is no socket).
// - The JSON-lines socket provides turns, logs, and the device list while the
//   daemon is running.
// - Start/stop goes through `duet-toggle`, because the socket only exists
//   while the daemon is alive.
Item {
  id: root

  visible: false
  width: 0
  height: 0

  property var config: null

  property string state: "stopped" // stopped | starting | running | stopping
  property bool socketConnected: false
  property var lastTurn: null
  property var logLines: []
  property var fileLogLines: []
  property var devices: ({ inputs: [], outputs: [] })
  property string lastError: ""
  property bool panelOpen: false
  // Incremented only by new socket turns, not when restoring the last turn
  // from `status`; this keeps the panel from jumping tabs when it opens.
  property int turnSerial: 0

  // After a click, do not let polling overwrite the optimistic transition.
  property double pendingUntil: 0

  readonly property bool running: state === "running"
  readonly property bool busy: state === "starting" || state === "stopping"
  readonly property string socketPath: config ? config.socketPath : ""
  readonly property string statusPath: config ? config.statusPath : ""
  readonly property string togglePath: config ? config.togglePath : ""
  readonly property string binaryPath: config ? config.binaryPath : ""
  readonly property string logPath: config ? config.logPath : ""

  // While the socket is alive, its events are freshest; the daemon's log file
  // is the fallback when Duet is stopped.
  readonly property var fullLog: logLines.length > 0 ? logLines : fileLogLines

  function pushLog(line) {
    var text = String(line || "")
    if (text.length === 0) return
    logLines = logLines.concat([text]).slice(-300)
  }

  function refreshStatus() {
    if (statusPath === "") return
    if (!statusProc.running) statusProc.running = true
  }

  function applyProcessState(raw) {
    var s = String(raw || "").trim()
    if (s !== "running" && s !== "starting" && s !== "stopping") s = "stopped"
    if (Date.now() < pendingUntil) {
      statusTimer.restart()
      return
    }
    if (s !== state) {
      state = s
      if (s === "running" || s === "starting") ensureSocket()
    }
    if (s === "stopped") {
      if (socket && socket.connected) socket.connected = false
      socketConnected = false
    }
    statusTimer.restart()
  }

  function ensureSocket() {
    if (socketConnected || socketPath === "") return
    // A failed attempt leaves the internal QLocalSocket in a state that cannot
    // be recovered with connected=true; create a new object instead.
    var previous = socket
    socket = socketFactory.createObject(root)
    if (previous) previous.destroy()
  }

  function send(obj) {
    if (!socketConnected || !socket || !socket.connected) return false
    socket.write(JSON.stringify(obj) + "\n")
    socket.flush()
    return true
  }

  function requestStatus() {
    send({ cmd: "status" })
  }

  function listDevices() {
    if (send({ cmd: "list_devices" })) return
    if (binaryPath === "") return
    if (!devicesProc.running) {
      devicesProc.command = [binaryPath, "--devices-json"]
      devicesProc.running = true
    }
  }

  function refresh() {
    refreshStatus()
    listDevices()
    requestStatus()
  }

  function start() {
    if (!config || state === "running" || busy) return
    var mic = config.micDevice && config.micDevice.length > 0 ? config.micDevice : "Built-in"
    var speaker = config.speakerDevice && config.speakerDevice.length > 0 ? config.speakerDevice : "Built-in"
    startProc.command = [
      config.togglePath, "start",
      "--mode", config.mode,
      "--mic", mic,
      "--speaker", speaker,
      "--model", config.modelPath,
      "--from", config.srcLang,
      "--to", config.dstLang,
      "--min-speech", String(config.minSpeechMs)
    ]
    startProc.running = true
    state = "starting"
    pendingUntil = Date.now() + 5000
    lastError = ""
    lastTurn = null
    logLines = []
    pushLog("Starting in " + (config.mode === "live" ? "live" : "Meet") + " mode...")
  }

  function stop() {
    if (!config || state === "stopped") return
    state = "stopping"
    pendingUntil = Date.now() + 4000
    pushLog("Stopping...")
    if (!send({ cmd: "stop" })) {
      // Without a socket (for example, while the daemon is still loading
      // models), the script performs the fallback shutdown.
      stopFallbackTimer.restart()
    } else {
      stopFallbackTimer.restart()
    }
  }

  function toggle() {
    if (state === "running") stop()
    else if (state === "stopped") start()
    else if (state === "starting") stop()
  }

  function applyDevices(raw) {
    var parsed = null
    try { parsed = JSON.parse(String(raw || "")) } catch (e) { parsed = null }
    if (!parsed) {
      pushLog("Could not read the device list")
      return
    }
    root.devices = {
      inputs: Array.isArray(parsed.inputs) ? parsed.inputs : [],
      outputs: Array.isArray(parsed.outputs) ? parsed.outputs : []
    }
  }

  function applySocketState(value) {
    var s = String(value || "")
    if (s === "idle") s = "stopped"
    if (s === "error") s = "stopped"
    if (s !== "starting" && s !== "running" && s !== "stopping" && s !== "stopped") return
    // Polling wins; an initial "idle" must not turn off a real startup.
    if (s === "stopped" && (state === "starting" || state === "running")) return
    if (s !== state) state = s
  }

  function handleLine(line) {
    var msg = null
    try { msg = JSON.parse(String(line || "")) } catch (e) { return }
    if (!msg || !msg.type) return
    if (msg.type === "hello") return
    if (msg.type === "status") {
      var p = msg.payload || ({})
      if (Array.isArray(p.log_tail) && p.log_tail.length > 0) logLines = p.log_tail.slice(-300)
      if (p.last_turn) lastTurn = p.last_turn
      applySocketState(p.state)
      return
    }
    if (msg.type === "state") {
      applySocketState(msg.state)
      return
    }
    if (msg.type === "turn") {
      lastTurn = msg
      turnSerial += 1
      return
    }
    if (msg.type === "log") {
      pushLog(msg.line)
      return
    }
    if (msg.type === "devices") {
      devices = {
        inputs: Array.isArray(msg.inputs) ? msg.inputs : [],
        outputs: Array.isArray(msg.outputs) ? msg.outputs : []
      }
      return
    }
    if (msg.type === "error") {
      lastError = String(msg.msg || "error")
      pushLog("ERROR: " + lastError)
      return
    }
    if (msg.type === "ok") {
      if (msg.cmd === "stop") pushLog("Stop accepted by the daemon")
      return
    }
  }

  function refreshFileLog() {
    if (logPath === "" || logProc.running) return
    logProc.command = ["tail", "-n", "120", logPath]
    logProc.running = true
  }

  // Retry the socket while the process is alive. The status poll turns this
  // timer on and off, so no cycles are wasted while the daemon is stopped.
  property var socket: null
  property Component socketFactory: Component {
    Socket {
      path: root.socketPath
      connected: true
      parser: SplitParser { onRead: line => root.handleLine(line) }
      onConnectedChanged: {
        root.socketConnected = connected
        if (connected) {
          root.requestStatus()
          root.listDevices()
        }
      }
      onError: root.socketConnected = false
    }
  }

  Timer {
    id: reconnectTimer
    interval: 1000
    repeat: true
    running: (state === "running" || state === "starting") && !socketConnected
    onTriggered: root.ensureSocket()
  }

  Timer {
    id: statusTimer
    interval: 1000
    repeat: false
    onTriggered: root.refreshStatus()
  }

  Timer {
    id: stopFallbackTimer
    interval: 5000
    repeat: false
    onTriggered: {
      if (root.state === "stopped") return
      if (!stopProc.running) {
        stopProc.command = [root.togglePath, "stop"]
        stopProc.running = true
      }
    }
  }

  Timer {
    id: fileLogTimer
    interval: 2000
    repeat: true
    running: root.panelOpen
    onTriggered: root.refreshFileLog()
  }

  Process {
    id: statusProc
    command: [root.statusPath]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.applyProcessState(text)
    }
    onExited: statusTimer.restart()
  }

  Process {
    id: startProc
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var msg = String(text || "").trim()
        if (msg.length > 0) root.lastError = msg
      }
    }
    onExited: function(code, status) {
      if (code !== 0 && root.lastError === "")
        root.lastError = "Could not start (code " + code + ")"
      root.refreshStatus()
    }
  }

  Process {
    id: stopProc
    onExited: function(code, status) {
      root.refreshStatus()
    }
  }

  Process {
    id: devicesProc
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.applyDevices(text)
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var msg = String(text || "").trim()
        if (msg.length > 0) root.pushLog("devices: " + msg)
      }
    }
    onExited: function(code, status) {
      if (code !== 0) root.pushLog("devices: command exited with code " + code)
    }
  }

  Process {
    id: logProc
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var lines = String(text || "").split("\n").filter(function(l) { return l.length > 0 })
        root.fileLogLines = lines.slice(-120)
      }
    }
  }

  onConfigChanged: {
    if (statusPath === "") return
    statusProc.running = false
    refreshStatus()
  }

  Component.onCompleted: {
    refreshStatus()
  }
}
