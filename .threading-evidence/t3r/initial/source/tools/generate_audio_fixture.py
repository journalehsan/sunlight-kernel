#!/usr/bin/env python3
"""Generate the deterministic Melody Mina PCM fixture without external tools."""

import math
import struct
import wave
from pathlib import Path

RATE = 48_000
SECONDS = 6
FRAMES = RATE * SECONDS
OUTPUT = Path(__file__).resolve().parents[1] / "assets/sounds/melody-mina-sample-48k-stereo.wav"

notes = (261.63, 329.63, 392.00, 523.25, 392.00, 329.63)

with wave.open(str(OUTPUT), "wb") as wav:
    wav.setnchannels(2)
    wav.setsampwidth(2)
    wav.setframerate(RATE)
    frames = bytearray()
    for index in range(FRAMES):
        t = index / RATE
        note = notes[min(int(t), len(notes) - 1)]
        fade = min(1.0, index / (RATE * 0.08), (FRAMES - index) / (RATE * 0.12))
        left = 0.32 * math.sin(2 * math.pi * note * t) + 0.10 * math.sin(2 * math.pi * note * 2 * t)
        right = 0.32 * math.sin(2 * math.pi * note * 1.5 * t) + 0.10 * math.sin(2 * math.pi * note * 3 * t)
        frames.extend(struct.pack("<hh", int(left * fade * 32767), int(right * fade * 32767)))
    wav.writeframes(frames)

print(f"{OUTPUT}: {RATE} Hz, stereo, signed 16-bit, {FRAMES} frames, {SECONDS:.3f}s")
