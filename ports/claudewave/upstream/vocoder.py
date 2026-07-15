"""
18-band channel vocoder + a chord-following carrier generator.

Use the vocals stem as modulator, and a stacked-saw chord carrier (built from
the song's chord progression) as carrier. The output is a synth that "sings"
the lyrics — classic vocoder sound, heard on countless vaporwave/citypop tracks.
"""
import math, random
import numpy as np
from scipy import signal

from .synths import chord_voicing_midi, midi_to_hz


def render_vocoder_carrier(bar_chords, total_s, sr, pitch_mult=1.0, base_midi=48):
    """Continuous chord carrier: two-octave stacked-saw voicings that hold
    each chord for its full bar, with ±7-cent detune and stereo spread.
    NO attack/release envelope — must be steady for vocoder to modulate.

    bar_chords: list of (start_s, end_s, chord_name)
    Returns stereo (n, 2) float32.
    """
    n_total = int(total_s*sr)
    L = np.zeros(n_total, dtype=np.float32); R = np.zeros(n_total, dtype=np.float32)

    for (bs, be, name) in bar_chords:
        if bs >= total_s: break
        if name is None: continue
        pitches = chord_voicing_midi(name, base_midi=base_midi)
        if not pitches: continue
        # two-octave stack for broad harmonic content (better vocoder bands)
        pitches = pitches + [p + 12 for p in pitches]

        bs_i = int(bs*sr); be_i = min(n_total, int((be+0.2)*sr))
        n_bar = be_i - bs_i
        if n_bar <= 0: continue
        t_bar = np.arange(n_bar)/sr
        xfade = int(0.08*sr)
        env = np.ones(n_bar, dtype=np.float32)
        if n_bar > 2*xfade:
            env[:xfade] = np.linspace(0,1,xfade)
            env[-xfade:] = np.linspace(1,0,xfade)

        for p in pitches:
            f = midi_to_hz(p, slow=pitch_mult)
            for det_cents, pan in [(-7, -0.3), (+7, +0.3), (0, 0.0)]:
                fd = f * 2**(det_cents/1200.0)
                ph = random.random()
                saw = 2.0*((t_bar*fd + ph) - np.floor(t_bar*fd + ph + 0.5))
                gl = math.cos((pan+1)*math.pi/4)
                gr = math.sin((pan+1)*math.pi/4)
                L[bs_i:be_i] += (saw * env * gl * 0.1).astype(np.float32)
                R[bs_i:be_i] += (saw * env * gr * 0.1).astype(np.float32)

    # gentle low-pass to tame the top end before vocoder bands
    sos = signal.butter(2, 10000/(sr/2), btype='low', output='sos')
    L = signal.sosfilt(sos, L).astype(np.float32)
    R = signal.sosfilt(sos, R).astype(np.float32)
    return np.stack([L, R], axis=1)


def channel_vocoder(modulator, carrier, sr, n_bands=18, fmin=80.0, fmax=9000.0,
                    env_lp_hz=30.0, band_gain=5.0):
    """Classic channel vocoder.
    modulator: stereo vocals (n, 2)
    carrier:   stereo (n, 2) — usually the stacked-saw chord carrier above
    Returns stereo modulated carrier."""
    mod_mono = modulator.mean(axis=1)
    bands = np.geomspace(fmin, fmax, n_bands+1)
    out = np.zeros_like(carrier, dtype=np.float32)
    sos_env = signal.butter(2, env_lp_hz/(sr/2), btype='low', output='sos')

    for i in range(n_bands):
        lo, hi = bands[i], bands[i+1]
        sos = signal.butter(4, [lo/(sr/2), min(hi/(sr/2), 0.99)],
                            btype='band', output='sos')
        voc_band = signal.sosfilt(sos, mod_mono).astype(np.float32)
        env = signal.sosfilt(sos_env, np.abs(voc_band)).astype(np.float32)
        env = env * band_gain
        car_l = signal.sosfilt(sos, carrier[:, 0]).astype(np.float32)
        car_r = signal.sosfilt(sos, carrier[:, 1]).astype(np.float32)
        out[:, 0] += car_l * env
        out[:, 1] += car_r * env
    return out


def ring_modulate(y, sr, freq_hz=82.0, mix=0.25):
    """Ring modulation: multiply by a sine carrier. mix=0 → dry, mix=1 → all ring.
    Around 60-120 Hz gives a metallic buzz, >500 Hz starts to sound inharmonic.
    """
    t = np.arange(y.shape[0])/sr
    mod = np.sin(2*np.pi*freq_hz*t).astype(np.float32)[:, None]
    return (y * (1 - mix) + y * mod * mix).astype(np.float32)
