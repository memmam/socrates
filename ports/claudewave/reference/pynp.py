"""
pynp — a stdlib-only shim for the numpy + scipy.signal subset used by the
claudewave lib files (synths, drums, dsp, choppers, ambience, vocoder).

This module is injected as sys.modules['numpy'] (and its `butter`/`sosfilt`
are exposed through a scipy/scipy.signal stub) by run_upstream.py, so the
upstream files run UNMODIFIED against it.  Semantics are pinned by
ports/pyl/CONTRACT.md:

  * f64 throughout; .astype(float32) is a copying no-op, .astype(int64)
    truncates toward zero (numpy semantics for non-negative values).
  * Arrays are 1-D (n,), column (n,1) (from x[:, None]) or stereo (n,2).
  * Butterworth designs follow scipy's zpk pipeline exactly
    (buttap -> prewarp 4*tan(pi*Wn/2) -> lp2lp/lp2hp/lp2bp_zpk ->
    bilinear_zpk(fs=2) -> zpk2sos with 'nearest' pairing); the binding
    artifact is the per-design coefficient freeze (sos_freeze.txt), which
    CI cross-checks against real scipy at 1e-9 relative.
  * sosfilt is direct-form II transposed with zero initial state,
    channels filtered independently (axis=0 for 2-D).
  * np.random.randn draws from the shared unit-float stream via
    Box-Muller (sqrt(-2*ln(max(u1,1e-300)))*cos(2*pi*u2)); the stream is
    wired in by set_rand_source().

Anything outside the subset the six files exercise raises
NotImplementedError — fail loudly, never guess.
"""

import builtins as _bi
import cmath as _cmath
import math as _math

_abs = _bi.abs
_max = _bi.max
_min = _bi.min

_F8 = 'f8'
_I8 = 'i8'
_B1 = 'b1'

# dtype tokens (numpy attribute lookalikes; only used as sentinels)
float32 = 'float32'
float64 = 'float64'
int64 = 'int64'

pi = _math.pi

_FLOAT_DTYPES = (float32, float64, _F8)


def _fail(msg):
    raise NotImplementedError('pynp: ' + msg)


# ===================================================================
# ndarray
# ===================================================================

