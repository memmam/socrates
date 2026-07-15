"""
Programmatic synth voices for claudewave remixes.

All voices return mono float32 samples in approximately [-1, 1] range,
EXCEPT voice_juno_pad which returns stereo (n, 2). Each is designed so you
can place many of them on a timeline and sum them together.

Important: before calling midi_to_hz, wrap it with your slowdown ratio so
synth notes stay in tune with slowed vocals:

    def midi_to_hz(m, slow=1.0): return 440.0 * 2**((m-69)/12.0) * slow
"""
import math, random
import numpy as np
from scipy import signal


def env_adsr(n, sr, a_s, d_s, s_lvl, r_s):
    a = int(a_s*sr); d = int(d_s*sr); r = int(r_s*sr)
    s = max(0, n - (a+d+r))
    env = np.concatenate([
        np.linspace(0,1,a,dtype=np.float32),
        np.linspace(1,s_lvl,d,dtype=np.float32),
        np.full(s, s_lvl, dtype=np.float32),
        np.linspace(s_lvl,0,r,dtype=np.float32),
    ])
    if env.shape[0] < n: env = np.pad(env, (0, n-env.shape[0]))
    return env[:n]


# ====================== Chord / melody instruments ======================

def voice_rhodes(freq, dur_s, sr, velocity=0.75):
    """DX7-style FM Rhodes. 2-op FM with decaying index (bell attack, mellow sustain)."""
    n = int(dur_s*sr)
    if n < 64: return np.zeros(n, dtype=np.float32)
    t = np.arange(n)/sr
    idx = velocity*3.0*np.exp(-t*4.5) + 0.4*velocity*np.exp(-t*0.8)
    mod = np.sin(2*np.pi*freq*t)
    vib = 0.003*np.sin(2*np.pi*5.5*t)*(1 - np.exp(-t*2.0))
    y = np.sin(2*np.pi*freq*(1+vib)*t + idx*mod)
    amp = (0.75 + 0.25*np.exp(-t*0.4)) * np.exp(-t*1.3)
    return (y*amp*velocity*0.9).astype(np.float32)


def voice_slap_bass(freq, dur_s, sr, velocity=0.8):
    """Funk/citypop bass: sub sine + filtered saw + click transient, tanh-saturated."""
    n = int(dur_s*sr)
    if n < 64: return np.zeros(n, dtype=np.float32)
    t = np.arange(n)/sr
    click_env = np.exp(-t*80)
    click = (np.random.randn(n)*0.5 + np.sin(2*np.pi*freq*2*t)*0.35).astype(np.float32) * click_env
    sos = signal.butter(2, [200/(sr/2), 2500/(sr/2)], btype='band', output='sos')
    click = signal.sosfilt(sos, click).astype(np.float32)
    sub = np.sin(2*np.pi*freq*t + 0.05*np.sin(2*np.pi*freq*0.5*t))
    saw = 2.0*((t*freq) - np.floor(t*freq + 0.5))
    sos2 = signal.butter(2, 800/(sr/2), btype='low', output='sos')
    saw = signal.sosfilt(sos2, saw).astype(np.float32)
    env = env_adsr(n, sr, 0.003, 0.08, 0.55, min(0.35, dur_s*0.3))
    y = (0.55*sub + 0.28*saw + 0.5*click) * env
    y = np.tanh(y*1.3) * 0.82
    return (y*velocity*0.7).astype(np.float32)


def voice_sub(freq, dur_s, sr, velocity=0.7):
    """Deep sine with gentle self-modulation. For vaporwave/slushwave foundation."""
    n = int(dur_s*sr)
    if n < 64: return np.zeros(n, dtype=np.float32)
    t = np.arange(n)/sr
    y = np.sin(2*np.pi*freq*t + 0.08*np.sin(2*np.pi*freq*0.5*t))
    env = env_adsr(n, sr, 0.05, 0.3, 0.6, min(0.8, dur_s*0.5))
    return (y*env*velocity*0.5).astype(np.float32)


def voice_bell(freq, dur_s, sr, velocity=0.5):
    """Glassy bell — sine with 2nd and 3rd harmonics, fast decay."""
    n = int(dur_s*sr)
    if n < 64: return np.zeros(n, dtype=np.float32)
    t = np.arange(n)/sr
    y = 0.7*np.sin(2*np.pi*freq*t) + 0.4*np.sin(2*np.pi*freq*2*t) + 0.2*np.sin(2*np.pi*freq*3*t)
    env = np.exp(-t*2.0)
    return (y*env*velocity*0.25).astype(np.float32)


