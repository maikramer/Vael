"""Render the bronze strike and the two descending calls as one spatial mono voice."""

from pathlib import Path
import numpy as np
import soundfile as sf

RATE = 44100
t = np.arange(18 * RATE) / RATE
signal = np.zeros_like(t)
for ratio, amp, decay in [
    (1, 0.43, 2.4),
    (1.99, 0.25, 1.6),
    (2.39, 0.22, 1.19),
    (3.04, 0.12, 0.92),
    (4.16, 0.13, 0.52),
    (5.38, 0.08, 0.32),
    (6.75, 0.05, 0.16),
]:
    phase = 2 * np.pi * 246.94 * ratio * t
    signal += (
        amp
        * (np.sin(phase) + 0.29 * np.sin(phase * 1.0031))
        * np.exp(-t / decay)
        * (1 - np.exp(-t / 0.003))
    )
for start, dur, notes in [
    (2.2, 4.8, [115, 174, 152, 96]),
    (7.1, 5.7, [103, 139, 118, 72]),
]:
    use = (t >= start) & (t < start + dur)
    u = (t[use] - start) / dur
    freq = np.interp(u, [0, 0.29, 0.57, 1], notes) * (1 + 0.012 * np.sin(17 * u))
    phase = np.cumsum(2 * np.pi * freq / RATE)
    envelope = np.sin(np.pi * u) ** 1.35
    voice = (
        np.sin(phase)
        + 0.51 * np.sin(2 * phase + 0.6)
        + (0.23 + 0.13 * np.sin(np.pi * u)) * np.sin(3 * phase)
    )
    voice += 0.11 * np.sin(5 * phase + 0.9 * np.sin(9 * u)) + 0.065 * np.sin(8 * phase)
    signal[use] += 0.40 * envelope * voice
wet = signal.copy()
for delay, gain in [
    (0.19, 0.20),
    (0.37, 0.14),
    (0.57, 0.085),
    (0.79, 0.048),
    (1.12, 0.032),
]:
    offset = int(delay * RATE)
    wet[offset:] += gain * signal[:-offset]
out = (
    Path(__file__).resolve().parents[2]
    / "assets/public/assets/audio/sfx/world/canto_da_mare.ogg"
)
sf.write(out, 0.79 * wet / max(abs(wet)), RATE, format="OGG", subtype="VORBIS")
