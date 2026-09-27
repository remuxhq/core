"""Silent samples in a recording's voice, found by where they fall.

    python3 engine/remuxd/smoke/holes.py ~/Movies/remux/remux-....mp4

A Bluetooth headset (hands-free, 16 kHz) carries nothing above 8 kHz, so a
transient above 9 kHz in its recording was made after the microphone: a few
samples of silence in the voice ring out across the whole band. When the
mixer reads a block short, those holes sit on its grid, one parity of its
480-frame blocks, every 960 samples; anything else lands anywhere. So the
transients are folded mod 960 and the busiest bin is compared with an even
spread, beside two periods the mixer has nothing to do with, the negative
controls. A real crackle folds about 26x at 960 against 2.5x to 2.8x at
1000 and 1024. Speak into the headset while recording: holes in silence
make no transient to find.
"""

import subprocess
import sys

import numpy as np

RATE = 48000
BINS = 48


def voice(path):
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-vn", "-af", "pan=mono|c0=c0",
         "-ar", str(RATE), "-f", "f32le", "pipe:1"],
        capture_output=True, check=True,
    ).stdout
    return np.frombuffer(raw, dtype=np.float32).astype(np.float64)


def transients(x):
    """Sample positions where the band above 9 kHz jumps over its surroundings."""
    spectrum = np.fft.rfft(x)
    spectrum[np.fft.rfftfreq(len(x), 1 / RATE) < 9000] = 0
    high = np.abs(np.fft.irfft(spectrum, len(x)))
    window = 480
    n = len(high) // window * window
    around = np.maximum(np.median(high[:n].reshape(-1, window), axis=1).repeat(window), 1e-5)
    peaks = np.where((high[:n] > 8 * around) & (high[:n] > 1e-3))[0]
    events, last = [], -(10**9)
    for p in peaks:  # one event per 2 ms
        if p - last > 96:
            events.append(p)
            last = p
    return np.array(events, dtype=np.int64)


def fold(events, period):
    """How many times an even spread the busiest bin of `events mod period` holds."""
    if len(events) == 0:
        return 0.0
    counts, _ = np.histogram(events % period, bins=BINS, range=(0, period))
    return counts.max() / len(events) * BINS


def main(path):
    x = voice(path)
    events = transients(x)
    seconds = len(x) / RATE
    level = 20 * np.log10(np.sqrt(np.mean(x**2)) + 1e-12)
    print(f"{path}: {seconds:.1f} s, voice {level:.1f} dBFS, "
          f"{len(events)} transients above 9 kHz ({len(events) / seconds * 60:.0f} a minute)")
    for period, what in ((960, "the mixer's grid"), (1000, "control"), (1024, "control")):
        print(f"  mod {period:>4} ({what}): busiest bin {fold(events, period):.1f}x an even spread")
    if level < -45:
        print("  the voice is too quiet to show a hole; speak into the headset")


if __name__ == "__main__":
    main(sys.argv[1])