def voice_juno_pad(freq, dur_s, sr, velocity=0.7, n_voices=5, spread_cents=12.0):
    """Warm detuned-saw pad, low-passed, stereo spread. RETURNS STEREO (n, 2)."""
    n = int(dur_s*sr)
    if n < 64: return np.zeros((n,2), dtype=np.float32)
    t = np.arange(n)/sr
    L = np.zeros(n, dtype=np.float32); R = np.zeros(n, dtype=np.float32)
    for i in range(n_voices):
        cents = (i-(n_voices-1)/2)*(spread_cents*2.0/(n_voices-1))
        f = freq * 2.0**(cents/1200.0); ph = random.random()
        saw = 2.0*((t*f+ph)-np.floor(t*f+ph+0.5))
        sine = np.sin(2*np.pi*f*t+ph)*0.22
        v = (saw + sine).astype(np.float32)
        pan = (i-(n_voices-1)/2) / max(1,(n_voices-1)/2)
        gl = math.cos((pan+1)*math.pi/4); gr = math.sin((pan+1)*math.pi/4)
        L += v*gl; R += v*gr
    s = np.stack([L, R], axis=1) / n_voices
    sos = signal.butter(2, 2800/(sr/2), btype='low', output='sos')
    s = signal.sosfilt(sos, s, axis=0).astype(np.float32)
    env = env_adsr(n, sr, 0.9, 0.7, 0.72, min(2.5, dur_s*0.35))
    return (s*env[:,None]*velocity*0.32).astype(np.float32)


def voice_fm_lead(freq, dur_s, sr, velocity=0.8):
    """Bright 2:1 FM lead for citypop countermelodies."""
    n = int(dur_s*sr)
    if n < 64: return np.zeros(n, dtype=np.float32)
    t = np.arange(n)/sr
    idx = velocity*2.5*np.exp(-t*2.5) + 0.5*velocity
    mod = np.sin(2*np.pi*freq*2.0*t)
    y = np.sin(2*np.pi*freq*t + idx*mod)
    amp = env_adsr(n, sr, 0.06, 0.2, 0.65, min(0.6, dur_s*0.4))
    return (y*amp*velocity*0.45).astype(np.float32)


def voice_whistle(freq, dur_s, sr, velocity=0.8):
    """Natural-feeling whistle.
    Pitch wobble is the sum of 3 randomized-per-note slow oscillators +
    smoothed noise (NOT a clean LFO — which would sound mechanical). Vibrato
    onset is delayed and depth varies over time. Continuous breath noise
    masks the pure-sine character."""
    n = int(dur_s*sr)
    if n < 64: return np.zeros(n, dtype=np.float32)
    t = np.arange(n)/sr

    rate_a = random.uniform(4.6, 6.2)
    rate_b = random.uniform(2.3, 3.5)
    rate_c = random.uniform(0.8, 1.4)
    ph_a, ph_b, ph_c = [random.uniform(0, 2*np.pi) for _ in range(3)]
    osc_a = np.sin(2*np.pi*rate_a*t + ph_a)
    osc_b = np.sin(2*np.pi*rate_b*t + ph_b)
    osc_c = np.sin(2*np.pi*rate_c*t + ph_c)

    # smoothed noise wobble for organic irregularity
    raw = np.random.randn(n).astype(np.float32)
    sos_n = signal.butter(2, 7/(sr/2), btype='low', output='sos')
    noise_wobble = signal.sosfilt(sos_n, raw).astype(np.float32)
    noise_wobble = noise_wobble / (np.max(np.abs(noise_wobble)) + 1e-9)

    onset_delay = random.uniform(0.30, 0.55)
    onset_rise = 0.25
    vib_env = np.clip((t - onset_delay) / onset_rise, 0.0, 1.0)
    depth_mod = 0.55 + 0.45 * (0.5 + 0.5*np.sin(2*np.pi*0.8*t + random.uniform(0, 2*np.pi)))
    pitch_wobble = vib_env * depth_mod * (
        0.004*osc_a + 0.002*osc_b + 0.0015*osc_c + 0.003*noise_wobble
    )
    fixed_offset = random.uniform(-0.004, 0.004)
    freq_t = freq * (1 + pitch_wobble + fixed_offset)

    # integrate freq → phase (time-varying pitch done right)
    dt = 1.0/sr
    phase = 2*np.pi * np.cumsum(freq_t) * dt
    y_sine = np.sin(phase) + 0.025 * np.sin(2*phase)

    # continuous breath noise, band-limited
    bnoise = np.random.randn(n).astype(np.float32)
    sos_b = signal.butter(3, [1800/(sr/2), 4800/(sr/2)], btype='band', output='sos')
    breath = signal.sosfilt(sos_b, bnoise).astype(np.float32)
    breath_env = 0.055 * (0.35 + 0.65*np.exp(-t*8.0))

    # amplitude: soft attack + slow random wobble (not sine tremolo)
    amp = env_adsr(n, sr, a_s=random.uniform(0.06, 0.12), d_s=0.15,
                   s_lvl=random.uniform(0.78, 0.90), r_s=min(0.22, dur_s*0.25))
    amp_raw = np.random.randn(n).astype(np.float32)
    amp_wobble = signal.sosfilt(signal.butter(2, 1.3/(sr/2), btype='low', output='sos'),
                                amp_raw).astype(np.float32)
    amp_wobble = 1.0 + 0.04 * amp_wobble / (np.max(np.abs(amp_wobble))+1e-9)

    y = (y_sine + breath*breath_env) * amp * amp_wobble * velocity * 0.36
    return y.astype(np.float32)