class NDArray:
    """f64 (or i8/bool) array; flat row-major data; shape (n,), (n,1), (n,2)."""
    __slots__ = ('data', 'shape', 'dtype')

    def __init__(self, data, shape, dtype=_F8):
        self.data = data
        self.shape = shape
        self.dtype = dtype

    # ---- misc numpy API -------------------------------------------
    def astype(self, dtype):
        if dtype in _FLOAT_DTYPES:
            if self.dtype == _I8:
                return NDArray([float(v) for v in self.data], self.shape, _F8)
            return NDArray(list(self.data), self.shape, _F8)
        if dtype in (int64, _I8):
            # numpy float->int astype truncates toward zero
            return NDArray([int(v) for v in self.data], self.shape, _I8)
        _fail('astype(%r)' % (dtype,))

    def copy(self):
        return NDArray(list(self.data), self.shape, self.dtype)

    def mean(self, axis=None):
        if axis is None:
            if not self.data:
                _fail('mean of empty array')
            return _bi.sum(self.data) / len(self.data)  # plain f64 accumulation
        if axis == 1 and len(self.shape) == 2:
            n, c = self.shape
            d = self.data
            out = []
            for i in range(n):
                s = 0.0
                base = i * c
                for j in range(c):
                    s += d[base + j]
                out.append(s / c)
            return NDArray(out, (n,), _F8)
        _fail('mean(axis=%r) on shape %r' % (axis, self.shape))

    def conj(self):
        _fail('conj')

    def __len__(self):
        return self.shape[0]

    def __repr__(self):
        return 'pynp.NDArray(shape=%r, dtype=%r)' % (self.shape, self.dtype)

    def __bool__(self):
        raise ValueError('truth value of a pynp array is ambiguous')

    # ---- indexing --------------------------------------------------
    def _row_slice(self, sl):
        start, stop, step = sl.indices(self.shape[0])
        if step != 1:
            _fail('slice step != 1')
        stop = _max(start, stop)
        return start, stop

    def __getitem__(self, key):
        if isinstance(key, tuple):
            if len(key) != 2:
                _fail('getitem tuple of len %d' % len(key))
            k0, k1 = key
            if k1 is None:
                # x[:, None] -> (n, 1)
                if (len(self.shape) == 1 and isinstance(k0, slice)
                        and k0 == slice(None)):
                    return NDArray(list(self.data), (self.shape[0], 1), self.dtype)
                _fail('[:, None] only supported on 1-D with full slice')
            if isinstance(k0, slice) and isinstance(k1, int):
                if len(self.shape) != 2:
                    _fail('column index on non-2-D array')
                start, stop = self._row_slice(k0)
                c = self.shape[1]
                col = k1 + c if k1 < 0 else k1
                if not 0 <= col < c:
                    raise IndexError('column index out of range')
                d = self.data
                out = [d[i * c + col] for i in range(start, stop)]
                return NDArray(out, (len(out),), self.dtype)
            _fail('getitem key %r' % (key,))
        if isinstance(key, slice):
            start, stop = self._row_slice(key)
            n = stop - start
            if len(self.shape) == 1:
                return NDArray(self.data[start:stop], (n,), self.dtype)
            c = self.shape[1]
            return NDArray(self.data[start * c:stop * c], (n, c), self.dtype)
        if isinstance(key, int):
            if len(self.shape) == 1:
                n = self.shape[0]
                i = key + n if key < 0 else key
                if not 0 <= i < n:
                    raise IndexError('index out of range')
                return self.data[i]
            _fail('scalar row index on 2-D array')
        if isinstance(key, NDArray):
            if key.dtype != _I8:
                _fail('fancy indexing requires an int64 index array')
            n = self.shape[0]
            idxs = key.data
            if len(self.shape) == 1:
                d = self.data
                out = []
                for i in idxs:
                    if i < 0:
                        i += n
                    if not 0 <= i < n:
                        raise IndexError('fancy index out of range')
                    out.append(d[i])
                return NDArray(out, (len(idxs),), self.dtype)
            c = self.shape[1]
            d = self.data
            out = []
            for i in idxs:
                if i < 0:
                    i += n
                if not 0 <= i < n:
                    raise IndexError('fancy index out of range')
                base = i * c
                out.extend(d[base:base + c])
            return NDArray(out, (len(idxs), c), self.dtype)
        _fail('getitem key %r' % (key,))

    def __setitem__(self, key, value):
        if isinstance(key, tuple):
            if (len(key) == 2 and isinstance(key[0], slice)
                    and isinstance(key[1], int) and len(self.shape) == 2):
                start, stop = self._row_slice(key[0])
                c = self.shape[1]
                col = key[1] + c if key[1] < 0 else key[1]
                if not 0 <= col < c:
                    raise IndexError('column index out of range')
                n = stop - start
                d = self.data
                if isinstance(value, NDArray):
                    if value.shape != (n,):
                        _fail('column assign shape mismatch %r vs (%d,)'
                              % (value.shape, n))
                    vd = value.data
                    for i in range(n):
                        d[(start + i) * c + col] = float(vd[i])
                    return
                if isinstance(value, (int, float)):
                    v = float(value)
                    for i in range(n):
                        d[(start + i) * c + col] = v
                    return
            _fail('setitem key %r' % (key,))
        if isinstance(key, slice):
            start, stop = self._row_slice(key)
            n = stop - start
            c = self.shape[1] if len(self.shape) == 2 else 1
            if isinstance(value, NDArray):
                vc = value.shape[1] if len(value.shape) == 2 else 1
                if value.shape[0] != n or vc != c:
                    _fail('slice assign shape mismatch %r into %d frames x %d ch'
                          % (value.shape, n, c))
                self.data[start * c:stop * c] = [float(v) for v in value.data]
                return
            if isinstance(value, (int, float)):
                v = float(value)
                self.data[start * c:stop * c] = [v] * (n * c)
                return
            _fail('slice assign from %r' % type(value))
        if isinstance(key, int):
            if len(self.shape) != 1:
                _fail('scalar setitem on 2-D array')
            n = self.shape[0]
            i = key + n if key < 0 else key
            if not 0 <= i < n:
                raise IndexError('index out of range')
            self.data[i] = float(value)
            return
        _fail('setitem key %r' % (key,))

    # ---- arithmetic -------------------------------------------------
    def _bin(self, other, op, int_ok, swapped=False):
        if isinstance(other, NDArray):
            if self.shape == other.shape:
                a, b = (other.data, self.data) if swapped else (self.data, other.data)
                out = [op(x, y) for x, y in zip(a, b)]
                dt = _I8 if (int_ok and self.dtype == _I8 and other.dtype == _I8) else _F8
                return NDArray(out, self.shape, dt)
            # broadcast (n,2) with (n,1)
            if (len(self.shape) == 2 and len(other.shape) == 2
                    and self.shape[0] == other.shape[0]):
                if self.shape[1] == 2 and other.shape[1] == 1:
                    wide, col, wide_first = self, other, True
                elif self.shape[1] == 1 and other.shape[1] == 2:
                    wide, col, wide_first = other, self, False
                else:
                    _fail('broadcast %r with %r' % (self.shape, other.shape))
                if swapped:
                    wide_first = not wide_first
                wd, cd = wide.data, col.data
                out = []
                if wide_first:
                    for i in range(wide.shape[0]):
                        v = cd[i]
                        out.append(op(wd[2 * i], v))
                        out.append(op(wd[2 * i + 1], v))
                else:
                    for i in range(wide.shape[0]):
                        v = cd[i]
                        out.append(op(v, wd[2 * i]))
                        out.append(op(v, wd[2 * i + 1]))
                return NDArray(out, wide.shape, _F8)
            _fail('shape mismatch %r vs %r' % (self.shape, other.shape))
        if isinstance(other, (int, float)):
            if swapped:
                out = [op(other, x) for x in self.data]
            else:
                out = [op(x, other) for x in self.data]
            dt = _I8 if (int_ok and self.dtype == _I8
                         and isinstance(other, int)
                         and not isinstance(other, bool)) else _F8
            return NDArray(out, self.shape, dt)
        return NotImplemented

    def __add__(self, o):
        return self._bin(o, lambda x, y: x + y, True)

    def __radd__(self, o):
        return self._bin(o, lambda x, y: x + y, True, swapped=True)

    def __sub__(self, o):
        return self._bin(o, lambda x, y: x - y, True)

    def __rsub__(self, o):
        return self._bin(o, lambda x, y: x - y, True, swapped=True)

    def __mul__(self, o):
        return self._bin(o, lambda x, y: x * y, True)

    def __rmul__(self, o):
        return self._bin(o, lambda x, y: x * y, True, swapped=True)

    def __truediv__(self, o):
        return self._bin(o, lambda x, y: x / y, False)

    def __rtruediv__(self, o):
        return self._bin(o, lambda x, y: x / y, False, swapped=True)

    def __pow__(self, k):
        if not isinstance(k, (int, float)):
            _fail('array ** %r' % type(k))
        if k == 2:
            return NDArray([v * v for v in self.data], self.shape, _F8)
        return NDArray([_math.pow(v, k) for v in self.data], self.shape, _F8)

    def __neg__(self):
        return NDArray([-v for v in self.data],
                       self.shape, self.dtype if self.dtype == _I8 else _F8)

    def __iadd__(self, other):
        if isinstance(other, NDArray):
            if other.shape != self.shape:
                _fail('+= shape mismatch %r vs %r' % (self.shape, other.shape))
            d = self.data
            od = other.data
            for i in range(len(d)):
                d[i] = d[i] + od[i]
            return self
        if isinstance(other, (int, float)):
            d = self.data
            for i in range(len(d)):
                d[i] = d[i] + other
            return self
        _fail('+= from %r' % type(other))

    def __lt__(self, o):
        if isinstance(o, (int, float)):
            return NDArray([v < o for v in self.data], self.shape, _B1)
        _fail('array < %r' % type(o))

    def __gt__(self, o):
        if isinstance(o, (int, float)):
            return NDArray([v > o for v in self.data], self.shape, _B1)
        _fail('array > %r' % type(o))


