# Models used by Duet

Everything runs **100% locally**; no stage calls the cloud.

## Audio generation (TTS)

To **generate audio** (text -> speech), the project uses **Piper**
(`rhasspy/piper-voices`) with one voice per language:

| | Spanish | English |
|---|---|---|
| Voice file | `models/es_MX-claude-high.onnx` | `models/en_US-lessac-medium.onnx` |
| Configuration | `es_MX-claude-high.onnx.json` | `en_US-lessac-medium.onnx.json` |
| Piper ID | `es/es_MX/claude/high` | `en/en_US/lessac/medium` |
| Language | Spanish (Mexico), `es_MX`, eSpeak `es-419` | English (US), `en_US`, eSpeak `en-us` |
| Dataset / speaker | `claude` | `lessac` |
| Quality | **high** | **medium** |
| Sample rate | 22050 Hz | 22050 Hz |
| Speakers | 1 | 1 |
| Size | ~63 MB | ~63 MB |

- **Architecture:** VITS models exported to ONNX, run by the Python `piper-tts`
  package (`PiperVoice.load`), with eSpeak-NG for phonemization.
- **Runtime:** `daemon/scripts/tts_server.py` keeps both voices **resident in
  memory** (loaded once) and synthesizes 16-bit mono WAV on demand over a
  JSON-lines protocol. This was deliberate: invoking the Piper CLI per turn cost
  ~4.4 s, almost all process startup and model loading.
- **Download sources:**
  `https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_MX/claude/high/es_MX-claude-high`
  and `.../en/en_US/lessac/medium/en_US-lessac-medium` (see
  `daemon/scripts/setup.sh`).
- Test WAVs such as `/tmp/duet-live_turn_*.wav` are outputs from exactly
  these two voices.

## Context: the rest of the local pipeline (does not generate audio)

- **ASR:** `models/ggml-base-q5_1.bin` — OpenAI's **Whisper base**, q5_1
  quantized, through `whisper.cpp` (`ggml-base.bin`, f16, is kept as a quality
  reference).
- **Translation:** **Argos/OPUS-MT v1.0** packages (`es_en-1_0`, `en_es-1_0` —
  Marian NMT) converted to **CTranslate2**, with SentencePiece tokenization.
- **VAD:** **WebRTC VAD** by default; **Silero v5.1.2**
  (`models/ggml-silero-v5.1.2.bin`) is available as an optional mode.
