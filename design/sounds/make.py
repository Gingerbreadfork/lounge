#!/usr/bin/env python3
"""Renders the three UI sounds (DESIGN.md §9) as 48 kHz 16-bit mono WAV files.

tick   a felt-covered key: a short wooden tap with a soft low body
open   a felt mallet on a warm wooden bar (G3) with a breath of air, lamp coming on
close  the same mallet a fourth lower (D3), darker, settling downward

Usage: python3 design/sounds/make.py [out dir]
"""
import math
import os
import sys

import numpy as np
from scipy import signal

SR = 48000
TARGET_LUFS = -18.0


def seconds(ms):
    return int(SR * ms / 1000)


def decay(n, tau_ms):
    t = np.arange(n) / SR
    return np.exp(-t / (tau_ms / 1000))


def attack(n, ms):
    a = seconds(ms)
    env = np.ones(n)
    env[:a] = 0.5 - 0.5 * np.cos(np.linspace(0, math.pi, a))
    return env


def partial(n, freq, tau_ms, amp, glide=1.0, attack_ms=3.0):
    """A sine partial with exponential decay; `glide` multiplies the pitch at the start and eases to 1."""
    t = np.arange(n) / SR
    g = 1 + (glide - 1) * np.exp(-t / (tau_ms / 1000))
    phase = 2 * math.pi * np.cumsum(freq * g) / SR
    return amp * np.sin(phase) * decay(n, tau_ms) * attack(n, attack_ms)


def noise_band(n, center, q, length_ms, amp, rise_ms=0.5, start_ms=0.0, seed=1):
    """Band-passed noise with a raised-cosine rise and an exponential fall."""
    rng = np.random.default_rng(seed)
    x = rng.standard_normal(n)
    b, a = signal.iirpeak(center, q, fs=SR)
    x = signal.lfilter(b, a, x)
    env = np.zeros(n)
    s = seconds(start_ms)
    length = seconds(length_ms)
    rise = max(1, seconds(rise_ms))
    env[s:s + rise] = 0.5 - 0.5 * np.cos(np.linspace(0, math.pi, rise))
    env[s + rise:s + length] = np.exp(-np.arange(length - rise) / SR / (length_ms / 1000 / 4))
    return amp * x * env / (np.abs(x).max() + 1e-9)


def room(x, tail_ms, level_db, tone_hz=5000, seed=7):
    """Convolves with a short decaying noise burst: a small, soft room."""
    n = seconds(tail_ms)
    rng = np.random.default_rng(seed)
    ir = rng.standard_normal(n) * decay(n, tail_ms / 4)
    b, a = signal.butter(2, tone_hz, fs=SR)
    ir = signal.lfilter(b, a, ir)
    ir /= np.sqrt((ir ** 2).sum()) + 1e-9
    wet = signal.fftconvolve(x, ir)[: len(x)]
    return x + wet * 10 ** (level_db / 20)


def lufs(x):
    """ITU-R BS.1770 K-weighted loudness of a mono signal over its whole length."""
    b1 = [1.53512485958697, -2.69169618940638, 1.19839281085285]
    a1 = [1.0, -1.69065929318241, 0.73248077421585]
    b2 = [1.0, -2.0, 1.0]
    a2 = [1.0, -1.99004745483398, 0.99007225036621]
    z = signal.lfilter(b2, a2, signal.lfilter(b1, a1, x))
    return -0.691 + 10 * math.log10((z ** 2).mean() + 1e-20)


def finish(x, length_ms):
    """Trims, normalises to the target loudness, keeps the peak under -1 dBFS and fades the edges."""
    n = seconds(length_ms)
    x = x[:n] if len(x) >= n else np.pad(x, (0, n - len(x)))
    x = x * 10 ** ((TARGET_LUFS - lufs(x)) / 20)
    peak = np.abs(x).max()
    limit = 10 ** (-1 / 20)
    if peak > limit:
        x = np.tanh(x / peak * 1.4) / math.tanh(1.4) * limit
    edge = seconds(1.5)
    x[:edge] *= np.linspace(0, 1, edge)
    x[-edge:] *= np.linspace(1, 0, edge)
    return x


def write(path, x):
    import wave
    rng = np.random.default_rng(3)
    dither = (rng.random(len(x)) - rng.random(len(x))) / 32768
    pcm = np.clip(np.round((x + dither) * 32767), -32768, 32767).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def tick():
    n = seconds(60)
    x = partial(n, 640, 9, 1.0, glide=1.3, attack_ms=0.6)
    x += partial(n, 215, 13, 0.55, glide=1.08, attack_ms=0.8)
    x += noise_band(n, 2400, 1.5, 3.0, 0.32, seed=11)
    x = room(x, 18, -22, tone_hz=4000)
    return finish(x, 40)


def mallet(n, f0, taus, amps, glide, attack_ms):
    x = np.zeros(n)
    for ratio, tau, amp in zip((1, 2, 4, 10), taus, amps):
        x += partial(n, f0 * ratio, tau, amp, glide=glide, attack_ms=attack_ms)
    return x


def open_():
    n = seconds(220)
    x = mallet(n, 196.0, (70, 45, 30, 12), (1.0, 0.35, 0.5, 0.15), 1.0, 3.0)
    x += noise_band(n, 1200, 1.0, 4.0, 0.25, seed=21)
    x += noise_band(n, 3000, 0.7, 90.0, 0.07, rise_ms=35, start_ms=8, seed=22)
    x = room(x, 70, -20)
    return finish(x, 170)


def close_():
    n = seconds(240)
    x = mallet(n, 146.83, (85, 50, 32, 10), (1.0, 0.4, 0.4, 0.08), 1.015, 4.0)
    x += noise_band(n, 900, 1.0, 4.0, 0.18, seed=31)
    x = room(x, 80, -19, tone_hz=3500)
    return finish(x, 190)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
    total = 0
    for name, render in (("ui_tick", tick), ("ui_open", open_), ("ui_close", close_)):
        x = render()
        path = os.path.join(out, f"{name}.wav")
        write(path, x)
        size = os.path.getsize(path)
        total += size
        print(f"{name}: {len(x) / SR * 1000:.0f} ms, {lufs(x):.1f} LUFS, peak {20 * math.log10(np.abs(x).max()):.1f} dBFS, {size} bytes")
    print(f"total {total} bytes")


if __name__ == "__main__":
    main()