def _as1d(x, who):
    if not isinstance(x, NDArray) or len(x.shape) != 1:
        _fail('%s expects a 1-D array' % who)
    return x


# ===================================================================
# constructors
# ===================================================================

def _shape_of(shape):
    if isinstance(shape, int):
        return (shape,)
    if (isinstance(shape, tuple) and len(shape) == 2
            and all(isinstance(v, int) for v in shape)):
        return shape
    _fail('unsupported shape %r' % (shape,))


def zeros(shape, dtype=None):
    sh = _shape_of(shape)
    n = sh[0] * (sh[1] if len(sh) == 2 else 1)
    return NDArray([0.0] * n, sh, _F8)


def zeros_like(a, dtype=None):
    if not isinstance(a, NDArray):
        _fail('zeros_like of %r' % type(a))
    return NDArray([0.0] * len(a.data), a.shape, _F8)


def ones(shape, dtype=None):
    sh = _shape_of(shape)
    n = sh[0] * (sh[1] if len(sh) == 2 else 1)
    return NDArray([1.0] * n, sh, _F8)


def full(n, v, dtype=None):
    if not isinstance(n, int):
        _fail('full with shape %r' % (n,))
    return NDArray([float(v)] * n, (n,), _F8)


def arange(n, *rest):
    if rest:
        _fail('arange with more than one argument')
    n = int(n)
    return NDArray([float(i) for i in range(n)], (n,), _F8)