# ====================== Chord voicing helpers ======================

ROOTS = {"C":0,"C#":1,"Db":1,"D":2,"D#":3,"Eb":3,"E":4,"F":5,"F#":6,"Gb":6,
         "G":7,"G#":8,"Ab":8,"A":9,"A#":10,"Bb":10,"B":11}


def parse_chord(name):
    """Return (root_pitch_class 0-11, kind in {'maj','m','7','m7','maj7'}) or (None, None)."""
    if name is None: return None, None
    base = name; kind = ""
    for sx in ("maj7","m7","7","m"):
        if base.endswith(sx):
            kind = sx; base = base[:-len(sx)]; break
    if base not in ROOTS: return None, None
    return ROOTS[base], kind or "maj"


def chord_root_midi(name, octave_midi=33):
    """Root of chord at a given base octave (default 33 = A1)."""
    r, _ = parse_chord(name)
    if r is None: return None
    return octave_midi + r


def chord_voicing_midi(name, base_midi=60):
    """Return a chord voicing in MIDI (standard close-ish, for Rhodes comp)."""
    r, kind = parse_chord(name)
    if r is None: return []
    if kind == "maj7":  degs = [r, (r+4)%12, (r+7)%12, (r+11)%12]
    elif kind == "m7":  degs = [r, (r+3)%12, (r+7)%12, (r+10)%12]
    elif kind == "7":   degs = [r, (r+4)%12, (r+7)%12, (r+10)%12]
    elif kind == "m":   degs = [r, (r+3)%12, (r+7)%12]
    else:               degs = [r, (r+4)%12, (r+7)%12]
    return [base_midi + d for d in degs]


def chord_pad_voicing_midi(name, base_midi=48, top_octave=True):
    """Wider voicing for pads — adds extensions + top octave."""
    r, kind = parse_chord(name)
    if r is None: return []
    if kind == "maj7": degs = [r, (r+4)%12, (r+7)%12, (r+11)%12, (r+2)%12]
    elif kind == "m7": degs = [r, (r+3)%12, (r+7)%12, (r+10)%12, (r+2)%12]
    elif kind == "7":  degs = [r, (r+4)%12, (r+7)%12, (r+10)%12]
    elif kind == "m":  degs = [r, (r+3)%12, (r+7)%12]
    else:              degs = [r, (r+4)%12, (r+7)%12]
    out = [base_midi + d for d in degs]
    if top_octave:
        out.append(base_midi + 12 + degs[0])
    return out


def midi_to_hz(m, slow=1.0):
    """MIDI note → Hz, optionally scaled by a slowdown ratio so synth pitches
    match slowed vocals. Use slow = your SLOW_RATIO (e.g. 0.90)."""
    return 440.0 * 2**((m-69)/12.0) * slow
