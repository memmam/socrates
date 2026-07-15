"""
Synthesized 'field recording' layers — no soundfonts needed.
Crickets, water lapping, wind, etc. Generated directly in numpy.
"""
import random
import numpy as np
from scipy import signal


def render_water(dur_s, sr, lap_rate=0.15, noise_band=(120, 1500), mix_level=0.12):
    """Slow amplitude-modulated band-passed noise — sounds like distant
    water lapping. Use at ~0.1-0.2 mix level."""
    n = int(dur_s*sr)
    noise = np.random.randn(n).astype(np.float32)
    sos = signal.butter(2, [noise_band[0]/(sr/2), noise_band[1]/(sr/2)],
                        btype='band', output='sos')
    water = signal.sosfilt(sos, noise).astype(np.float32)
    t = np.arange(n)/sr
    # slow double-sinusoidal envelope so it doesn't feel periodic
    env = (0.5 + 0.5*np.sin(2*np.pi*lap_rate*t)) * (0.5 + 0.5*np.sin(2*np.pi*(lap_rate*2.1)*t))
    return (water * env * mix_level).astype(np.float32)


def render_crickets(dur_s, sr, chirp_rate_hz=4.0, chirp_freq_band=(4200, 6200),
                    chirp_duration_ms=30, p_chirp=0.75, level=0.06):
    """Rhythmic cricket chirps: filtered noise bursts at chirp_rate with
    random skipping (p_chirp = probability each slot fires)."""
    n = int(dur_s*sr)
    cnoise = np.random.randn(n).astype(np.float32)
    sos = signal.butter(3, [chirp_freq_band[0]/(sr/2), chirp_freq_band[1]/(sr/2)],
                        btype='band', output='sos')
    cnoise = signal.sosfilt(sos, cnoise).astype(np.float32)
    out = np.zeros(n, dtype=np.float32)
    period = int(sr / chirp_rate_hz)
    chirp_dur = int(chirp_duration_ms * 1e-3 * sr)
    for i in range(0, n, period):
        if random.random() < p_chirp:
            end = min(n, i + chirp_dur)
            tt = np.arange(end-i)/sr
            env = np.exp(-tt*80) * (1 - np.exp(-tt*400))
            out[i:end] += cnoise[i:end] * env * random.uniform(0.4, 1.0)
    return (out * level).astype(np.float32)


def render_wind(dur_s, sr, rate=0.15, level=0.04):
    """Slow low-passed noise with amplitude modulation — distant wind."""
    n = int(dur_s*sr)
    noise = np.random.randn(n).astype(np.float32)
    sos = signal.butter(2, 800/(sr/2), btype='low', output='sos')
    w = signal.sosfilt(sos, noise).astype(np.float32)
    t = np.arange(n)/sr
    env = (0.4 + 0.6 * (0.5 + 0.5*np.sin(2*np.pi*rate*t + 0.7)))
    return (w * env * level).astype(np.float32)


def stereo(mono, pan=0.0):
    """Mono → stereo with equal-power pan."""
    import math
    gl = math.cos((pan+1)*math.pi/4)
    gr = math.sin((pan+1)*math.pi/4)
    return np.stack([mono*gl, mono*gr], axis=1)
