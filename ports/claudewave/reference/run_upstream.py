#!/usr/bin/env python3
"""
run_upstream.py — render the ground-truth battery from the UNMODIFIED
upstream claudewave lib files, running them against the stdlib-only pynp
shim (ports/pyl/CONTRACT.md).

Usage:
    python3 run_upstream.py OUTDIR

Environment:
    CLAUDEWAVE_SRC     path to the upstream claudewave lib/ directory
                       (default: the vendored byte-for-byte copy in
                       ports/claudewave/upstream/)
    CLAUDEWAVE_STREAM  path to rand_stream.txt (default: alongside this
                       script; generate it with gen_stream.py — it is a
                       runtime artifact, never committed)

What is injected before the upstream modules are imported (the upstream
*.py files themselves are byte-for-byte untouched):
  * sys.modules['numpy']        -> pynp
  * sys.modules['scipy'],
    sys.modules['scipy.signal'] -> stub exposing pynp.butter / pynp.sosfilt
  * sys.modules['soundfile']    -> stub (dsp.py imports it at module level;
                                   load_stereo/resample_rate are not part
                                   of the battery and raise if called)
  * sys.modules['random']       -> stream-fed module per the contract:
        random()      = next unit float u
        uniform(a,b)  = a + (b-a)*u
        randint(a,b)  = a + floor(u*(b-a+1)), clamped to b (inclusive)
        choice(seq)   = seq[min(floor(u*len(seq)), len(seq)-1)]
                        (for the upstream [-1, 1] argument this is exactly
                        the contract's choice_pm1: -1 if u < 0.5 else 1)
    np.random.randn(n) draws pairs from the SAME stream via Box-Muller
    (sqrt(-2*ln(max(u1,1e-300)))*cos(2*pi*u2), sine twin discarded).

Battery item i seeks the shared stream cursor to offset i*100000 before
rendering, so every item is independently reproducible.  Two sample rates
are used: sr=8000 where the upstream filter designs are valid at that
rate, and sr=22050 for the voices whose fixed band edges (up to 10 kHz)
require nyquist > 10025 Hz (butter raises 0 < Wn < 1 otherwise; a faithful
shim must too).

Outputs: OUTDIR/<item>.paw (PAW1 text audio), OUTDIR/chords.txt (chord
helper ground truth), OUTDIR/sos_freeze.txt (every distinct butter design
constructed during the run — the committed copy lives next to this
script), OUTDIR/manifest.txt (per-item summary).
"""

import importlib
import math
import os
import sys
import types
from array import array

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import pynp  # noqa: E402

# The vendored byte-for-byte upstream copy (ports/claudewave/upstream/),
# so CI needs no network fetch; override with CLAUDEWAVE_SRC to run
# against a fresh upstream clone's lib/ directory instead.
DEFAULT_SRC = os.path.join(HERE, '..', 'upstream')

ITEM_STRIDE = 100000  # stream offset per battery item


# -------------------------------------------------------------------
# shared unit-float stream
# -------------------------------------------------------------------

class Stream:
    def __init__(self, path):
        with open(path, 'r') as f:
            self.vals = array('d', (float(line) for line in f))
        if not self.vals:
            raise RuntimeError('empty rand stream: %s' % path)
        self.pos = 0

    def take(self):
        i = self.pos
        if i >= len(self.vals):
            raise RuntimeError('rand stream exhausted at pos=%d' % i)
        self.pos = i + 1
        return self.vals[i]

    def seek(self, off):
        if not 0 <= off <= len(self.vals):
            raise RuntimeError('stream seek out of range: %d' % off)
        self.pos = off


def make_random_module(stream):
    m = types.ModuleType('random')
    m.__doc__ = 'stream-fed replacement for the stdlib random module (pyl contract)'

    def _random():
        return stream.take()

    def _uniform(a, b):
        return a + (b - a) * stream.take()

    def _randint(a, b):
        v = a + int(math.floor(stream.take() * (b - a + 1)))
        return b if v > b else v

    def _choice(seq):
        i = int(stream.take() * len(seq))
        if i >= len(seq):
            i = len(seq) - 1
        return seq[i]

    m.random = _random
    m.uniform = _uniform
    m.randint = _randint
    m.choice = _choice

    def _getattr(name):
        if name.startswith('__') and name.endswith('__'):
            raise AttributeError(name)
        raise NotImplementedError(
            'stream-fed random module: %r is not part of the pyl contract' % name)

    m.__getattr__ = _getattr
    return m


