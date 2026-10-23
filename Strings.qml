import QtQuick

// UI strings. The panel's EN/ES button changes `lang`; the other views use
// `t("key")` in their bindings and re-evaluate automatically.
// The Spanish catalog is intentional: the product is bilingual even though
// comments, docs, and diagnostics are in English.
QtObject {
  id: root

  property string lang: "en" // es | en

  readonly property var es: ({
    "panel.title": "Duet ES / EN",
    "status.running": "En vivo · listo",
    "status.starting": "Iniciando…",
    "status.stopping": "Deteniendo…",
    "status.stopped": "Apagado",
    "status.loading": "Cargando…",

    "action.start": "Iniciar Duet",
    "action.stop": "Detener Duet",
    "action.wait": "Esperá…",
    "action.refresh": "Actualizar",
    "action.close": "Cerrar",

    "tab.settings": "Ajustes",
    "tab.activity": "Actividad",

    "mode.header": "MODO",
    "mode.label": "Cómo se usa",
    "mode.meet": "Meet · bidireccional",
    "mode.live": "En vivo · micrófono",
    "mode.locked": "Detené Duet para cambiar el modo, los idiomas o los dispositivos.",

    "languages.header": "IDIOMAS",
    "lang.you": "Vos hablás",
    "lang.other": "La otra persona",
    "lang.es": "Español (ES)",
    "lang.en": "Inglés (EN)",
    "lang.swap": "Invertir idiomas",

    "devices.header": "DISPOSITIVOS",
    "devices.refresh": "Actualizar dispositivos",
    "devices.mic": "Micrófono (tu voz)",
    "devices.speaker": "Salida / auriculares (la traducción)",
    "devices.default": "Predeterminado del sistema",
    "devices.defaultSuffix": "  · predeterminado",
    "devices.hint.meet": "En Meet, elegí «Duet-Mic» como micrófono y «Duet-Meet» como altavoz dentro de la videollamada.",
    "devices.hint.live": "En vivo: Duet escucha tu micrófono y reproduce la traducción en tus auriculares.",

    "sensitivity.label": "Pausa para cerrar el turno",
    "sensitivity.200": "Rápida · 200 ms",
    "sensitivity.300": "Normal · 300 ms",
    "sensitivity.500": "Conservadora · 500 ms",
    "sensitivity.800": "Muy conservadora · 800 ms",

    "advanced.show": "Ajustes avanzados",
    "advanced.hide": "Ocultar ajustes avanzados",
    "privacy": "Duet corre 100 % local. No se envía audio ni texto a ningún servicio.",

    "models.header": "MODELO DE WHISPER",
    "models.local": "Modelo local",
    "models.applyHint": "Se aplica en el próximo inicio. Detené Duet para cambiarlo.",
    "models.custom": "Ruta personalizada",
    "models.customHint": "Ruta absoluta a un .bin fuera de la carpeta de modelos.",
    "models.downloads": "Descargar modelos",
    "models.hideDownloads": "Ocultar descargas",
    "models.downloadHint": "Catálogo fijo desde whisper.cpp. Se verifican tamaño y SHA-256 antes de instalar.",
    "models.download": "Descargar",
    "models.use": "Usar",
    "models.inUse": "En uso",
    "models.downloading": "Descargando",
    "models.verifying": "Verificando integridad",
    "models.installing": "Instalando",
    "models.cancel": "Cancelar",
    "models.info.downloaded": "Modelo descargado y verificado. Ya aparece en la lista.",
    "models.error.space": "No hay espacio suficiente en el disco.",
    "models.error.download": "Falló la descarga.",
    "models.error.size": "El archivo descargado tiene un tamaño inesperado.",
    "models.error.hash": "La verificación SHA-256 no coincide.",
    "models.error.install": "No se pudo instalar el modelo.",

    "models.baseq5": "Base q5_1",
    "models.baseF16": "Base (f16)",
    "models.smallq5": "Small q5_1",
    "models.smallF16": "Small (f16)",
    "models.mediumq5": "Medium q5_0",
    "models.turboq5": "Large v3 turbo q5_0",
    "models.largeq5": "Large v3 q5_0",
    "models.note.base": "Recomendado para CPU",
    "models.note.baseF16": "Referencia local; usa CPU",
    "models.note.small": "Más calidad; usa GPU",
    "models.note.smallF16": "Más calidad; GPU si está disponible",
    "models.note.medium": "Mejor con GPU; lento en CPU",
    "models.note.turbo": "Mejor opción con GPU",
    "models.note.large": "Máxima calidad; requiere GPU y RAM",

    "activity.last": "ÚLTIMO TURNO",
    "activity.header": "ACTIVIDAD",
    "activity.empty": "Todavía no hay turnos. Cuando hables, acá aparece la transcripción y su traducción.",
    "activity.refresh": "Actualizar registro",
    "activity.openLog": "Abrir el log completo en una terminal",

    "turn.userToMeet": "VOS -> MEET",
    "turn.meetToUser": "MEET -> VOS",

    "log.noService": "Sin conexión con Duet.",
    "log.empty": "Sin actividad todavía.",

    "ui.toSpanish": "Cambiar la interfaz a español",
    "ui.toEnglish": "Cambiar la interfaz a inglés",

    "widget.live": "Duet en vivo",
    "widget.starting": "Duet iniciando…",
    "widget.stopping": "Duet deteniendo…",
    "widget.off": "Duet apagado",
    "widget.hint": " — clic para encender/apagar, clic derecho para ajustes"
  })

  readonly property var en: ({
    "panel.title": "Duet ES / EN",
    "status.running": "Live · ready",
    "status.starting": "Starting…",
    "status.stopping": "Stopping…",
    "status.stopped": "Off",
    "status.loading": "Loading…",

    "action.start": "Start Duet",
    "action.stop": "Stop Duet",
    "action.wait": "Please wait…",
    "action.refresh": "Refresh",
    "action.close": "Close",

    "tab.settings": "Settings",
    "tab.activity": "Activity",

    "mode.header": "MODE",
    "mode.label": "How it's used",
    "mode.meet": "Meet · bidirectional",
    "mode.live": "Live · microphone",
    "mode.locked": "Stop Duet to change the mode, languages, or devices.",

    "languages.header": "LANGUAGES",
    "lang.you": "You speak",
    "lang.other": "The other person",
    "lang.es": "Spanish (ES)",
    "lang.en": "English (EN)",
    "lang.swap": "Swap languages",

    "devices.header": "DEVICES",
    "devices.refresh": "Refresh devices",
    "devices.mic": "Microphone (your voice)",
    "devices.speaker": "Output / headphones (translation)",
    "devices.default": "System default",
    "devices.defaultSuffix": "  · default",
    "devices.hint.meet": "In Meet, choose “Duet-Mic” as the microphone and “Duet-Meet” as the speaker inside the call.",
    "devices.hint.live": "Live: Duet listens to your microphone and plays the translation in your headphones.",

    "sensitivity.label": "Turn-end pause",
    "sensitivity.200": "Fast · 200 ms",
    "sensitivity.300": "Normal · 300 ms",
    "sensitivity.500": "Conservative · 500 ms",
    "sensitivity.800": "Very conservative · 800 ms",

    "advanced.show": "Advanced settings",
    "advanced.hide": "Hide advanced settings",
    "privacy": "Duet runs 100% locally. No audio or text is sent to any service.",

    "models.header": "WHISPER MODEL",
    "models.local": "Local model",
    "models.applyHint": "Applied on the next start. Stop Duet to change it.",
    "models.custom": "Custom path",
    "models.customHint": "Absolute path to a .bin outside the models folder.",
    "models.downloads": "Download models",
    "models.hideDownloads": "Hide downloads",
    "models.downloadHint": "Fixed catalog from whisper.cpp. Size and SHA-256 are verified before installing.",
    "models.download": "Download",
    "models.use": "Use",
    "models.inUse": "In use",
    "models.downloading": "Downloading",
    "models.verifying": "Verifying integrity",
    "models.installing": "Installing",
    "models.cancel": "Cancel",
    "models.info.downloaded": "Model downloaded and verified. It now appears in the list.",
    "models.error.space": "Not enough free disk space.",
    "models.error.download": "Download failed.",
    "models.error.size": "The downloaded file has an unexpected size.",
    "models.error.hash": "SHA-256 verification failed.",
    "models.error.install": "The model could not be installed.",

    "models.baseq5": "Base q5_1",
    "models.baseF16": "Base (f16)",
    "models.smallq5": "Small q5_1",
    "models.smallF16": "Small (f16)",
    "models.mediumq5": "Medium q5_0",
    "models.turboq5": "Large v3 turbo q5_0",
    "models.largeq5": "Large v3 q5_0",
    "models.note.base": "Recommended for CPU",
    "models.note.baseF16": "Local reference; uses CPU",
    "models.note.small": "Higher quality; uses GPU",
    "models.note.smallF16": "Higher quality; GPU if available",
    "models.note.medium": "Best with GPU; slow on CPU",
    "models.note.turbo": "Best choice with a GPU",
    "models.note.large": "Maximum quality; needs GPU and RAM",

    "activity.last": "LAST TURN",
    "activity.header": "ACTIVITY",
    "activity.empty": "No turns yet. When you speak, the transcript and its translation appear here.",
    "activity.refresh": "Refresh log",
    "activity.openLog": "Open the full log in a terminal",

    "turn.userToMeet": "YOU -> MEET",
    "turn.meetToUser": "MEET -> YOU",

    "log.noService": "No connection to Duet.",
    "log.empty": "No activity yet.",

    "ui.toSpanish": "Switch the interface to Spanish",
    "ui.toEnglish": "Switch the interface to English",

    "widget.live": "Duet live",
    "widget.starting": "Duet starting…",
    "widget.stopping": "Duet stopping…",
    "widget.off": "Duet off",
    "widget.hint": " — click to turn on/off, right-click for settings"
  })

  readonly property var table: lang === "en" ? en : es

  function t(key) {
    var value = table[key]
    return typeof value === "string" ? value : key
  }
}
