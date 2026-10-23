# Daemon architecture and benchmarks

This document covers the Rust daemon and the Python sidecars. For installation
and plugin usage, see the repository root `README.md`.

## Pipeline

```text
WAV ──► VAD ──► ASR ──► MT ──► TTS ──► translated WAV
        │       │       │       │
        │       │       │       └─ resident Python sidecar (Piper/ONNX)
        │       │       └───────── resident Python sidecar (CTranslate2/OPUS-MT)
        │       └───────────────── native whisper.cpp (whisper-rs), resident model
        └───────────────────────── energy VAD in Rust
```

- **Rust daemon (`daemon/src/`):** orchestrates everything, loads the Whisper
  model once, and reuses the graph across turns.
- **Resident Python sidecars (`daemon/scripts/`):** MT and TTS. They are
  separate persistent processes for performance: invoking the CLIs per turn
  cost **4.4 s** (TTS), and reloading models per turn was similarly expensive.
  Keeping them alive reduces both to hundreds of milliseconds.
- **IPC:** JSON-lines over stdin/stdout with each sidecar.

The design was validated first with standalone CLI tools; the numbers forced
this architecture. A future native-Rust port (`ct2rs` for MT, `ort` or
`sherpa-onnx` for TTS) would remove the Python interpreters (~30-60 MB and some
jitter) without changing the pipeline.

## Live mode

```bash
duet --live --model models/ggml-base-q5_1.bin
```

Microphone (`pw-record`) -> streaming VAD -> when the phrase closes:
ASR -> MT -> TTS -> playback (`pw-play`).

During processing and playback the daemon discards microphone input. This
prevents the echo loop and is consistent with turn-based operation. If you speak
while the translation is playing, that part is lost; headphones are recommended.

### VAD choice

| VAD | Result on the development machine |
|---|---|
| **Energy** (RMS) | The internal microphone noise floor is around **-30 dBFS**. A relative threshold (`floor x 3`) required speaking much louder than the room and detected nothing. |
| **Silero** (whisper.cpp) | Accurate but **slower than real time**: 1600 samples -> 127 ms, 16000 -> 436 ms. whisper.cpp rebuilds the ggml graph on every call (~90 ms fixed overhead). Not viable on this CPU; still available with `--vad silero`. |
| **WebRTC** (selected) | Stateful GMM designed for streaming. Negligible CPU cost and much more robust to background noise. |

With the internal microphone, measuring 5 s of room noise: WebRTC VAD mode 0
reported 30% false "speech", mode 2 reported 13%, and **mode 3 reported 4%**.
The default is therefore mode 3, plus a **200 ms** confirmation before opening a
turn (to reject isolated peaks).

### Measured live latency

The number that matters is **"you stop speaking -> the translation starts
playing"**:

```text
end of speech -> playback = VAD pause + ASR + MT + TTS
```

| Turn | Pause | ASR | MT | TTS | **Total** |
|---|---:|---:|---:|---:|---:|
| 1.3 s of speech | 0.70 s | 0.67 s | 0.04 s | 0.15 s | **1.56 s** |
| 3.8 s of speech | 0.70 s | 1.17 s | 0.12 s | 0.44 s | **2.42 s** |

The breakdown matches the program's own measurements.

## Meet mode

```bash
duet --meet \
  --model models/ggml-base-q5_1.bin \
  --mic-device Built-in \
  --speaker-device Redmi
```

The daemon creates two virtual nodes and removes them on exit, including
leftovers from a previous interrupted run:

- `duet_mic_src` (`module-pipe-source`): the translation **toward** Meet.
  It is the only device Meet sees as a microphone; there is no intermediate
  sink, so it does not also appear as an output.
- `duet_meet` (`module-null-sink`): what **comes from** Meet. It is the
  only device Meet sees as a speaker; its monitor is captured with `parec`,
  without an intermediate source, so it does not also appear as an input.

In Meet, select:

```text
Microphone: Duet-Mic
Speakers:   Duet-Meet
```

### Why `parec` for the monitor

`pw-record --target <sink>.monitor` does not work: `pw-dump` does not expose
monitors as PipeWire nodes (only the PulseAudio compatibility layer lists them),
so `pw-record` cannot resolve the name and records silence. `parec` resolves
`<sink>.monitor`, delivers raw f32 PCM, and connects even when Meet is not
playing anything yet.

### Why a pipe source for the microphone

Chromium hides `*.monitor` devices from its microphone selector, so Meet needs a
real source. A `module-null-sink` + `module-remap-source` works, but exposes two
devices (the sink in outputs and the source in inputs). `module-pipe-source`
provides the source without a sink: the daemon writes 22050 Hz mono s16le PCM
into a FIFO and Meet captures it directly. No shell rules are needed.

### Echo control

English audio goes to the virtual microphone FIFO (it does not play through the
room, so there is no acoustic echo). Spanish audio goes to your headphones; while
it plays, microphone input is discarded to suppress distant echo. Headphones are
recommended.

## Results

Development machine: **Intel i5-5257U** (Broadwell, 2c/4t), **7.8 GB RAM**
(~4.9 GB available), and **Intel Iris 6100** graphics (Vulkan 1.3 through Mesa
ANV). No NVIDIA/CUDA GPU. Test audio: 28 s of real human speech (a Spanish
Wikipedia article) and 11 s of synthetic Piper audio.