def make_stub_module(name, attrs):
    m = types.ModuleType(name)
    for k, v in attrs.items():
        setattr(m, k, v)

    def _getattr(attr):
        if attr.startswith('__') and attr.endswith('__'):
            raise AttributeError(attr)
        raise NotImplementedError('%s stub: %r not implemented' % (name, attr))

    m.__getattr__ = _getattr
    return m


def _not_implemented(what):
    def f(*a, **k):
        raise NotImplementedError('%s is not part of the shim' % what)
    return f


# -------------------------------------------------------------------
# PAW output (contract's PPM-spirited text audio format)
# -------------------------------------------------------------------

def write_paw(path, arr, sr):
    shape = arr.shape
    n = shape[0]
    ch = shape[1] if len(shape) == 2 else 1
    if ch not in (1, 2):
        raise ValueError('PAW supports 1 or 2 channels, got %d' % ch)
    data = arr.data
    lines = ['PAW1', '%d %d %d' % (sr, ch, n)]
    if ch == 1:
        for v in data:
            lines.append(repr(float(v)))
    else:
        for i in range(n):
            lines.append(repr(float(data[2 * i])) + ' ' + repr(float(data[2 * i + 1])))
    with open(path, 'w') as f:
        f.write('\n'.join(lines))
        f.write('\n')


# -------------------------------------------------------------------
# main
# -------------------------------------------------------------------

