"""
Chop & screw helpers — extract a hook phrase from a vocal stem, then
place hypnotic echoing loops at other moments in the track.

The eccojams / vaporwave move: take a 5-10s phrase (often a chorus line),
and paste 2-5 staggered copies with decaying gain at chorus moments.
"""
import numpy as np


def extract_hook(vocals_slowed, start_s, end_s, sr, fade_in_s=0.1, fade_out_s=0.4):
    """Extract a chunk from the slowed vocals with brief in/out fades.

    Returns a stereo chunk (n, 2) float32 ready to be looped.
    """
    s = int(start_s * sr)
    e = int(end_s   * sr)
    chunk = vocals_slowed[s:e].copy()
    fi = int(fade_in_s*sr); fo = int(fade_out_s*sr)
    env = np.ones(chunk.shape[0], dtype=np.float32)
    if fi > 0: env[:fi] = np.linspace(0, 1, fi)
    if fo > 0: env[-fo:] = np.linspace(1, 0, fo)
    return chunk * env[:, None]


def place_hook_loops(total_n, sr, hook_chunk, drop_points, base_gain=0.75,
                     decay_per_copy=0.7):
    """Paste the hook_chunk at each drop_point with staggered echoing copies.

    drop_points: list of (start_s, n_copies, stagger_s). At each drop:
      - first copy starts at start_s with gain = base_gain
      - each subsequent copy starts stagger_s later, gain *= decay_per_copy

    Returns stereo buffer (total_n, 2) float32 ready to be added to the mix.
    """
    out = np.zeros((total_n, 2), dtype=np.float32)
    for (start_t, n_copies, stagger) in drop_points:
        for i in range(n_copies):
            t = start_t + i * stagger
            gain = base_gain * (decay_per_copy ** i)
            si = int(t * sr)
            ei = min(total_n, si + hook_chunk.shape[0])
            if si >= total_n: break
            out[si:ei] += hook_chunk[:ei-si] * gain
    return out