def linspace(start, stop, num, dtype=None):
    # Contract: endpoint included; num==1 -> [start]; num==0 -> empty;
    # step = (stop-start)/(num-1); out[-1] forced to stop.
    num = int(num)
    if num < 0:
        raise ValueError('linspace num < 0')
    if num == 0:
        return NDArray([], (0,), _F8)
    start = float(start)
    stop = float(stop)
    if num == 1:
        return NDArray([start], (1,), _F8)
    step = (stop - start) / (num - 1)
    out = [start + i * step for i in range(num)]
    out[-1] = stop
    return NDArray(out, (num,), _F8)


def geomspace(start, stop, num):
    # numpy: 10 ** linspace(log10(start), log10(stop), num), endpoints forced.
    num = int(num)
    start = float(start)
    stop = float(stop)
    if num < 2 or start <= 0.0 or stop <= 0.0:
        _fail('geomspace(%r, %r, %r)' % (start, stop, num))
    l0 = _math.log10(start)
    l1 = _math.log10(stop)
    step = (l1 - l0) / (num - 1)
    out = [10.0 ** (l0 + i * step) for i in range(num)]
    out[0] = start
    out[-1] = stop
    return NDArray(out, (num,), _F8)


def concatenate(parts):
    out = []
    for p in parts:
        p = _as1d(p, 'concatenate')
        out.extend(p.data)
    return NDArray(out, (len(out),), _F8)


def pad(a, pad_width, mode='constant'):
    if mode != 'constant':
        _fail('pad mode %r' % (mode,))
    if not isinstance(a, NDArray):
        _fail('pad of %r' % type(a))
    if len(a.shape) == 1:
        if (not isinstance(pad_width, tuple) or len(pad_width) != 2
                or not all(isinstance(v, int) for v in pad_width)):
            _fail('1-D pad width %r' % (pad_width,))
        before, after = pad_width
        if before < 0 or after < 0:
            _fail('negative pad')
        out = [0.0] * before + [float(v) for v in a.data] + [0.0] * after
        return NDArray(out, (len(out),), _F8)
    if len(a.shape) == 2:
        try:
            (r0, r1), (c0, c1) = pad_width
        except (TypeError, ValueError):
            _fail('2-D pad width %r' % (pad_width,))
        if c0 != 0 or c1 != 0:
            _fail('2-D pad on channel axis')
        if r0 < 0 or r1 < 0:
            _fail('negative pad')
        c = a.shape[1]
        out = [0.0] * (r0 * c) + [float(v) for v in a.data] + [0.0] * (r1 * c)
        return NDArray(out, (a.shape[0] + r0 + r1, c), _F8)
    _fail('pad of shape %r' % (a.shape,))


def stack(arrays, axis=0):
    if axis != 1:
        _fail('stack(axis=%r)' % (axis,))
    arrs = [_as1d(a, 'stack') for a in arrays]
    if not arrs:
        _fail('stack of nothing')
    n = arrs[0].shape[0]
    for a in arrs:
        if a.shape[0] != n:
            _fail('stack length mismatch')
    k = len(arrs)
    if k not in (1, 2):
        _fail('stack of %d arrays' % k)
    out = []
    for i in range(n):
        for a in arrs:
            out.append(float(a.data[i]))
    return NDArray(out, (n, k), _F8)