### Latency (`base-q5_1`)

| Stage | Per turn (real audio, ~6 s/turn) | Per turn (synthetic, ~3 s/turn) |
|---|---:|---:|
| ASR | 2.06 s | 1.02 s |
| MT | 0.18 s | 0.10 s |
| TTS | 0.90 s | 0.41 s |
| **Compute total** | **3.15 s** | **1.53 s** |
| VAD pause wait | +0.9 s | +0.7 s |
| **Perceived latency** | **~4.0 s** | **~2.2 s** |

- **Compute RTF ~= 0.5x:** the pipeline processes audio faster than it is
  spoken and keeps up with a conversation.
- Cost scales with phrase length (constant RTF); there is no fixed ceiling.

### RAM (steady state)

| Component | RSS |
|---|---:|
| Rust daemon (Whisper base-q5_1) | ~75 MB |
| MT sidecar (2 OPUS-MT int8 models) | ~600 MB |
| TTS sidecar (2 Piper voices) | ~460 MB |
| **Total** | **~1.2 GB** |

The dominant MT cost is inherent to CTranslate2: each ~85 MB int8 model uses
~330 MB of RAM (`float32` would use ~590 MB). It is not configurable through
`compute_type` or thread count.

### GPU (Vulkan)

Whisper.cpp can use the integrated GPU through the Vulkan backend. The device on
this machine is:

```text
Intel(R) Iris(R) Graphics 6100 (BDW GT3)
Vulkan 1.3, Mesa ANV, fp16: yes, integer dot: no, matrix cores: none
```

Without `--features vulkan`, the binary is CPU-only. To enable Vulkan:

```bash
sudo pacman -S vulkan-headers
cargo build --release -p duet --features vulkan
```

The runtime is already covered by `vulkan-intel` + `vulkan-icd-loader`.

`--gpu auto` (the default) uses the GPU for models of **150 MiB or more**:
`small`, `medium`, and `large`; `base` stays on the CPU because Vulkan does not
offset the initialization cost at that size. Force it with `--gpu on` or
`--gpu off`.

Measurement with a synthetic 3.6 s turn (`--input --passes 2`, steady state):

| Model | CPU | GPU Vulkan | Gain |
|---|---:|---:|---:|
| `base-q5_1` | 1.08 s | 1.14 s | not worth it (auto uses CPU) |
| `small-q5_1` | 3.77 s | 2.85 s | 1.3x |
| `medium-q5_0` | 12.44 s | 5.78 s | 2.2x |
| `large-v3-turbo-q5_0` | 18.65 s | 6.39 s | 2.9x |

The first Vulkan run of a model compiles shaders and can take much longer (for
`large`, the first turn measured ~18 s versus ~6.7 s in steady state); after
that it is cached. In live mode, the first turn after that initial startup pays
the cost.

`--meter`, the VAD, MT, and TTS still run on the CPU; only Whisper ASR is moved
to the GPU.

### ASR model selection

| Model | ASR/turn | Quality |
|---|---|---|
| `tiny-q5_1` | 2.0 s | Unusable: repeats the same phrase in a loop |
| **`base-q5_1`** | **1.0 s** | **Selected for CPU** — good quality and low latency |
| `base` (f16) | 1.0 s | Same, but +85 MB RAM |
| `small-q5_1` | 3.7 s CPU / 2.9 s GPU | Slightly better; a good fit with the GPU |

### Observed quality

Translation quality is good. Example:

> `La isla de Margarita... llamada La Perla del Caribe está ubicada...`
> -> `The island of Margarita... called La Perla del Caribe is located...`

The weak link is **ASR with proper nouns and accented words**:
`venezolano -> venizolano`, `Sucre -> Suque`, `Denominado -> Donominado`.
Improve it with `--prompt` (a proper-noun glossary) or a larger model if latency
allows.

#### Real voice (40 s recording of the script, `bench/script_es.txt`)

Translation of sentences with numbers, dates, and names was mostly good, except:

| Spoken | ASR | Translation | Problem |
|---|---|---|---|
| "buen día" | `bon dia` | "Hello, Bon dia" | ASR |
| "revises el informe de Ana" | `raíces ... ana` | "the Ana report **rooted**" | ASR + MT |
| "45 mil dólares" | correct | "**$450,000**" | MT (x10) |
| "el servidor se cayó" | correct | "the server **fell**" | MT (should be *crashed*) |

Conclusions: **numbers** are a weak point of OPUS-MT (`45 thousand -> $450,000`);
numeric normalization before translation would fix it. ASR errors propagate to
MT (`raíces` -> *rooted*), so improving ASR has double value.

## Known limitations

- ASR proper-noun and accent errors remain the main quality issue.
- `initial_prompt` did not help in the tests (it was slightly worse); it remains
  available for real glossaries.
- The internal microphone has a very high noise floor (~-30 dBFS). A Bluetooth
  headset microphone close to the mouth works better.
- Live mode loses speech that happens during playback (microphone input is
  discarded to suppress echo).
- The VAD cannot distinguish background conversation from the speaker; a
  better signal-to-noise ratio helps more than a heavier VAD.

## Development

```bash
bash daemon/scripts/setup.sh
cargo build --release -p duet
cargo build --release -p duet --features vulkan
omarchy plugin validate .
python3 -m py_compile daemon/scripts/*.py
bash -n daemon/scripts/*.sh
```
