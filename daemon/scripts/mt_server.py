#!/usr/bin/env python3
"""
MT sidecar for Duet.

Keeps the CTranslate2 (opus-mt) models resident in memory and translates over
a JSON-lines protocol on stdin/stdout. It is designed to be launched once by
the Rust daemon and stay alive (avoiding a model reload on every turn).

Protocol:
  stdin  : one JSON line per request  {"dir":"es_en"|"en_es","text":"..."}
  stdout : one JSON line per response {"ok":true,"text":"...","ms":12.3}
           or {"ok":false,"error":"..."}
"""
import json
import os
import sys
import time
from pathlib import Path

import ctranslate2
import sentencepiece as spm

ROOT = Path(os.environ.get("DUET_ROOT", Path(__file__).resolve().parent.parent))


def _models_dir(root: Path) -> Path:
    override = os.environ.get("DUET_MODELS_DIR")
    if override:
        return Path(override)
    # Installed runtime: <runtime>/models. Development checkout:
    # <repo>/daemon/models does not exist, but <repo>/models does.
    for candidate in (root / "models", root.parent / "models"):
        if candidate.is_dir():
            return candidate
    return root / "models"


MODELS_DIR = _models_dir(ROOT)
MODELS_ROOT = MODELS_DIR / "mt"

# dir -> (ct2 model dir, sentencepiece model)
#
# Note: we use the Argos v1.0 packages for both directions because they ship a
# standard binary `sentencepiece.model`. The es->en v1.9 package uses a
# subword-nmt-style BPE (merge codes as plain text) that would require a
# separate tokenizer, so it is intentionally not supported here.
CONFIG = {
    "es_en": (
        MODELS_ROOT / "es_en-1_0" / "es_en" / "model",
        MODELS_ROOT / "es_en-1_0" / "es_en" / "sentencepiece.model",
    ),
    "en_es": (
        MODELS_ROOT / "en_es-1_0" / "en_es" / "model",
        MODELS_ROOT / "en_es-1_0" / "en_es" / "sentencepiece.model",
    ),
}


def load(intra_threads: int):
    engines = {}
    for key, (model_dir, spm_path) in CONFIG.items():
        if not model_dir.exists():
            print(f"[mt] missing model {key} at {model_dir}", file=sys.stderr)
            sys.exit(2)
        sp = spm.SentencePieceProcessor()
        sp.load(str(spm_path))
        # compute_type int8: quantize on load if the model is stored as f32.
        translator = ctranslate2.Translator(
            str(model_dir), device="cpu", compute_type="int8",
            intra_threads=intra_threads, inter_threads=1,
        )
        engines[key] = (translator, sp)
        print(f"[mt] loaded {key} from {model_dir}", file=sys.stderr)
    return engines


def translate(engines, key, text):
    translator, sp = engines[key]
    tokens = sp.encode(text, out_type=str)
    results = translator.translate_batch([tokens], beam_size=1)
    out_tokens = results[0].hypotheses[0]
    return sp.decode(out_tokens)


def main():
    intra = int(sys.argv[1]) if len(sys.argv) > 1 else 2
    engines = load(intra)
    print("[mt] ready", file=sys.stderr, flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            t0 = time.perf_counter()
            out = translate(engines, req["dir"], req["text"])
            ms = (time.perf_counter() - t0) * 1000.0
            resp = {"ok": True, "text": out, "ms": round(ms, 2)}
        except Exception as exc:  # noqa: BLE001 - report any failure to the daemon
            resp = {"ok": False, "error": f"{type(exc).__name__}: {exc}"}
        sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
