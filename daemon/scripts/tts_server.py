#!/usr/bin/env python3
"""
TTS sidecar for Duet.

Keeps the Piper voices resident in memory (ONNX loaded once) and synthesizes
over a JSON-lines protocol on stdin/stdout. It exists because we measured that
invoking the Piper CLI per turn cost ~4.4 s, almost all of it process startup
and model loading, not synthesis.

Protocol:
  stdin  : {"voice":"es"|"en","text":"...","out":"/path.wav"}
  stdout : {"ok":true,"ms":123.4,"audio_s":2.31} | {"ok":false,"error":"..."}
"""
import json
import os
import sys
import time
import wave
from pathlib import Path

from piper.voice import PiperVoice

ROOT = Path(os.environ.get("DUET_ROOT", Path(__file__).resolve().parent.parent))


def _models_dir(root: Path) -> Path:
    override = os.environ.get("DUET_MODELS_DIR")
    if override:
        return Path(override)
    for candidate in (root / "models", root.parent / "models"):
        if candidate.is_dir():
            return candidate
    return root / "models"


MODELS_DIR = _models_dir(ROOT)
VOICES = {
    "es": MODELS_DIR / "es_MX-claude-high.onnx",
    "en": MODELS_DIR / "en_US-lessac-medium.onnx",
}


def load():
    voices = {}
    for key, path in VOICES.items():
        if not path.exists():
            print(f"[tts] missing voice {key} at {path}", file=sys.stderr)
            sys.exit(2)
        voices[key] = PiperVoice.load(str(path))
        print(f"[tts] loaded voice {key} ({path.name})", file=sys.stderr)
    return voices


def main():
    voices = load()
    # Warmup: the first synthesis pays onnxruntime's lazy initialization.
    import io

    for key, voice in voices.items():
        buf = io.BytesIO()
        with wave.open(buf, "wb") as wf:
            voice.synthesize_wav("ok", wf)
        print(f"[tts] warmup {key} ready", file=sys.stderr)
    print("[tts] ready", file=sys.stderr, flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            voice = voices[req["voice"]]
            out = Path(req["out"])
            t0 = time.perf_counter()
            # We set the format ourselves: for empty text Piper emits no chunks,
            # so synthesize_wav would never get a chance to set it.
            with wave.open(str(out), "wb") as wf:
                wf.setnchannels(1)
                wf.setsampwidth(2)
                wf.setframerate(voice.config.sample_rate)
                if req["text"].strip():
                    voice.synthesize_wav(req["text"], wf, set_wav_format=False)
            ms = (time.perf_counter() - t0) * 1000.0
            with wave.open(str(out), "rb") as wf:
                audio_s = wf.getnframes() / float(wf.getframerate())
            resp = {"ok": True, "ms": round(ms, 2), "audio_s": round(audio_s, 3)}
        except Exception as exc:  # noqa: BLE001 - report any failure to the daemon
            resp = {"ok": False, "error": f"{type(exc).__name__}: {exc}"}
        sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
