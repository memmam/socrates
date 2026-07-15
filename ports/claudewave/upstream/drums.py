"""
Synthesized drum kit for claudewave — kick, brush snare, shaker, hi-hat, congas.

All voices return mono float32 arrays. Use render_drums_on_beats() to place
hits on a beat grid (from librosa.beat.beat_track).
"""
import math, random
import numpy as np
from scipy import signal


def voice_kick(sr, f_start=105, f_end=52, dur_s=0.30):
    n = int(dur_s*sr); t = np.arange(n)/sr
    pitch = f_end + (f_start-f_end)*np.exp(-t*25.0)
    y = np.sin(2*np.pi*np.cumsum(pitch)/sr)
    click = np.zeros(n); click[:90] = np.random.randn(90)*0.4
    amp = np.exp(-t*8.0)
    return (y*amp + click*np.exp(-t*90)*0.3).astype(np.float32)*0.75


def voice_brush_snare(sr, dur_s=0.22):
    n = int(dur_s*sr); t = np.arange(n)/sr
    noise = np.random.randn(n).astype(np.float32)
    sos = signal.butter(2, [250/(sr/2), 5500/(sr/2)], btype='band', output='sos')
    body = signal.sosfilt(sos, noise).astype(np.float32)
    tone = np.sin(2*np.pi*210*t) * 0.22
    amp = np.exp(-t*14.0)
    return (body*amp + tone*amp*0.45).astype(np.float32)*0.5


def voice_snap(sr, dur_s=0.12):
    n = int(dur_s*sr); t = np.arange(n)/sr
    noise = np.random.randn(n).astype(np.float32)
    sos = signal.butter(2, [1200/(sr/2), 8000/(sr/2)], btype='band', output='sos')
    y = signal.sosfilt(sos, noise).astype(np.float32)
    env = np.exp(-t*50.0)
    return (y*env*0.6).astype(np.float32)


def voice_shaker(sr, dur_s=0.08):
    n = int(dur_s*sr); t = np.arange(n)/sr
    noise = np.random.randn(n).astype(np.float32)
    sos = signal.butter(3, [4500/(sr/2), 10000/(sr/2)], btype='band', output='sos')
    y = signal.sosfilt(sos, noise).astype(np.float32)
    env = np.exp(-t*60.0)*(1 - np.exp(-t*550))
    return (y*env*0.24).astype(np.float32)


def voice_hat(sr, dur_s=0.06):
    n = int(dur_s*sr); t = np.arange(n)/sr
    noise = np.random.randn(n).astype(np.float32)
    sos = signal.butter(3, 8500/(sr/2), btype='high', output='sos')
    y = signal.sosfilt(sos, noise).astype(np.float32)
    env = np.exp(-t*130.0)
    return (y*env*0.26).astype(np.float32)


def voice_conga(sr, dur_s=0.16, f=250):
    n = int(dur_s*sr); t = np.arange(n)/sr
    pitch = f + (f*1.5 - f)*np.exp(-t*40)
    y = np.sin(2*np.pi*np.cumsum(pitch)/sr)
    amp = np.exp(-t*20)
    return (y*amp*0.45).astype(np.float32)


def render_drums_on_beats(beat_times, total_s, sr, sections, tight=True):
    """Render a stereo drum track placing hits on the provided beat_times grid.

    sections: list of (start_s, end_s, gain, style) tuples. Style is one of:
      'off'    — no drums
      'light'  — half-time: kick on 1 of bar, snare on 3 of bar, shaker every beat
      'groove' — full: kick on 1&3, snare on 2&4, shaker 8ths, hat on offbeats
      'lift'   — like groove with extra ghost-kick fills

    tight: if False, jitter hits by ±5ms. If the track has swing or rubato,
    set tight=False for human feel; otherwise keep True for citypop/vaporwave.
    """
    n_total = int(total_s*sr)
    L = np.zeros(n_total, dtype=np.float32)
    R = np.zeros(n_total, dtype=np.float32)
    kick = voice_kick(sr); snare = voice_brush_snare(sr)
    shaker = voice_shaker(sr); hat = voice_hat(sr)

    def hit(t, y, pan=0.0, gain=1.0):
        si = int(t*sr); ei = min(n_total, si+y.shape[0])
        gl = math.cos((pan+1)*math.pi/4)*gain
        gr = math.sin((pan+1)*math.pi/4)*gain
        L[si:ei] += y[:ei-si]*gl; R[si:ei] += y[:ei-si]*gr

    def section_at(t):
        for s,e,g,st in sections:
            if s <= t < e: return g, st
        return 0.0, "off"

    for i, t in enumerate(beat_times):
        if t >= total_s: break
        g, style = section_at(t)
        if g <= 0.01 or style == "off": continue
        bib = i % 4
        jitter = 0 if tight else (random.random()-0.5)*0.005
        tj = t + jitter
        hit(tj, shaker, pan=random.uniform(-0.25,0.25), gain=0.60*g)
        if style in ("groove","lift"):
            if i+1 < len(beat_times):
                th = (beat_times[i] + beat_times[i+1]) / 2
                hit(th, shaker, pan=random.uniform(-0.25,0.25), gain=0.45*g)
                if bib in (1, 3):
                    hit(th, hat, pan=random.uniform(-0.15,0.15), gain=0.38*g)
            if bib in (0, 2):
                hit(tj, kick, pan=0.0, gain=0.80*g)
            if bib in (1, 3):
                hit(tj, snare, pan=0.06, gain=0.48*g)
            if style == "lift" and bib == 3 and (i // 4) % 4 == 3 and i+1 < len(beat_times):
                # fill before next bar
                th = (beat_times[i] + beat_times[i+1]) / 2
                hit(th, snare, pan=-0.05, gain=0.35*g)
        elif style == "light":
            if bib == 0:
                hit(tj, kick, pan=0.0, gain=0.68*g)
            if bib == 2:
                hit(tj, snare, pan=0.06, gain=0.42*g)
    return np.stack([L, R], axis=1)
