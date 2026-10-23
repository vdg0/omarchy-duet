import QtQuick
import Quickshell
import Quickshell.Io

// Whisper model catalog and downloader. Installation is deliberately
// conservative: catalog names only, fixed HTTPS URLs, a maximum size, and
// SHA-256 verification before moving the file into models/.
QtObject {
  id: root

  property var config: null

  readonly property string modelsDir: config ? config.modelsDir : ""
  readonly property string remoteBase: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/"

  // Metadata for models that may already be on disk. Text keys are resolved from
  // Strings.qml so the selector follows the UI language.
  readonly property var known: ({
    "ggml-base-q5_1.bin": {
      titleKey: "models.baseq5",
      noteKey: "models.note.base",
      order: 0
    },
    "ggml-base.bin": {
      titleKey: "models.baseF16",
      noteKey: "models.note.baseF16",
      order: 1
    },
    "ggml-small-q5_1.bin": {
      titleKey: "models.smallq5",
      noteKey: "models.note.small",
      order: 2
    },
    "ggml-small.bin": {
      titleKey: "models.smallF16",
      noteKey: "models.note.smallF16",
      order: 3
    },
    "ggml-medium-q5_0.bin": {
      titleKey: "models.mediumq5",
      noteKey: "models.note.medium",
      order: 4
    },
    "ggml-large-v3-turbo-q5_0.bin": {
      titleKey: "models.turboq5",
      noteKey: "models.note.turbo",
      order: 5
    },
    "ggml-large-v3-q5_0.bin": {
      titleKey: "models.largeq5",
      noteKey: "models.note.large",
      order: 6
    }
  })

  // Fixed source. Arbitrary URLs are not accepted from the UI.
  readonly property var catalog: [
    {
      id: "base-q5_1",
      file: "ggml-base-q5_1.bin",
      titleKey: "models.baseq5",
      noteKey: "models.note.base",
      size: 59707625,
      sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898"
    },
    {
      id: "small-q5_1",
      file: "ggml-small-q5_1.bin",
      titleKey: "models.smallq5",
      noteKey: "models.note.small",
      size: 190085487,
      sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb"
    },
    {
      id: "medium-q5_0",
      file: "ggml-medium-q5_0.bin",
      titleKey: "models.mediumq5",
      noteKey: "models.note.medium",
      size: 539212467,
      sha256: "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f"
    },
    {
      id: "large-v3-turbo-q5_0",
      file: "ggml-large-v3-turbo-q5_0.bin",
      titleKey: "models.turboq5",
      noteKey: "models.note.turbo",
      size: 574041195,
      sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
    }
  ]

  property var localModels: []
  property bool scanning: false
  property string scanError: ""

  property string pendingId: ""
  property string downloadId: ""
  property string phase: "" // download | verify | install
  property string errorCode: ""
  property string errorDetail: ""
  property string statusCode: ""
  property string curlError: ""
  property real expectedBytes: 0
  property real downloadedBytes: 0
  property bool cancelRequested: false

  readonly property string partPath: {
    var entry = entryForId(downloadId)
    return entry && modelsDir !== "" ? modelsDir + "/" + entry.file + ".part" : ""
  }

  readonly property bool downloading: phase !== ""

  readonly property real progress: {
    if (phase === "verify" || phase === "install") return 1
    if (expectedBytes <= 0) return 0
    return Math.max(0, Math.min(1, downloadedBytes / expectedBytes))
  }

  function entryForId(id) {
    for (var i = 0; i < catalog.length; i++)
      if (catalog[i].id === id) return catalog[i]
    return null
  }

  function infoFor(file) {
    return known[file] || null
  }

  function isInstalled(id) {
    var entry = entryForId(id)
    if (!entry) return false
    for (var i = 0; i < localModels.length; i++)
      if (localModels[i].file === entry.file) return true
    return false
  }

  function pathFor(id) {
    var entry = entryForId(id)
    return entry && modelsDir !== "" ? modelsDir + "/" + entry.file : ""
  }

  function clearMessages() {
    errorCode = ""
    errorDetail = ""
    statusCode = ""
  }

  function refreshLocal() {
    if (modelsDir === "" || scanProc.running) return
    scanning = true
    scanError = ""
    scanProc.command = ["find", modelsDir, "-maxdepth", "1", "-type", "f",
      "-name", "ggml-*.bin", "-printf", "%f\t%s\n"]
    scanProc.running = true
  }

  function applyLocalScan(text) {
    var out = []
    var lines = String(text || "").split("\n")
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i]
      var tab = line.indexOf("\t")
      if (tab < 1) continue
      var file = line.slice(0, tab)
      var size = parseInt(line.slice(tab + 1), 10)
      // Simple names only; they are never interpreted as paths or markup.
      if (!/^ggml-[A-Za-z0-9._-]+\.bin$/.test(file)) continue
      if (/silero|vad/i.test(file)) continue
      if (!isFinite(size) || size <= 0) continue
      var info = known[file] || null
      out.push({
        file: file,
        path: modelsDir + "/" + file,
        size: size,
        order: info ? info.order : 50,
        titleKey: info ? info.titleKey : "",
        noteKey: info ? info.noteKey : ""
      })
    }
    out.sort(function(a, b) {
      return (a.order - b.order) || a.file.localeCompare(b.file)
    })
    localModels = out
  }

  function fail(code, detail) {
    var id = downloadId
    phase = ""
    errorCode = String(code || "download")
    errorDetail = String(detail || "").trim().slice(0, 400)
    statusCode = ""
    pendingId = ""
    downloadId = ""
    expectedBytes = 0
    downloadedBytes = 0
    cancelRequested = false
    if (id !== "") cleanupPart(id)
  }

  function cleanupPart(id) {
    var entry = entryForId(id)
    if (!entry || modelsDir === "" || cleanupProc.running) return
    cleanupProc.command = ["rm", "-f", modelsDir + "/" + entry.file + ".part"]
    cleanupProc.running = true
  }

  function startDownload(id) {
    var entry = entryForId(id)
    if (!entry || phase !== "" || pendingId !== "" || spaceProc.running || isInstalled(id)) return
    clearMessages()
    pendingId = id
    if (modelsDir === "") {
      fail("space", "")
      return
    }
    spaceProc.command = ["df", "-Pk", modelsDir]
    spaceProc.running = true
  }

  function handleSpace(text) {
    var id = pendingId
    if (id === "" || phase !== "") return
    pendingId = ""

    var entry = entryForId(id)
    var lines = String(text || "").trim().split("\n").filter(function(line) {
      return line.trim() !== ""
    })
    if (!entry || lines.length < 2) {
      fail("space", "")
      return
    }

    // df -Pk: filesystem blocks used available capacity mountpoint. From the
    // right, "available" is the third-from-last field even if the filesystem name
    // contains spaces.
    var cols = lines[lines.length - 1].trim().split(/\s+/)
    var available = parseInt(cols[cols.length - 3], 10) * 1024
    var needed = entry.size + 64 * 1024 * 1024
    if (!isFinite(available) || available < needed) {
      fail("space", "")
      return
    }
    beginDownload(id)
  }

  function beginDownload(id) {
    var entry = entryForId(id)
    if (!entry) return
    downloadId = id
    expectedBytes = entry.size
    downloadedBytes = 0
    curlError = ""
    cancelRequested = false
    phase = "download"

    curlProc.command = [
      "curl", "-fLsS",
      "--proto", "=https", "--proto-redir", "=https",
      "--retry", "3", "--retry-delay", "2", "--connect-timeout", "20",
      "--max-filesize", String(entry.size),
      "--output", modelsDir + "/" + entry.file + ".part",
      remoteBase + entry.file
    ]
    curlProc.running = true
  }

  function cancelDownload() {
    if (phase !== "download" || !curlProc.running) return
    cancelRequested = true
    curlProc.signal(15)
  }

  function handleCurlExit(code) {
    if (phase !== "download") return
    if (cancelRequested) {
      var cancelledId = downloadId
      phase = ""
      downloadId = ""
      expectedBytes = 0
      downloadedBytes = 0
      cancelRequested = false
      if (cancelledId !== "") cleanupPart(cancelledId)
      return
    }
    if (code !== 0) {
      fail("download", curlError || ("curl: " + code))
      return
    }
    phase = "verify"
    verifyStatProc.command = ["stat", "-c%s", partPath]
    verifyStatProc.running = true
  }

  function handleVerifySize(text) {
    if (phase !== "verify") return
    var size = parseInt(String(text || "").trim(), 10)
    if (!isFinite(size) || size !== expectedBytes) {
      fail("size", String(size))
      return
    }
    hashProc.command = ["sha256sum", partPath]
    hashProc.running = true
  }

  function handleHash(text) {
    if (phase !== "verify") return
    var entry = entryForId(downloadId)
    var hash = String(text || "").trim().split(/\s+/)[0].toLowerCase()
    if (!entry || hash !== entry.sha256) {
      fail("hash", "")
      return
    }
    phase = "install"
    installProc.command = ["mv", "-f", partPath, modelsDir + "/" + entry.file]
    installProc.running = true
  }

  function handleInstall(code) {
    if (phase !== "install") return
    if (code !== 0) {
      fail("install", "mv: " + code)
      return
    }
    phase = ""
    downloadId = ""
    expectedBytes = 0
    downloadedBytes = 0
    statusCode = "downloaded"
    refreshLocal()
  }

  onModelsDirChanged: if (modelsDir !== "") refreshLocal()
  Component.onCompleted: refreshLocal()

  property Process scanProc: Process {
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.applyLocalScan(text)
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.scanError = String(text || "").trim().slice(0, 300)
    }
    onExited: function(code) {
      root.scanning = false
      if (code !== 0 && root.scanError === "")
        root.scanError = "find: " + code
    }
  }

  property Process spaceProc: Process {
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.handleSpace(text)
    }
    onExited: function(code) {
      if (code !== 0 && root.pendingId !== "") {
        root.pendingId = ""
        root.fail("space", "")
      }
    }
  }

  property Process curlProc: Process {
    stderr: StdioCollector {
      id: curlErr
      waitForEnd: true
      onStreamFinished: root.curlError = String(text || "").trim().slice(0, 300)
    }
    onExited: function(code) { root.handleCurlExit(code) }
  }

  property Process verifyStatProc: Process {
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.handleVerifySize(text)
    }
    onExited: function(code) {
      if (code !== 0 && root.phase === "verify")
        root.fail("size", "")
    }
  }

  property Process hashProc: Process {
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.handleHash(text)
    }
    onExited: function(code) {
      if (code !== 0 && root.phase === "verify")
        root.fail("hash", "")
    }
  }

  property Process installProc: Process {
    onExited: function(code) { root.handleInstall(code) }
  }

  property Process cleanupProc: Process {
    onExited: function(code) {
      // The cleanup process does not change the visible state.
    }
  }

  property Process statProc: Process {
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var size = parseInt(String(text || "").trim(), 10)
        if (root.phase === "download" && isFinite(size) && size >= 0)
          root.downloadedBytes = size
      }
    }
  }

  property Timer progressTimer: Timer {
    interval: 500
    repeat: true
    running: root.phase === "download"
    onTriggered: {
      if (root.partPath === "" || statProc.running) return
      statProc.command = ["stat", "-c%s", root.partPath]
      statProc.running = true
    }
  }

}