def main():
    if len(sys.argv) != 2:
        sys.stderr.write('usage: python3 run_upstream.py OUTDIR\n')
        return 2
    outdir = sys.argv[1]
    os.makedirs(outdir, exist_ok=True)

    src = os.environ.get('CLAUDEWAVE_SRC', DEFAULT_SRC)
    stream_path = os.environ.get('CLAUDEWAVE_STREAM',
                                 os.path.join(HERE, 'rand_stream.txt'))
    if not os.path.isdir(src):
        sys.stderr.write('upstream lib not found: %s (set CLAUDEWAVE_SRC)\n' % src)
        return 2
    if not os.path.isfile(stream_path):
        sys.stderr.write('rand stream not found: %s '
                         '(run gen_stream.py first, or set CLAUDEWAVE_STREAM)\n'
                         % stream_path)
        return 2

    stream = Stream(stream_path)
    pynp.set_rand_source(stream.take)

    # ---- inject the shims, then import the upstream files unmodified ----
    sig = make_stub_module('scipy.signal', {
        'butter': pynp.butter,
        'sosfilt': pynp.sosfilt,
        'resample': _not_implemented('scipy.signal.resample'),
    })
    sci = make_stub_module('scipy', {'signal': sig})
    sf = make_stub_module('soundfile', {
        'read': _not_implemented('soundfile.read'),
        'write': _not_implemented('soundfile.write'),
    })
    sys.modules['numpy'] = pynp
    sys.modules['scipy'] = sci
    sys.modules['scipy.signal'] = sig
    sys.modules['soundfile'] = sf
    sys.modules['random'] = make_random_module(stream)

    pkg = types.ModuleType('cwlib')
    pkg.__path__ = [src]
    sys.modules['cwlib'] = pkg
    synths = importlib.import_module('cwlib.synths')
    drums = importlib.import_module('cwlib.drums')
    dsp = importlib.import_module('cwlib.dsp')
    choppers = importlib.import_module('cwlib.choppers')
    ambience = importlib.import_module('cwlib.ambience')
    vocoder = importlib.import_module('cwlib.vocoder')

    # ---- deterministic helper buffers (no stream draws) ----
    def base_stereo_8k():
        # 0.5 s stereo at 8 kHz: rhodes left, fm lead right (both deterministic)
        return pynp.stack([synths.voice_rhodes(220.0, 0.5, 8000),
                           synths.voice_fm_lead(220.0, 0.5, 8000)], axis=1)

    def chop_base_8k():
        # 1.0 s stereo at 8 kHz: rhodes left, bell right (deterministic)
        return pynp.stack([synths.voice_rhodes(220.0, 1.0, 8000),
                           synths.voice_bell(220.0, 1.0, 8000)], axis=1)

    def mod_stereo_22k():
        # 0.5 s stereo at 22050: rhodes left, fm lead right (deterministic)
        return pynp.stack([synths.voice_rhodes(220.0, 0.5, 22050),
                           synths.voice_fm_lead(330.0, 0.5, 22050)], axis=1)

    BEATS = [0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75]
    SECTIONS = [(0.0, 0.75, 1.0, 'groove'),
                (0.75, 1.5, 0.9, 'lift'),
                (1.5, 2.0, 0.8, 'light')]
    CARRIER_CHORDS_08 = [(0.0, 0.4, 'Am7'), (0.4, 0.8, 'Fmaj7')]
    CARRIER_CHORDS_05 = [(0.0, 0.25, 'Am7'), (0.25, 0.5, 'Fmaj7')]

    def vocoder_channel_item():
        carrier = vocoder.render_vocoder_carrier(CARRIER_CHORDS_05, 0.5, 22050)
        return vocoder.channel_vocoder(mod_stereo_22k(), carrier, 22050)

    # ---- the battery ----------------------------------------------------
    # (name, sr, description, builder). Item i uses stream offset i*100000.
    battery = [
        ('env_adsr', 8000,
         'env_adsr(n=4000, sr=8000, a=0.05, d=0.1, s=0.6, r=0.1); deterministic',
         lambda: synths.env_adsr(4000, 8000, 0.05, 0.1, 0.6, 0.1)),
        ('voice_rhodes_220', 8000,
         'voice_rhodes(220 Hz, 0.5 s); deterministic FM Rhodes',
         lambda: synths.voice_rhodes(220.0, 0.5, 8000)),
        ('voice_sub_220', 8000,
         'voice_sub(220 Hz, 0.5 s); deterministic',
         lambda: synths.voice_sub(220.0, 0.5, 8000)),
        ('voice_bell_220', 8000,
         'voice_bell(220 Hz, 0.5 s); deterministic',
         lambda: synths.voice_bell(220.0, 0.5, 8000)),
        ('voice_fm_lead_220', 8000,
         'voice_fm_lead(220 Hz, 0.5 s); deterministic',
         lambda: synths.voice_fm_lead(220.0, 0.5, 8000)),
        ('voice_slap_bass_110', 8000,
         'voice_slap_bass(110 Hz, 0.5 s); stream-fed (np.random.randn click)',
         lambda: synths.voice_slap_bass(110.0, 0.5, 8000)),
        ('voice_juno_pad_220', 8000,
         'voice_juno_pad(220 Hz, 0.5 s); stereo; stream-fed (per-voice phase)',
         lambda: synths.voice_juno_pad(220.0, 0.5, 8000)),
        ('voice_whistle_880', 22050,
         'voice_whistle(880 Hz, 0.5 s); stream-fed; sr=22050 because its '
         '1800-4800 Hz breath band needs nyquist > 4800',
         lambda: synths.voice_whistle(880.0, 0.5, 22050)),
        ('drum_kick', 22050,
         'drums.voice_kick(sr=22050); stream-fed (randn click)',
         lambda: drums.voice_kick(22050)),
        ('drum_brush_snare', 22050,
         'drums.voice_brush_snare(sr=22050); 250-5500 Hz band needs sr=22050',
         lambda: drums.voice_brush_snare(22050)),
        ('drum_snap', 22050,
         'drums.voice_snap(sr=22050); 1200-8000 Hz band',
         lambda: drums.voice_snap(22050)),
        ('drum_shaker', 22050,
         'drums.voice_shaker(sr=22050); 4500-10000 Hz band',
         lambda: drums.voice_shaker(22050)),
        ('drum_hat', 22050,
         'drums.voice_hat(sr=22050); 8500 Hz order-3 HIGH-pass',
         lambda: drums.voice_hat(22050)),
        ('drum_conga', 22050,
         'drums.voice_conga(sr=22050); deterministic (no filter, no random)',
         lambda: drums.voice_conga(22050)),
        ('drums_on_beats', 22050,
         'render_drums_on_beats(beats=8 x 0.25 s, total 2.0 s, sections '
         'groove/lift/light, tight=False); stereo; stream-fed',
         lambda: drums.render_drums_on_beats(BEATS, 2.0, 22050, SECTIONS,
                                             tight=False)),
        ('dsp_rms_normalize', 8000,
         'rms_normalize(base stereo, -18 dB); deterministic base = '
         'stack(rhodes 220, fm_lead 220) 0.5 s @ 8 kHz',
         lambda: dsp.rms_normalize(base_stereo_8k(), -18.0)),
        ('dsp_tape_wobble', 8000,
         'tape_wobble(base stereo, sr=8000, defaults); deterministic',
         lambda: dsp.tape_wobble(base_stereo_8k(), 8000)),
        ('dsp_sidechain_pump', 8000,
         'sidechain_pump(base stereo, sr=8000, bpm=120, depth=0.3); deterministic',
         lambda: dsp.sidechain_pump(base_stereo_8k(), 8000, bpm=120, depth=0.3)),
        ('dsp_vinyl_crackle', 22050,
         'vinyl_crackle(0.5 s, sr=22050); stream-fed (randint/uniform/choice '
         'pops + randn hiss); 500-6000 Hz band needs sr=22050',
         lambda: dsp.vinyl_crackle(0.5, 22050)),
        ('dsp_cassette_hiss', 22050,
         'cassette_hiss(0.5 s, sr=22050); stream-fed; 1200-8000 Hz band',
         lambda: dsp.cassette_hiss(0.5, 22050)),
        ('dsp_pad_to', 8000,
         'pad_to(base stereo 4000 frames, 6000); deterministic',
         lambda: dsp.pad_to(base_stereo_8k(), 6000)),
        ('dsp_mix_fades', 8000,
         'mix_fades(base stereo, fade_in 0.1 s, fade_out 0.2 s); deterministic',
         lambda: dsp.mix_fades(base_stereo_8k(), 8000, 0.1, 0.2)),
        ('dsp_limit_peak', 8000,
         'limit_peak(base stereo * 5.0, 0.97); deterministic; exercises the '
         'peak > target branch',
         lambda: dsp.limit_peak(base_stereo_8k() * 5.0, 0.97)),
        ('chop_extract_hook', 8000,
         'extract_hook(chop base 1.0 s stereo, 0.2..0.7 s); deterministic',
         lambda: choppers.extract_hook(chop_base_8k(), 0.2, 0.7, 8000)),
        ('chop_place_hook_loops', 8000,
         'place_hook_loops(total 1.0 s, hook from item 23, drops '
         '[(0.0,3,0.15),(0.55,2,0.2)]); deterministic',
         lambda: choppers.place_hook_loops(
             8000, 8000, choppers.extract_hook(chop_base_8k(), 0.2, 0.7, 8000),
             [(0.0, 3, 0.15), (0.55, 2, 0.2)])),
        ('amb_water', 8000,
         'render_water(0.5 s, sr=8000, defaults); stream-fed (randn noise)',
         lambda: ambience.render_water(0.5, 8000)),
        ('amb_crickets', 22050,
         'render_crickets(0.5 s, sr=22050, defaults); stream-fed; '
         '4200-6200 Hz band needs sr=22050',
         lambda: ambience.render_crickets(0.5, 22050)),
        ('amb_wind', 8000,
         'render_wind(0.5 s, sr=8000, defaults); stream-fed',
         lambda: ambience.render_wind(0.5, 8000)),
        ('amb_stereo_pan', 8000,
         'ambience.stereo(voice_bell(220, 0.5 s), pan=0.3); deterministic',
         lambda: ambience.stereo(synths.voice_bell(220.0, 0.5, 8000), pan=0.3)),
        ('vocoder_carrier', 22050,
         'render_vocoder_carrier([(0,0.4,Am7),(0.4,0.8,Fmaj7)], 0.8 s, '
         'sr=22050); stereo; stream-fed (detune phases); 10 kHz low-pass '
         'needs sr=22050',
         lambda: vocoder.render_vocoder_carrier(CARRIER_CHORDS_08, 0.8, 22050)),
        ('vocoder_channel', 22050,
         'channel_vocoder(mod=stack(rhodes 220, fm_lead 330) 0.5 s, '
         'carrier=render_vocoder_carrier([(0,0.25,Am7),(0.25,0.5,Fmaj7)], '
         '0.5 s), sr=22050, 18 geomspace bands 80-9000 Hz, order-4 bandpass)',
         vocoder_channel_item),
        ('vocoder_ring_mod', 22050,
         'ring_modulate(mod stereo, sr=22050, 82 Hz, mix=0.25); deterministic',
         lambda: vocoder.ring_modulate(mod_stereo_22k(), 22050, 82.0, 0.25)),
    ]

    manifest = []
    print('battery: %d items, stream stride %d' % (len(battery), ITEM_STRIDE))
    for idx, (name, sr, desc, build) in enumerate(battery):
        offset = idx * ITEM_STRIDE
        stream.seek(offset)
        arr = build()
        draws = stream.pos - offset
        if draws >= ITEM_STRIDE:
            raise RuntimeError('%s consumed %d draws (>= stride %d)'
                               % (name, draws, ITEM_STRIDE))
        n = arr.shape[0]
        ch = arr.shape[1] if len(arr.shape) == 2 else 1
        lo = min(arr.data) if arr.data else 0.0
        hi = max(arr.data) if arr.data else 0.0
        finite = all(math.isfinite(v) for v in arr.data)
        write_paw(os.path.join(outdir, name + '.paw'), arr, sr)
        line = ('%02d %-24s offset=%-8d sr=%-5d frames=%-6d ch=%d '
                'draws=%-6d min=%.6g max=%.6g finite=%s | %s'
                % (idx, name, offset, sr, n, ch, draws, lo, hi, finite, desc))
        manifest.append(line)
        print(line)
        if not finite:
            raise RuntimeError('%s produced non-finite samples' % name)

    # ---- chord helper ground truth (text; consumes no stream draws) ----
    chords = ['C', 'Am', 'F', 'G7', 'Cmaj7', 'Am7', 'Dm7', 'F#m7', 'Bb',
              'Ebmaj7', 'Db7', 'G#m', 'Xyz', None]
    with open(os.path.join(outdir, 'chords.txt'), 'w') as f:
        for name in chords:
            r, kind = synths.parse_chord(name)
            root = synths.chord_root_midi(name)
            voicing = synths.chord_voicing_midi(name)
            pad = synths.chord_pad_voicing_midi(name)
            hz = [repr(synths.midi_to_hz(m)) for m in voicing]
            hz_slow = [repr(synths.midi_to_hz(m, slow=0.9)) for m in voicing]
            f.write('name=%s parse=(%r, %r) root=%r voicing=%r pad=%r\n'
                    % (name, r, kind, root, voicing, pad))
            f.write('  hz=[%s]\n' % ', '.join(hz))
            f.write('  hz_slow090=[%s]\n' % ', '.join(hz_slow))
    print('chords.txt: %d chords' % len(chords))

    # ---- freeze every distinct butter design constructed in this run ----
    freeze_path = os.path.join(outdir, 'sos_freeze.txt')
    n_designs = 0
    with open(freeze_path, 'w') as f:
        f.write('# FROZEN -- do not hand-edit or casually regenerate. This file is the\n'
                '# binding artifact for ports/pyl/CONTRACT.md section 6: the Socrates port must\n'
                '# reproduce every coefficient below to <=1e-12 relative, and CI checks\n'
                '# every coefficient against real scipy to <=1e-9 relative (via\n'
                '# check_sos_vs_scipy.py) so this dump cannot silently drift from the\n'
                '# real library. Regenerated only by run_upstream.py, from the shim\'s\n'
                '# own butter() implementation (CONTRACT.md steps 1-5) -- one line per\n'
                '# SOS section, `<design-id> b0 b1 b2 1 a1 a2`.\n')
        for design_id, sos in pynp.BUTTER_REGISTRY.values():
            n_designs += 1
            for row in sos:
                b0, b1, b2, a0, a1, a2 = row
                f.write('%s %s %s %s %s %s %s\n'
                        % (design_id, repr(b0), repr(b1), repr(b2),
                           repr(a0), repr(a1), repr(a2)))
    print('sos_freeze.txt: %d distinct designs' % n_designs)

    with open(os.path.join(outdir, 'manifest.txt'), 'w') as f:
        f.write('\n'.join(manifest))
        f.write('\n')
    print('done: %s' % outdir)
    return 0


if __name__ == '__main__':
    sys.exit(main())
