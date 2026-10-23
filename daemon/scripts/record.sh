#!/usr/bin/env bash
# Record from the microphone to 16 kHz mono WAV (the pipeline input format).
#
# Usage:  bash daemon/scripts/record.sh [output.wav] [seconds]
#         bash daemon/scripts/record.sh /tmp/duet-recording.wav 40
set -euo pipefail

out="${1:-/tmp/duet-recording.wav}"
secs="${2:-30}"
mkdir -p "$(dirname "$out")"

echo "Recording ${secs}s to ${out}"
echo "Speak now (pauses of ~1s between phrases)."
# SIGINT makes pw-record close the WAV cleanly.
timeout -s INT "$secs" pw-record --rate 16000 --channels 1 --format s16 "$out" || true

if [ -s "$out" ]; then
  dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$out" 2>/dev/null || echo "?")
  echo "Done: $out (${dur}s)"
else
  echo "ERROR: no audio was generated. Is the microphone muted or in use?" >&2
  exit 1
fi
