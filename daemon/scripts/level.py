#!/usr/bin/env python3
"""Measure the level of a PipeWire input node (RMS/peak in dBFS).

Used to diagnose why the VAD does not trigger: if the level stays below the
threshold while speaking, detection is impossible.

Usage:  pw-record --target <node> --rate 16000 --channels 1 --format f32 --raw - \
          | python3 daemon/scripts/level.py [seconds]
"""
import sys
import array
import math

SR = 16000
BLOCK = SR // 10  # 100 ms


def dbfs(rms: float) -> float:
    return 20.0 * math.log10(max(rms, 1e-9))


def main() -> None:
    secs = float(sys.argv[1]) if len(sys.argv) > 1 else 3.0
    want = int(secs * SR)
    raw = sys.stdin.buffer.read(want * 4)
    samples = array.array("f")
    samples.frombytes(raw[: len(raw) - (len(raw) % 4)])
    if not samples:
        print("  (no samples: the node delivered no audio)")
        return

    levels = []
    for i in range(0, len(samples) - BLOCK + 1, BLOCK):
        block = samples[i : i + BLOCK]
        rms = math.sqrt(sum(s * s for s in block) / len(block))
        levels.append(rms)
    if not levels:
        return

    peak = max(abs(s) for s in samples)
    avg = sum(levels) / len(levels)
    loud = sorted(levels)[int(len(levels) * 0.95)]  # 95th percentile
    print(
        f"  {len(samples)/SR:4.1f}s | average {dbfs(avg):6.1f} dBFS | "
        f"p95 {dbfs(loud):6.1f} dBFS | peak {dbfs(peak) if peak>0 else -99:6.1f} dBFS"
    )
    print("  VAD threshold ~= -41.9 dBFS (RMS 0.008; rises with background noise)")


main()
