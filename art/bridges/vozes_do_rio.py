"""Render Vessa's struck bronze and the river's four-voice answer as mono Vorbis."""

from pathlib import Path
from subprocess import run

import numpy as np

RATE = 44100
OUT = Path(__file__).resolve().parents[2] / "assets/public/assets/audio/sfx/world"


def save(name, wave, level):
    # The game's positional custom-clip voice is stopped in the quiet tail.
    pcm = np.pad(level * wave / np.max(np.abs(wave)), (0, RATE * 3)).astype("<f4")
    run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "f32le", "-ar", str(RATE),
         "-ac", "1", "-i", "-", "-c:a", "libvorbis", "-q:a", "5", str(OUT / (name + ".ogg"))],
        input=pcm.tobytes(), check=True)


def bell(t, hz):
    result = np.zeros_like(t)
    for ratio, gain, decay in [(1,.72,1.02), (2.005,.25,.64), (2.73,.18,.38), (4.16,.15,.24),
                                 (5.43,.064,.14), (7.28,.042,.09)]:
        angle = 2*np.pi*hz*ratio*t
        result += gain*(np.sin(angle)+.36*np.sin(angle*1.0023))*np.exp(-t/decay)
    return result*(1-np.exp(-t/.004))


def voice(t, hz, detune):
    phase = 2*np.pi*t*hz + .12*np.sin(2*np.pi*(4.2+detune)*t)
    note = np.zeros_like(t)
    for harmonic in range(1, 14):
        f = hz*harmonic
        # A rounded "ou" opening to "ah".
        weight = .17/harmonic + .4*np.exp(-((f-580)/210)**2) + .14*np.exp(-((f-1100)/230)**2)
        note += weight*np.sin(harmonic*phase)/np.sqrt(harmonic)
    return note*np.minimum(t/.45, 1)**2 * np.exp(-t/1.02) * np.clip((2.65-t)/.8, 0, 1)


def render():
    d4 = 293.6648
    t = np.arange(3*RATE) / RATE
    save("vessa_bronze", bell(t, d4)*np.clip((3-t)/.4, 0, 1), .72)
    t = np.arange(6*RATE) / RATE
    song = np.zeros_like(t)
    for start, semitone in [(0,0), (.63,5), (1.35,2), (2.28,-1)]:
        inside = t >= start
        local = t[inside]-start
        hz = d4 * 2**(semitone/12)
        song[inside] += .28*bell(local,hz) + voice(local,hz/2,-.15) + .55*voice(local,hz*1.003,.22)
    tail = np.zeros_like(t)
    for delay, gain in [(.12,.2), (.205,.13), (.36,.075)]:
        n = int(delay*RATE)
        tail[n:] += gain*song[:-n]
    save("vessa_quarta_voz", (song+tail)*np.clip((6-t)/.65, 0, 1), .78)


if __name__ == "__main__":
    render()
