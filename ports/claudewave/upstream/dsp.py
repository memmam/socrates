"""
DSP helpers for claudewave: tape warmth, sidechain pump, vinyl crackle,
hiss, audio I/O, slowdown.
"""
import math, random
import numpy as np
import soundfile as sf
from scipy import signal


def load_stereo(path, target_sr=44100):
    """Load any audio as stereo float32 at target SR."""
    y, sr = sf.read(path, always_2d=True)
    if sr != target_sr:
        y = signal.resample(y, int(round(y.shape[0]*target_sr/sr)), axis=0)
    if y.shape[1] == 1:
        y = np.repeat(y, 2, axis=1)
    return y.astype(np.float32)


def resample_rate(y, ratio):
    """Tape-style slow (or speedup): changes playback rate, which also drops pitch.

    ratio < 1 → slower + lower pitch (classic vaporwave)
    ratio > 1 → faster + higher pitch (future funk)
    """
    n_new = int(round(y.shape[0] / ratio))
    out = np.zeros((n_new, y.shape[1]), dtype=np.float32)
    for c in range(y.shape[1]):
        out[:, c] = signal.resample(y[:, c], n_new).astype(np.float32)
    return out


def rms_normalize(y, target_db=-18.0):
    rms = np.sqrt(np.mean(y**2) + 1e-12)
    return y * (10**(target_db/20.0) / (rms + 1e-12))


def tape_wobble(y, sr, rate_hz=0.22, depth=32.0, flutter_rate=5.0, flutter_depth=5.0):
    """Cassette wow+flutter via sample-position modulation.
    rate/depth are for the slower wow, flutter_* for the faster flutter."""
    n = y.shape[0]
    t = np.arange(n) / sr
    mod = depth*np.sin(2*np.pi*rate_hz*t + 0.9) + flutter_depth*np.sin(2*np.pi*flutter_rate*t + 0.2)
    idx = np.clip(np.arange(n) + mod, 0, n-2)
    i0 = idx.astype(np.int64); frac = (idx - i0).astype(np.float32)[:, None]
    return (y[i0]*(1-frac) + y[np.minimum(i0+1, n-1)]*frac).astype(np.float32)


def sidechain_pump(y, sr, bpm=90, depth=0.15):
    """Synthwave-style breathing: gentle volume dip on every beat, smooth recovery."""
    beat = 60.0/bpm
    t = np.arange(y.shape[0])/sr
    phase = (t / beat) - np.floor(t / beat)
    env = np.where(phase < 0.08,
                   (1.0 - depth) + (depth*phase/0.08),
                   1.0 - depth * np.exp(-(phase-0.08)*6))
    return (y * env[:, None]).astype(np.float32)


def vinyl_crackle(dur_s, sr, density=90.0, level=0.03):
    """Vinyl crackle layer: stereo, mono-equal-both-channels.

    density: pops per second. level: peak amplitude per pop.
    """
    n = int(dur_s * sr)
    out = np.zeros(n, dtype=np.float32)
    for _ in range(int(density * dur_s)):
        p = random.randint(0, n-3)
        a = random.uniform(0.2, 1.0) * level
        out[p]   += a * random.choice([-1, 1])
        out[p+1] += a * 0.3 * random.choice([-1, 1])
    hiss = np.random.randn(n).astype(np.float32) * level * 0.25
    sos = signal.butter(2, [500/(sr/2), 6000/(sr/2)], btype='band', output='sos')
    hiss = signal.sosfilt(sos, hiss).astype(np.float32)
    mono = out + hiss
    return np.stack([mono, mono], axis=1)


def cassette_hiss(dur_s, sr, level=0.010):
    """Soft band-limited tape hiss, stereo-equal."""
    n = int(dur_s*sr)
    h = np.random.randn(n).astype(np.float32) * level
    sos = signal.butter(2, [1200/(sr/2), 8000/(sr/2)], btype='band', output='sos')
    h = signal.sosfilt(sos, h).astype(np.float32)
    return np.stack([h, h], axis=1)


def pad_to(y, n):
    """Zero-pad stereo array up to n samples."""
    return np.pad(y, ((0, max(0, n-y.shape[0])),(0,0)))[:n]


def mix_fades(mix, sr, fade_in_s=2.5, fade_out_s=6.0):
    """Apply linear fade-in / fade-out to an already-mixed stereo track."""
    n = mix.shape[0]
    env = np.ones(n, dtype=np.float32)
    fi = int(fade_in_s*sr); fo = int(fade_out_s*sr)
    if fi > 0: env[:fi] = np.linspace(0, 1, fi)
    if fo > 0 and fo < n: env[-fo:] = np.linspace(1, 0, fo)
    return (mix * env[:, None]).astype(np.float32)


def limit_peak(y, target_peak=0.97):
    """Hard-clip guard: if peak > target, divide so peak == target."""
    peak = float(np.max(np.abs(y)))
    if peak > target_peak:
        return y / peak * target_peak
    return y