def repeat(a, repeats, axis=None):
    if (isinstance(a, NDArray) and len(a.shape) == 2 and a.shape[1] == 1
            and axis == 1 and isinstance(repeats, int) and repeats >= 1):
        out = []
        for v in a.data:
            out.extend([float(v)] * repeats)
        return NDArray(out, (a.shape[0], repeats), _F8)
    _fail('repeat(%r, %r, axis=%r)' % (getattr(a, 'shape', type(a)), repeats, axis))


# ===================================================================
# ufuncs / reductions
# ===================================================================

def _ufunc(f, name):
    def g(x):
        if isinstance(x, NDArray):
            return NDArray([f(v) for v in x.data], x.shape, _F8)
        if isinstance(x, (int, float)):
            return f(x)
        _fail('%s of %r' % (name, type(x)))
    g.__name__ = name
    return g


sin = _ufunc(_math.sin, 'sin')
cos = _ufunc(_math.cos, 'cos')
exp = _ufunc(_math.exp, 'exp')
sqrt = _ufunc(_math.sqrt, 'sqrt')
tanh = _ufunc(_math.tanh, 'tanh')
floor = _ufunc(lambda v: float(_math.floor(v)), 'floor')
absolute = _ufunc(_abs, 'abs')
abs = absolute  # noqa: A001 — numpy exposes np.abs


def mean(x, axis=None):
    if isinstance(x, NDArray):
        return x.mean(axis=axis)
    _fail('mean of %r' % type(x))


def amax(x):
    if isinstance(x, NDArray):
        if not x.data:
            raise ValueError('max of empty array')
        return _max(x.data)
    _fail('max of %r' % type(x))


max = amax  # noqa: A001 — numpy exposes np.max


def clip(x, lo, hi):
    if isinstance(x, NDArray):
        lo = float(lo)
        hi = float(hi)
        out = [_min(_max(float(v), lo), hi) for v in x.data]
        return NDArray(out, x.shape, _F8)
    _fail('clip of %r' % type(x))


def minimum(a, b):
    if isinstance(a, NDArray) and isinstance(b, (int, float)):
        dt = _I8 if (a.dtype == _I8 and isinstance(b, int)
                     and not isinstance(b, bool)) else _F8
        if dt == _I8:
            out = [v if v < b else b for v in a.data]
        else:
            out = [float(v) if v < b else float(b) for v in a.data]
        return NDArray(out, a.shape, dt)
    if isinstance(a, NDArray) and isinstance(b, NDArray) and a.shape == b.shape:
        dt = _I8 if (a.dtype == _I8 and b.dtype == _I8) else _F8
        out = [x if x < y else y for x, y in zip(a.data, b.data)]
        if dt == _F8:
            out = [float(v) for v in out]
        return NDArray(out, a.shape, dt)
    _fail('minimum(%r, %r)' % (type(a), type(b)))


def where(cond, a, b):
    if not (isinstance(cond, NDArray) and cond.dtype == _B1):
        _fail('where condition must be a boolean array')
    n = len(cond.data)

    def _vals(x):
        if isinstance(x, NDArray):
            if x.shape != cond.shape:
                _fail('where operand shape mismatch')
            return x.data
        if isinstance(x, (int, float)):
            return [float(x)] * n
        _fail('where operand %r' % type(x))

    av = _vals(a)
    bv = _vals(b)
    out = [float(av[i]) if cond.data[i] else float(bv[i]) for i in range(n)]
    return NDArray(out, cond.shape, _F8)


def cumsum(x):
    x = _as1d(x, 'cumsum')
    out = []
    s = 0.0
    for v in x.data:
        s += v  # plain f64 accumulation, left to right
        out.append(s)
    return NDArray(out, x.shape, _F8)


# ===================================================================
# np.random — stream-fed Box-Muller
# ===================================================================

class _RandomShim:
    def __init__(self):
        self._source = None

    def randn(self, n):
        src = self._source
        if src is None:
            raise RuntimeError('pynp: random source not wired '
                               '(call pynp.set_rand_source first)')
        n = int(n)
        out = []
        two_pi = 2.0 * _math.pi
        for _ in range(n):
            u1 = src()
            u2 = src()
            if u1 < 1e-300:
                u1 = 1e-300
            out.append(_math.sqrt(-2.0 * _math.log(u1)) * _math.cos(two_pi * u2))
        return NDArray(out, (n,), _F8)

    def __getattr__(self, name):
        _fail('np.random.%s' % name)


