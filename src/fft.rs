//! Fast Fourier transforms for the `fft.*` namespace (v0.7).
//!
//! Split-complex representation (separate re/im arrays), f64 throughout.
//! Power-of-two sizes use an iterative radix-2 Cooley–Tukey; everything
//! else goes through Bluestein's chirp-z algorithm (which reduces to a
//! power-of-two convolution), so any length n >= 1 works in O(n log n).
//! Results match numpy/pocketfft to ~1e-12 relative on well-scaled input
//! (twiddles here are plain sin/cos; pocketfft uses extended-precision
//! twiddle generation, so bit-exactness is not expected — the CI
//! cross-check runs at 1e-9).

/// In-place radix-2 DIT FFT. `n` must be a power of two. `inverse` only
/// flips the twiddle sign — no 1/n normalization here.
fn fft_pow2(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    if n <= 1 {
        return;
    }
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * 2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let (ur, ui) = (re[i + k], im[i + k]);
                let (vr, vi) = (
                    re[i + k + len / 2] * cr - im[i + k + len / 2] * ci,
                    re[i + k + len / 2] * ci + im[i + k + len / 2] * cr,
                );
                re[i + k] = ur + vr;
                im[i + k] = ui + vi;
                re[i + k + len / 2] = ur - vr;
                im[i + k + len / 2] = ui - vi;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Bluestein's algorithm: an arbitrary-n DFT as a power-of-two convolution.
fn fft_bluestein(re: &[f64], im: &[f64], inverse: bool) -> (Vec<f64>, Vec<f64>) {
    let n = re.len();
    let sign = if inverse { 1.0 } else { -1.0 };
    // Chirp: w_k = exp(sign * i * pi * k^2 / n). k^2 mod 2n keeps the
    // angle argument small (k*k overflows nothing at i64 for our sizes).
    let chirp = |k: usize| -> (f64, f64) {
        let k2 = ((k as u64 * k as u64) % (2 * n as u64)) as f64;
        let ang = sign * std::f64::consts::PI * k2 / n as f64;
        (ang.cos(), ang.sin())
    };
    let m = (2 * n - 1).next_power_of_two();
    let (mut ar, mut ai) = (vec![0.0; m], vec![0.0; m]);
    let (mut br, mut bi) = (vec![0.0; m], vec![0.0; m]);
    for k in 0..n {
        let (cr, ci) = chirp(k);
        // a_k = x_k * chirp(k)
        ar[k] = re[k] * cr - im[k] * ci;
        ai[k] = re[k] * ci + im[k] * cr;
        // b_k = conj(chirp(k)), wrapped for circular convolution
        br[k] = cr;
        bi[k] = -ci;
        if k > 0 {
            br[m - k] = cr;
            bi[m - k] = -ci;
        }
    }
    fft_pow2(&mut ar, &mut ai, false);
    fft_pow2(&mut br, &mut bi, false);
    for k in 0..m {
        let (xr, xi) = (ar[k], ai[k]);
        ar[k] = xr * br[k] - xi * bi[k];
        ai[k] = xr * bi[k] + xi * br[k];
    }
    fft_pow2(&mut ar, &mut ai, true);
    let inv_m = 1.0 / m as f64;
    let mut out_re = vec![0.0; n];
    let mut out_im = vec![0.0; n];
    for k in 0..n {
        let (cr, ci) = chirp(k);
        let (xr, xi) = (ar[k] * inv_m, ai[k] * inv_m);
        out_re[k] = xr * cr - xi * ci;
        out_im[k] = xr * ci + xi * cr;
    }
    (out_re, out_im)
}

/// Forward DFT of a split-complex signal, any n >= 1. No normalization.
pub fn fft(re: &[f64], im: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = re.len();
    if n.is_power_of_two() {
        let (mut r, mut i) = (re.to_vec(), im.to_vec());
        fft_pow2(&mut r, &mut i, false);
        (r, i)
    } else {
        fft_bluestein(re, im, false)
    }
}

/// Inverse DFT with 1/n normalization, any n >= 1.
pub fn ifft(re: &[f64], im: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = re.len();
    let (mut r, mut i) = if n.is_power_of_two() {
        let (mut r, mut i) = (re.to_vec(), im.to_vec());
        fft_pow2(&mut r, &mut i, true);
        (r, i)
    } else {
        fft_bluestein(re, im, true)
    };
    let inv = 1.0 / n as f64;
    for v in r.iter_mut() {
        *v *= inv;
    }
    for v in i.iter_mut() {
        *v *= inv;
    }
    (r, i)
}

/// Real-input FFT: the first n/2 + 1 bins of `fft(x, 0)` (numpy.fft.rfft).
pub fn rfft(x: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = x.len();
    let (r, i) = fft(x, &vec![0.0; n]);
    let bins = n / 2 + 1;
    (r[..bins].to_vec(), i[..bins].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    /// Textbook O(n^2) DFT as the oracle.
    fn dft_naive(re: &[f64], im: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let n = re.len();
        let mut or_ = vec![0.0; n];
        let mut oi = vec![0.0; n];
        for k in 0..n {
            for t in 0..n {
                let ang = -2.0 * std::f64::consts::PI * (k * t) as f64 / n as f64;
                or_[k] += re[t] * ang.cos() - im[t] * ang.sin();
                oi[k] += re[t] * ang.sin() + im[t] * ang.cos();
            }
        }
        (or_, oi)
    }

    #[test]
    fn matches_naive_dft_for_all_small_sizes() {
        for n in 1..=33usize {
            // A deterministic, awkward signal.
            let re: Vec<f64> = (0..n).map(|i| ((i * 37 + 11) % 17) as f64 - 8.0).collect();
            let im: Vec<f64> = (0..n).map(|i| ((i * 23 + 5) % 13) as f64 * 0.5).collect();
            let (fr, fi) = fft(&re, &im);
            let (nr, ni) = dft_naive(&re, &im);
            for k in 0..n {
                assert!(close(fr[k], nr[k]) && close(fi[k], ni[k]), "n={n} bin {k}");
            }
            // Round trip.
            let (br, bi) = ifft(&fr, &fi);
            for k in 0..n {
                assert!(close(br[k], re[k]) && close(bi[k], im[k]), "roundtrip n={n} k={k}");
            }
        }
    }

    #[test]
    fn rfft_matches_full_fft_prefix() {
        for n in [1usize, 2, 7, 8, 12, 100] {
            let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.7).sin()).collect();
            let (rr, ri) = rfft(&x);
            let (fr, fi) = fft(&x, &vec![0.0; n]);
            assert_eq!(rr.len(), n / 2 + 1);
            for k in 0..rr.len() {
                assert!(close(rr[k], fr[k]) && close(ri[k], fi[k]), "n={n} bin {k}");
            }
        }
    }
}
