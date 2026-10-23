# Duet

Duet is a local **Spanish <-> English voice interpreter** for video calls,
packaged as an [Omarchy](https://omarchy.org) plugin. Only one participant runs
the app; the other participant does not install anything and keeps using Meet
normally.

Duet is a vibe-coded project in an experimental state. It works well enough for
everyday use, but expect rough edges and occasional breaking changes.

The public repository is
**[omarchy-duet](https://github.com/vdg0/omarchy-duet)** and the Omarchy plugin
ID is `io.github.vdg0.duet`. The repository root is the plugin. The Rust daemon
and its Python sidecars live under `daemon/`.

## Features

- **Live mode:** listens to the microphone, detects speech turns, and plays the
  translation through the speakers or headphones.
- **Meet mode:** bidirectional translation inside a video call using virtual
  PipeWire devices (`duet_mic_src` and `duet_meet`).
- **100% local:** Whisper.cpp for ASR, CTranslate2/OPUS-MT for translation, and
  Piper for TTS. No cloud calls.
- **Bilingual UI:** Spanish and English interface strings; logs and diagnostics
  are in English.
- **GPU acceleration:** optional Vulkan backend for Whisper. Models of 150 MiB
  or more use the GPU automatically; base models stay on the CPU.
- **Model manager:** the panel can download and verify models from a fixed
  catalog.

## Requirements

- Omarchy/Arch Linux with PipeWire (and the PulseAudio compatibility tools).
- `pw-record`, `pw-play`, `parec`, `pactl`, `socat`, `curl`, `unzip`.
- Rust toolchain for building the daemon.
- Python 3.12 and [`uv`](https://github.com/astral-sh/uv) for the sidecars.
- Optional: `ffprobe` (from `ffmpeg`) for the development recording helper.
- Optional: `vulkan-headers` and a Vulkan-capable GPU for GPU inference.

`install.sh` can install the runtime dependencies it knows about, but it does
not use `sudo`.

## Install as an Omarchy plugin

```bash
omarchy plugin add https://github.com/vdg0/omarchy-duet.git
```

This installs the plugin into
`~/.config/omarchy/plugins/io.github.vdg0.duet`. Then install the runtime
from that directory:

```bash
cd ~/.config/omarchy/plugins/io.github.vdg0.duet
./install.sh --gpu        # or --cpu, or no flag for auto-detection
```

`install.sh` builds the daemon and installs the runtime outside the plugin
checkout, so the plugin directory stays clean:

| What | Default location |
|---|---|
| Binary, helpers, models, venv | `${XDG_DATA_HOME:-~/.local/share}/duet` |
| Configuration | `${XDG_CONFIG_HOME:-~/.config}/duet/config.json` |
| Log | `${XDG_STATE_HOME:-~/.local/state}/duet.log` |
| Control socket and PID | `${XDG_RUNTIME_DIR:-/tmp}/duet.sock` |
| Generated WAV files | `${XDG_RUNTIME_DIR:-/tmp}/duet/out` |

### Install from a checkout

```bash
git clone https://github.com/vdg0/omarchy-duet.git omarchy-duet
cd omarchy-duet
./install.sh --gpu
```

For development without rebuilding or re-downloading models:

```bash
./install.sh --binary target/release/duet \
  --link-models --link-venv --no-models
```

See `./install.sh --help` for all options.

## Usage

Open the Omarchy panel and enable the **Duet** bar widget. The panel can:

- start and stop Duet;
- choose live or Meet mode;
- select the microphone and speaker/headphones;
- choose the ASR model and download new models;
- switch the interface language between Spanish and English;
- show the current state, the last turn, and the daemon log.

The bar widget shows the current state and toggles the daemon with a click.

### Live mode

Use headphones: speaker output feeds back into the microphone. While the
translation is playing, microphone input is discarded to avoid an echo loop, so
speech during playback is intentionally lost.

### Meet mode

1. Start Meet mode from the panel.
2. In Meet, open **Settings -> Audio** and select:
   - **Microphone:** `Duet-Mic`
   - **Speakers:** `Duet-Meet`
3. Speak in Spanish and pause: Meet receives the English translation. When the
   other participant speaks English, you hear the Spanish translation in your
   headphones.

The virtual devices are removed when the daemon exits, including leftovers from
an interrupted previous run.

## CLI

The daemon can also be used directly for diagnostics and benchmarking. Add the
runtime `bin` directory to `PATH` (or use the absolute path):

```bash
export PATH="${XDG_DATA_HOME:-$HOME/.local/share}/duet/bin:$PATH"

duet --input conversation.wav --from es --to en
duet --live --model models/ggml-base-q5_1.bin
duet --meet --model models/ggml-base-q5_1.bin
```

Useful flags:

| Flag | Description |
|---|---|
| `--from es`, `--to en` | Source and target languages. Legacy aliases `--de` and `--a` are still accepted. |
| `--input FILE` | WAV file to process in benchmark mode. Legacy alias `--in` is accepted. |
| `--live` | Live microphone mode. |
| `--meet` | Bidirectional Meet mode. |
| `--max-turns N` | Stop after N turns (0 = unlimited). |
| `--pause-ms 700` | Silence pause that closes a turn. Lower it for faster replies. |
| `--min-speech-ms 300` | Minimum speech duration before opening a turn. |
| `--mic-device NAME` | PipeWire source (name or description fragment). |
| `--speaker-device NAME` | PipeWire sink (name or description fragment). |
| `--list-devices` | List microphones and speakers with their node names. |
| `--meter` | Show live microphone level and VAD probability. |
| `--test-speaker` | Play a short tone through the selected output. |
| `--gpu auto\|on\|off` | Select the Whisper backend. `auto` uses Vulkan for models >= 150 MiB. |
| `--vad auto\|webrtc\|silero\|energy` | Streaming VAD engine. WebRTC is the default. |
| `--no-play` | Do not play translated audio (print only). |
| `--verbose` | Do not silence whisper.cpp logs. |

The runtime launcher is:

```bash
RUNTIME="${XDG_DATA_HOME:-$HOME/.local/share}/duet"
"$RUNTIME/bin/duet-toggle" start
"$RUNTIME/bin/duet-toggle" stop
"$RUNTIME/bin/duet-toggle" status
```

## Troubleshooting

### The VAD does not detect my voice

Run the meter and speak normally:

```bash
"${XDG_DATA_HOME:-$HOME/.local/share}/duet/bin/duet" --meter
```

If the level bar does not move, the problem is the microphone (wrong device,
gain, or Bluetooth profile), not the VAD.

### List and test audio devices

```bash
duet --list-devices
duet --test-speaker --speaker-device Redmi
duet --live --model models/ggml-base-q5_1.bin \
  --mic-device "Built-in" --speaker-device "Redmi"
```

Device names and descriptions are accepted; a description fragment such as
`Redmi` is enough.

### Bluetooth

- A Bluetooth headset cannot use A2DP (high-quality output) and HFP (headset
  microphone) at the same time. If only `bluez_output...` appears and no
  `bluez_input...`, there is no microphone to capture. Switch the profile:

  ```bash
  wpctl status
  wpctl set-profile <device-id> headset-head-unit
  duet --list-devices
  ```

- WirePlumber may remember a headset that is not currently connected. If the
  default output points to a missing device, select the correct output with
  `--speaker-device` or in the panel.

### The daemon does not start

Check the log:

```bash
tail -n 100 ${XDG_STATE_HOME:-$HOME/.local/state}/duet.log
```

If the runtime is missing, run `install.sh` from the plugin directory.

## Development

```bash
# Toolchain, Python sidecar environment, and SHA-256-verified models.
bash daemon/scripts/setup.sh

# CPU build
cargo build --release -p duet

# GPU build (requires vulkan-headers)
cargo build --release -p duet --features vulkan

# Validate the Omarchy plugin (run before setup.sh, or from a clean clone:
# the ignored models/ and .venv-py/ development directories contain symlinks).
omarchy plugin validate .

# Quick syntax checks
python3 -m py_compile daemon/scripts/*.py
bash -n daemon/scripts/*.sh
```

The workspace root is a virtual Cargo workspace; the daemon package is
`daemon/`. `target/`, `models/`, and `.venv-py/` are intentionally ignored.

## Repository layout

```text
.
├── manifest.json           Omarchy plugin manifest
├── BarWidget.qml           Bar widget
├── Panel.qml               Main panel
├── Config.qml              XDG paths and persisted preferences
├── Models.qml              Model catalog/downloader
├── Service.qml             Daemon/socket bridge
├── Strings.qml             ES/EN UI strings
├── install.sh              Runtime installer
├── LICENSE                 GPL-3.0-only
├── Cargo.toml              Workspace
└── daemon/
    ├── Cargo.toml
    ├── src/                Rust daemon
    ├── scripts/            Python sidecars and launcher scripts
    ├── bench/              Benchmark fixtures (Spanish test script)
    ├── MODELS.md           Model details
    └── README.md           Architecture and benchmark notes
```

## Benchmarks

Measured on an Intel i5-5257U (2c/4t, 7.8 GB RAM) with Intel Iris 6100 Vulkan
graphics, using `base-q5_1`:

| Stage | Real audio (~6 s/turn) | Synthetic audio (~3 s/turn) |
|---|---:|---:|
| ASR | 2.06 s | 1.02 s |
| MT | 0.18 s | 0.10 s |
| TTS | 0.90 s | 0.41 s |
| **Compute total** | **3.15 s** | **1.53 s** |
| VAD pause wait | +0.9 s | +0.7 s |
| **Perceived latency** | **~4.0 s** | **~2.2 s** |

Steady-state RSS is about 1.2 GB (daemon + MT + TTS). See
`daemon/README.md` for the full benchmark notes, GPU results, and design
decisions.

## Known limitations

- The weak link is ASR with proper nouns and accented words; an initial prompt
  or a larger model can help.
- OPUS-MT can mis-handle numbers (`45 thousand -> $450,000`); numeric
  normalization before translation would improve this.
- The internal microphone on the development laptop has a high noise floor.
  A headset microphone close to the mouth works better.
- Live mode intentionally discards microphone input during playback to avoid
  echo, so speech during playback is lost.
- Silero VAD is available but too slow for real-time streaming on the
  development CPU; WebRTC VAD is the default.

## License

This project is licensed under the GNU General Public License, version 3 only
(GPL-3.0-only). See [LICENSE](LICENSE).