random = _RandomShim()


def set_rand_source(fn):
    """Wire np.random.randn to a unit-float stream (fn() -> next float)."""
    random._source = fn


# ===================================================================
# scipy.signal subset: butter (output='sos') + sosfilt
# ===================================================================
# Faithful pure-Python port of scipy's design pipeline so that CI's
# comparison against real scipy holds to <= 1e-9 relative.

# Every distinct design constructed at runtime is recorded here:
# key (N, btype, (wn...)) -> (design_id, sos rows)   (insertion-ordered)
BUTTER_REGISTRY = {}

_BTYPE_ALIASES = {
    'low': 'lowpass', 'lowpass': 'lowpass', 'lp': 'lowpass', 'l': 'lowpass',
    'high': 'highpass', 'highpass': 'highpass', 'hp': 'highpass', 'h': 'highpass',
    'band': 'bandpass', 'bandpass': 'bandpass', 'bp': 'bandpass', 'pass': 'bandpass',
}


def _isreal(c):
    return c.imag == 0.0


def _nreal(cs):
    n = 0
    for c in cs:
        if _isreal(c):
            n += 1
    return n


def _prod(cs):
    out = complex(1.0, 0.0)
    for c in cs:
        out = out * c
    return out


def _cplxreal(items):
    """scipy.signal._cplxreal: split into (complex pair representatives, reals)."""
    if not items:
        return [], []
    tol = 100.0 * 2.220446049250313e-16  # 100 * eps(float64)
    zs = sorted((complex(c) for c in items),
                key=lambda c: (c.real, _abs(c.imag)))  # lexsort((|imag|, real))
    real_mask = [_abs(c.imag) <= tol * _abs(c) for c in zs]
    zr = [c.real for c, m in zip(zs, real_mask) if m]
    if len(zr) == len(zs):
        return [], zr
    zrest = [c for c, m in zip(zs, real_mask) if not m]
    zp = [c for c in zrest if c.imag > 0]
    zn = [c for c in zrest if c.imag < 0]
    if len(zp) != len(zn):
        raise ValueError('complex value with no matching conjugate')
    # runs of (approximately) equal real part -> sort each by |imag|
    m = len(zp)
    same_real = [(zp[i + 1].real - zp[i].real) <= tol * _abs(zp[i])
                 for i in range(m - 1)]
    i = 0
    while i < m - 1:
        if same_real[i]:
            start = i
            stop = i + 1
            while stop < m - 1 and same_real[stop]:
                stop += 1
            # run covers indices start..stop inclusive
            zp[start:stop + 1] = sorted(zp[start:stop + 1],
                                        key=lambda c: _abs(c.imag))
            zn[start:stop + 1] = sorted(zn[start:stop + 1],
                                        key=lambda c: _abs(c.imag))
            i = stop + 1
        else:
            i += 1
    for a, b in zip(zp, zn):
        if _abs(a - b.conjugate()) > tol * _abs(b):
            raise ValueError('complex value with no matching conjugate')
    zc = [(a + b.conjugate()) / 2.0 for a, b in zip(zp, zn)]
    return zc, zr


def _nearest_rc(fro, to, which):
    """scipy.signal._nearest_real_complex_idx (stable sort; ties keep order)."""
    order = sorted(range(len(fro)), key=lambda i: _abs(fro[i] - to))
    if which == 'any':
        if not order:
            raise ValueError('no zeros left to pair')
        return order[0]
    for i in order:
        r = _isreal(fro[i])
        if (which == 'real' and r) or (which == 'complex' and not r):
            return i
    raise ValueError('no %s zero left to pair' % which)


def _poly(roots):
    """numpy.poly by sequential convolution with [1, -r]; real coefficients."""
    c = [complex(1.0, 0.0)]
    for r in roots:
        r = complex(r)
        nxt = [complex(0.0, 0.0)] * (len(c) + 1)
        for i, cv in enumerate(c):
            nxt[i] += cv
            nxt[i + 1] += cv * (-r)
        c = nxt
    return [cv.real for cv in c]


def _single_zpksos(z, p, k):
    b = [k * v for v in _poly(z)]
    a = _poly(p)
    row = [0.0] * 6
    row[3 - len(b):3] = b
    row[6 - len(a):6] = a
    return row


