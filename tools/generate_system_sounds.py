#!/usr/bin/env python3
"""Generate the original "Sunlight Default" system-sound theme.

Development-time only: audiod embeds the resulting 48 kHz S16LE stereo WAV
files and needs no runtime synthesizer or external codec.
"""

from __future__ import annotations

import math
import struct
import wave
from pathlib import Path

RATE = 48_000
PEAK = 0.34
OUT = Path(__file__).resolve().parents[1] / "assets" / "sounds" / "Sunlight Default"


def envelope(t: float, duration: float) -> float:
    attack = min(0.018, duration * 0.16)
    release = min(0.095, duration * 0.38)
    a = min(1.0, t / attack) if attack else 1.0
    r = min(1.0, max(0.0, duration - t) / release) if release else 1.0
    return math.sin(a * math.pi / 2) * math.sin(r * math.pi / 2)


def render(name: str, duration: float, notes: list[tuple[float, float, float]]) -> None:
    """Render (start_seconds, frequency_hz, relative_gain) sine partials."""
    frames = bytearray()
    total = round(duration * RATE)
    for index in range(total):
        t = index / RATE
        sample = 0.0
        for start, frequency, gain in notes:
            local = t - start
            if local < 0:
                continue
            note_len = duration - start
            note_env = envelope(local, note_len)
            # A quiet octave partial gives every sound the same warm identity.
            sample += gain * note_env * (
                math.sin(2 * math.pi * frequency * local)
                + 0.16 * math.sin(2 * math.pi * frequency * 2 * local)
            )
        sample = max(-1.0, min(1.0, sample * PEAK))
        value = round(sample * 32767)
        frames.extend(struct.pack("<hh", value, value))

    OUT.mkdir(parents=True, exist_ok=True)
    with wave.open(str(OUT / f"{name}.wav"), "wb") as wav:
        wav.setnchannels(2)
        wav.setsampwidth(2)
        wav.setframerate(RATE)
        wav.writeframes(frames)


def main() -> None:
    # All tones use the same restrained sine-plus-octave palette.
    render("volume-changed", 0.12, [(0.00, 880, 0.66)])
    render("device-connected", 0.22, [(0.00, 523.25, 0.48), (0.07, 783.99, 0.42)])
    render("device-disconnected", 0.22, [(0.00, 659.25, 0.46), (0.07, 392.00, 0.42)])
    render("success", 0.30, [(0.00, 523.25, 0.42), (0.08, 659.25, 0.38), (0.15, 783.99, 0.34)])
    render("message", 0.28, [(0.00, 659.25, 0.44), (0.09, 783.99, 0.34)])
    render("notification", 0.34, [(0.00, 587.33, 0.42), (0.10, 880.00, 0.36)])
    render("question", 0.32, [(0.00, 523.25, 0.42), (0.13, 698.46, 0.38)])
    render("warning", 0.36, [(0.00, 440.00, 0.46), (0.12, 440.00, 0.38)])
    render("error", 0.42, [(0.00, 392.00, 0.48), (0.12, 329.63, 0.42)])
    render(
        "critical",
        0.65,
        [(0.00, 349.23, 0.46), (0.18, 293.66, 0.42), (0.36, 349.23, 0.38)],
    )


if __name__ == "__main__":
    main()
