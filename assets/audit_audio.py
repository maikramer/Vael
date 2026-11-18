#!/usr/bin/env python3
"""Audit the shipped OGG files and their manifests in CPU, including decoded true peaks."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

from regen_audio import AUDIO_ROOT, load_rows


def levels(path: Path) -> tuple[float, float, float, float]:
    """Duration, integrated LUFS, true peak in dBTP and energy above 7.5 kHz."""
    data, rate = sf.read(path, dtype="float32", always_2d=True)
    if not data.size or not np.isfinite(data).all() or not np.any(data):
        raise ValueError("empty, silent or non-finite signal")
    # Sum channel powers independently so out-of-phase stereo stays audible in the measurement.
    width = min(2048, len(data))
    blocks = data[: len(data) // width * width].reshape(-1, width, data.shape[1])
    power = (abs(np.fft.rfft(blocks * np.hanning(width)[None, :, None], axis=1)) ** 2).sum(axis=(0, 2))
    treble = 100 * power[np.fft.rfftfreq(width, 1 / rate) > 7500].sum() / power.sum()
    analysis = subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-hide_banner",
            "-nostats",
            "-i",
            str(path),
            "-af",
            "ebur128=peak=true",
            "-f",
            "null",
            "-",
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stderr.rsplit("Summary:", 1)[-1]
    values = re.findall(r"(?:\bI|Peak):\s+([\d.-]+) (?:LUFS|dBFS)", analysis)
    if len(values) != 2:
        raise ValueError("ffmpeg could not measure loudness and true peak")
    return len(data) / rate, float(values[0]), float(values[1]), treble


def main() -> int:
    rows = load_rows()
    files = {p.relative_to(AUDIO_ROOT).with_suffix("").as_posix() for p in AUDIO_ROOT.rglob("*.ogg")}
    ids = {r["id"] for r in rows}
    problems = [f"unlisted OGG: {clip}" for clip in sorted(files - ids)]
    if len(ids) != len(rows):
        problems.append("duplicate clip id in the manifests")
    print(f"{'Clip':34s} Segundos    LUFS    dBTP   >7,5kHz")
    for row in rows:
        clip = row["id"]
        path = AUDIO_ROOT / f"{clip}.ogg"
        try:
            side = json.loads(path.with_suffix(".ogg.json").read_text(encoding="utf-8"))
            if not side.get("model_id") or not side.get("prompt"):
                raise ValueError("missing model or prompt in sidecar")
            duration, lufs, peak, treble = levels(path)
        except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as err:
            problems.append(f"{clip}: {err}")
            continue
        print(f"{clip:34s}  {duration:7.2f}  {lufs:6.1f}  {peak:6.1f}  {treble:7.1f}%")
        if (row["profile"] == "music" or row["field_loop"]) and abs(duration - row["duration"]) > 0.25:
            problems.append(f"{clip}: loop length differs from manifest")
        if peak < -55:
            problems.append(f"{clip}: peak below -55 dBTP, clip is effectively silent")
        if peak >= 0:
            problems.append(f"{clip}: true peak reaches full scale")
    print(f"{len(rows)} entradas nos manifestos, {len(files)} OGG, {len(problems)} problemas")
    for problem in problems:
        print(problem, file=sys.stderr)
    return int(bool(problems))


if __name__ == "__main__":
    sys.exit(main())