def _zpk2sos_digital(z, p, k):
    """scipy.signal.zpk2sos with pairing='nearest', analog=False."""
    if len(z) == 0 and len(p) == 0:
        return [[float(k), 0.0, 0.0, 1.0, 0.0, 0.0]]
    p = list(p) + [complex(0.0, 0.0)] * _max(len(z) - len(p), 0)
    z = list(z) + [complex(0.0, 0.0)] * _max(len(p) - len(z), 0)
    n_sections = (_max(len(p), len(z)) + 1) // 2
    if len(p) % 2 == 1:  # pairing == 'nearest'
        p.append(complex(0.0, 0.0))
        z.append(complex(0.0, 0.0))
    if len(z) != len(p):
        raise AssertionError('zpk2sos: unbalanced z/p')

    zc, zr = _cplxreal(z)
    z = zc + [complex(v, 0.0) for v in zr]
    pc, pr = _cplxreal(p)
    p = pc + [complex(v, 0.0) for v in pr]

    def idx_worst(ps):
        # digital: closest to the unit circle; first minimum (np.argmin)
        return _min(range(len(ps)), key=lambda i: _abs(1.0 - _abs(ps[i])))

    sos = [[0.0] * 6 for _ in range(n_sections)]
    for si in range(n_sections - 1, -1, -1):
        p1 = p.pop(idx_worst(p))

        if _isreal(p1) and _nreal(p) == 0:
            # Special case (1): last remaining real pole
            z1 = z.pop(_nearest_rc(z, p1, 'real'))
            sos[si] = _single_zpksos([z1, 0], [p1, 0], 1.0)
        elif (len(p) + 1 == len(z) and not _isreal(p1)
              and _nreal(p) == 1 and _nreal(z) == 1):
            # Special case (2): one real pole and one real zero left --
            # must pair p1 with a complex zero
            z1 = z.pop(_nearest_rc(z, p1, 'complex'))
            sos[si] = _single_zpksos([z1, z1.conjugate()],
                                     [p1, p1.conjugate()], 1.0)
        else:
            if _isreal(p1):
                realidx = [i for i in range(len(p)) if _isreal(p[i])]
                sub = [p[i] for i in realidx]
                p2 = p.pop(realidx[idx_worst(sub)])
            else:
                p2 = p1.conjugate()
            if len(z) > 0:
                z1 = z.pop(_nearest_rc(z, p1, 'any'))
                if not _isreal(z1):
                    sos[si] = _single_zpksos([z1, z1.conjugate()], [p1, p2], 1.0)
                elif len(z) > 0:
                    z2 = z.pop(_nearest_rc(z, p1, 'real'))
                    if not _isreal(z2):
                        raise AssertionError('expected a real zero')
                    sos[si] = _single_zpksos([z1, z2], [p1, p2], 1.0)
                else:
                    sos[si] = _single_zpksos([z1], [p1, p2], 1.0)
            else:
                sos[si] = _single_zpksos([], [p1, p2], 1.0)

    if p or z:
        raise AssertionError('zpk2sos: leftover poles/zeros')
    sos[0][0] *= k
    sos[0][1] *= k
    sos[0][2] *= k
    return sos


def _butter_sos(N, wn, bt):
    # buttap: N poles on unit circle, left half-plane; no zeros; gain 1
    z = []
    p = [-_cmath.exp(1j * _math.pi * m / (2.0 * N)) for m in range(-N + 1, N, 2)]
    k = 1.0
    fs = 2.0
    warped = [2.0 * fs * _math.tan(_math.pi * w / fs) for w in wn]

    if bt == 'lowpass':
        wo = warped[0]
        degree = len(p) - len(z)
        p = [wo * pp for pp in p]
        k = k * wo ** degree
    elif bt == 'highpass':
        wo = warped[0]
        degree = len(p) - len(z)
        zn = [wo / zz for zz in z]
        pn = [wo / pp for pp in p]
        k = k * (_prod([-zz for zz in z]) / _prod([-pp for pp in p])).real
        z = zn + [complex(0.0, 0.0)] * degree
        p = pn
    else:  # bandpass
        bw = warped[1] - warped[0]
        wo = _math.sqrt(warped[0] * warped[1])
        degree = len(p) - len(z)
        z_lp = [zz * bw / 2 for zz in z]
        p_lp = [pp * bw / 2 for pp in p]
        z = ([zl + _cmath.sqrt(zl * zl - wo * wo) for zl in z_lp]
             + [zl - _cmath.sqrt(zl * zl - wo * wo) for zl in z_lp]
             + [complex(0.0, 0.0)] * degree)
        p = ([pl + _cmath.sqrt(pl * pl - wo * wo) for pl in p_lp]
             + [pl - _cmath.sqrt(pl * pl - wo * wo) for pl in p_lp])
        k = k * bw ** degree

    # bilinear_zpk, fs = 2
    degree = len(p) - len(z)
    fs2 = 2.0 * fs
    num = _prod([fs2 - zz for zz in z])
    den = _prod([fs2 - pp for pp in p])
    kz = k * (num / den).real
    z = [(fs2 + zz) / (fs2 - zz) for zz in z] + [complex(-1.0, 0.0)] * degree
    p = [(fs2 + pp) / (fs2 - pp) for pp in p]

    return _zpk2sos_digital(z, p, kz)


def butter(N, Wn, btype='low', analog=False, output='ba', fs=None):
    if analog:
        _fail('butter(analog=True)')
    if fs is not None:
        _fail('butter(fs=...)')
    if output != 'sos':
        _fail("butter output=%r (only 'sos')" % (output,))
    bt = _BTYPE_ALIASES.get(btype)
    if bt is None:
        _fail('butter btype=%r' % (btype,))
    if isinstance(Wn, (list, tuple)):
        wn = [float(w) for w in Wn]
    elif isinstance(Wn, (int, float)):
        wn = [float(Wn)]
    else:
        _fail('butter Wn=%r' % (Wn,))
    for w in wn:
        if not (0.0 < w < 1.0):
            raise ValueError('Digital filter critical frequencies '
                             'must be 0 < Wn < 1')
    if bt == 'bandpass':
        if len(wn) != 2:
            raise ValueError('bandpass needs Wn = [low, high]')
        if not wn[0] < wn[1]:
            raise ValueError('Wn[0] must be less than Wn[1]')
    else:
        if len(wn) != 1:
            raise ValueError('%s needs a single Wn' % bt)
    N = int(N)
    if N < 1:
        raise ValueError('butter order must be >= 1')

    sos = _butter_sos(N, wn, bt)

    key = (N, bt, tuple(wn))
    if key not in BUTTER_REGISTRY:
        design_id = 'butter,%d,%s,%s' % (N, bt, ','.join(repr(w) for w in wn))
        BUTTER_REGISTRY[key] = (design_id, [row[:] for row in sos])
    return sos


def _sosfilt_1d(sos, xs):
    ys = [float(v) for v in xs]
    n = len(ys)
    for row in sos:
        b0, b1, b2, a0, a1, a2 = row
        if a0 != 1.0:
            _fail('sosfilt requires normalized sections (a0 == 1)')
        s1 = 0.0
        s2 = 0.0
        for i in range(n):
            x = ys[i]
            y = b0 * x + s1
            s1 = b1 * x - a1 * y + s2
            s2 = b2 * x - a2 * y
            ys[i] = y
    return ys


def sosfilt(sos, x, axis=None, zi=None):
    if zi is not None:
        _fail('sosfilt(zi=...)')
    if not isinstance(x, NDArray):
        _fail('sosfilt input %r' % type(x))
    if len(x.shape) == 1:
        # scipy default axis=-1 is the only axis of a 1-D array
        if axis not in (None, -1, 0):
            _fail('sosfilt axis=%r on 1-D input' % (axis,))
        return NDArray(_sosfilt_1d(sos, x.data), x.shape, _F8)
    if axis != 0:
        _fail('sosfilt on 2-D input requires axis=0')
    n, c = x.shape
    d = x.data
    out = [0.0] * (n * c)
    for ch in range(c):
        col = _sosfilt_1d(sos, d[ch::c])
        out[ch::c] = col
    return NDArray(out, x.shape, _F8)


# ===================================================================
# fail loudly on any numpy attribute the shim does not implement
# ===================================================================

def __getattr__(name):
    if name.startswith('__') and name.endswith('__'):
        raise AttributeError(name)
    raise NotImplementedError(
        'pynp: numpy attribute %r is not implemented by the shim '
        '(implement it only if upstream claudewave code exercises it)' % name)
