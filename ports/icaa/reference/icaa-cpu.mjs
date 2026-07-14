#!/usr/bin/env node
// ===========================================================================
// icaa-cpu.mjs — float64 CPU reference implementation of ICAA
// (Isoline-Coverage Anti-Aliasing), transliterated from the three.js TSL
// shader node ICAANode.js (MIT licensed). Self-contained: Node >= 18, ESM,
// zero dependencies.
//
// Usage:
//   node icaa-cpu.mjs <in.ppm> <out.ppm> <quality|fast> [debug N] [probe X Y]
//
//   debug N   — set the integer debug uniform (0..5).
//               1=confidence 2=coverage 3=distance 4=orientation other>0=rms
//   probe X Y — process ONLY output pixel (X,Y); print 'name=value' lines
//               instead of writing the image (cross-debugging tool).
//
// PPM I/O is plain-text P3 with maxval 255. All math is IEEE-754 float64.
// Expression order and associativity mirror the TSL source exactly — no
// algebraic simplification (a.add(b).mul(c) is (a+b)*c, exactly).
//
// Pinned GLSL-function semantics (cross-implementation contract):
//   clamp(x,lo,hi)      = min(max(x,lo),hi)
//   mix(x,y,a)          = x*(1-a) + y*a                    [componentwise]
//   smoothstep(a,b,x)   = t = clamp((x-a)/(b-a),0,1); t*t*(3.0 - 2.0*t)
//   normalize(v)        = (v.x/L, v.y/L), L = sqrt(v.x*v.x + v.y*v.y)
//   inverseSqrt(x)      = 1.0/sqrt(x)
//   dot                 = left-fold sum of products (x*x' + y*y' [+ z*z'])
//   luma(rgb)           = r*0.299 + g*0.587 + b*0.114      (left fold)
//   vec ops             = componentwise; scalar ops broadcast
//   select(c,a,b)       = c ? a : b   (operands effect-free)
//   bool.notEqual(bool) = XOR
//
// Texture sampling (bilinear, clamp-to-edge, texel centers at half-texel):
//   x = u*W - 0.5; x0 = floor(x); fx = x - x0   (same for y)
//   indices clamped to [0, size-1] per axis;
//   top = c00*(1-fx)+c10*fx; bot = c01*(1-fx)+c11*fx; out = top*(1-fy)+bot*fy
// ===========================================================================

import { readFileSync, writeFileSync } from 'node:fs';

// ------------------------------------------------------------- math helpers

const clamp = (x, lo, hi) => Math.min(Math.max(x, lo), hi);
const mix = (x, y, a) => x * (1.0 - a) + y * a;
const smoothstep = (e0, e1, x) => {
	const t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
	return t * t * (3.0 - 2.0 * t);
};

// vec2 helpers ([x, y] arrays, componentwise)
const v2add = (a, b) => [a[0] + b[0], a[1] + b[1]];
const v2sub = (a, b) => [a[0] - b[0], a[1] - b[1]];
const v2mul = (a, b) => [a[0] * b[0], a[1] * b[1]]; // componentwise product
const v2scale = (a, s) => [a[0] * s, a[1] * s]; // scalar broadcast
const dot2 = (a, b) => a[0] * b[0] + a[1] * b[1];
const normalize2 = (v) => {
	const L = Math.sqrt(v[0] * v[0] + v[1] * v[1]);
	return [v[0] / L, v[1] / L];
};

// rgb helpers ([r, g, b] arrays)
const v3add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const v3scale = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
const v3mix = (a, b, t) => [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)];

// LUMA on raw (display-referred) sampled values
const lumaOf = (c) => c[0] * 0.299 + c[1] * 0.587 + c[2] * 0.114;

// sRGB transfer curves — three.js r178 constants, exactly.
// Used only in the 'quality' resynthesis mix.
const sRGBEOTF = (c) => (c <= 0.04045 ? c * 0.0773993808 : Math.pow(c * 0.9478672986 + 0.0521327014, 2.4));
const sRGBOETF = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 0.41666) - 0.055);

// ---------------------------------------------------------------- PPM I/O

function readPPM(path) {
	const text = readFileSync(path, 'latin1');
	const tokens = [];
	for (const line of text.split('\n')) {
		const t = line.trim();
		if (t.length === 0) continue;
		if (t.startsWith('#')) continue; // whole-line comment
		for (const tok of t.split(/\s+/)) tokens.push(tok);
	}
	if (tokens[0] !== 'P3') throw new Error(`${path}: not a plain-text P3 PPM (magic '${tokens[0]}')`);
	const w = Number(tokens[1]);
	const h = Number(tokens[2]);
	const maxval = Number(tokens[3]);
	if (!Number.isInteger(w) || !Number.isInteger(h) || w <= 0 || h <= 0) {
		throw new Error(`${path}: bad dimensions`);
	}
	if (maxval !== 255) throw new Error(`${path}: maxval must be 255 (got ${maxval})`);
	const n = w * h * 3;
	if (tokens.length < 4 + n) throw new Error(`${path}: truncated pixel data (${tokens.length - 4} of ${n} values)`);
	const data = new Float64Array(n);
	for (let i = 0; i < n; i++) {
		const v = Number(tokens[4 + i]);
		if (!Number.isFinite(v)) throw new Error(`${path}: bad pixel value '${tokens[4 + i]}'`);
		data[i] = v / 255.0; // alpha = 1.0 implicitly
	}
	return { w, h, data };
}

function toByte(c) {
	if (Number.isNaN(c)) return 0; // guard; shader math should never produce NaN
	const r = Math.round(c * 255.0);
	return Math.min(Math.max(r, 0), 255);
}

function writePPM(path, w, h, rgbF64) {
	const parts = [`P3`, `${w} ${h}`, `255`];
	for (let i = 0; i < w * h; i++) {
		const r = toByte(rgbF64[i * 3 + 0]);
		const g = toByte(rgbF64[i * 3 + 1]);
		const b = toByte(rgbF64[i * 3 + 2]);
		parts.push(`${r} ${g} ${b}`);
	}
	writeFileSync(path, parts.join('\n') + '\n');
}

// ----------------------------------------------------- bilinear tex sampler

const clampIdx = (i, n) => (i < 0 ? 0 : (i > n - 1 ? n - 1 : i));

function makeSampler(img) {
	const { w, h, data } = img;
	// returns [r, g, b]; alpha of the source is 1.0 and never varies
	return (u, v) => {
		const x = u * w - 0.5;
		const y = v * h - 0.5;
		const x0 = Math.floor(x);
		const y0 = Math.floor(y);
		const fx = x - x0;
		const fy = y - y0;
		const cx0 = clampIdx(x0, w);
		const cx1 = clampIdx(x0 + 1, w);
		const cy0 = clampIdx(y0, h);
		const cy1 = clampIdx(y0 + 1, h);
		const i00 = (cy0 * w + cx0) * 3;
		const i10 = (cy0 * w + cx1) * 3;
		const i01 = (cy1 * w + cx0) * 3;
		const i11 = (cy1 * w + cx1) * 3;
		const out = [0.0, 0.0, 0.0];
		for (let k = 0; k < 3; k++) {
			const top = data[i00 + k] * (1.0 - fx) + data[i10 + k] * fx;
			const bot = data[i01 + k] * (1.0 - fx) + data[i11 + k] * fx;
			out[k] = top * (1.0 - fy) + bot * fy;
		}
		return out;
	};
}

// ------------------------------------------------------------- ICAA proper
//
// Transliteration of ICAANode.setup()/ApplyICAA. `sampleColor(q)` takes a
// uv [u,v] and returns [r,g,b]. `T` is a trace callback (probe mode) or null.
// Returns the output [r,g,b] (alpha in = 1.0, out written without alpha).

// exact area of the unit square (centered at origin) on the side { x . n <= d }
// n must be unit length. Piecewise-quadratic closed form.
function areaLE(nrm, d) {
	const ax = Math.abs(nrm[0]);
	const ay = Math.abs(nrm[1]);
	const hi = Math.max(ax, ay);
	const lo = Math.min(ax, ay);
	const s = (hi + lo) * 0.5; // corner distance
	const q = (hi - lo) * 0.5; // linear-zone half width

	const linear = clamp(d / hi + 0.5, 0.0, 1.0);
	const loSafe = Math.max(lo, 1e-3);
	const cutLow = ((d + s) * (d + s)) / ((hi * loSafe) * 2.0);
	const cutHigh = 1.0 - (((s - d) * (s - d)) / ((hi * loSafe) * 2.0));

	const pieced = (d <= -s) ? 0.0
		: ((d >= s) ? 1.0
			: ((d < -q) ? cutLow
				: ((d > q) ? cutHigh : linear)));

	return (lo < 1e-3) ? linear : pieced;
}

// crossing of the segment between taps (v1,f1)-(v2,f2), NaN-safe
// v1, v2 are plain numbers (compile-time constants in the TSL source)
function pairCross(f1, f2, v1, v2) {
	const den = f1 - f2;
	const denSafe = (Math.abs(den) < 1e-6) ? 1e-6 : den;
	const has = (f1 * f2) < 0.0;
	return {
		v: (f1 * (v2 - v1)) / denSafe + v1,
		w: Math.abs(den) * (has ? 1.0 : 0.0),
		has,
	};
}

function applyICAA(sampleColor, uvIn, texel, cfg, T) {
	const { fast, contrastAbs, contrastRel, cohMin, strength, widthK, widthBase, tentMix, debug } = cfg;

	const lumaAt = (q) => lumaOf(sampleColor(q));

	const p = uvIn;
	const colorC = sampleColor(p); // colorC.a = 1.0

	// ---- 1. cheap early exit on the 5-tap cross ----

	const lC = lumaOf(colorC);
	const cN = sampleColor(v2add(p, v2mul(texel, [0.0, -1.0])));
	const cS = sampleColor(v2add(p, v2mul(texel, [0.0, 1.0])));
	const cE = sampleColor(v2add(p, v2mul(texel, [1.0, 0.0])));
	const cW = sampleColor(v2add(p, v2mul(texel, [-1.0, 0.0])));
	const lN = lumaOf(cN);
	const lS = lumaOf(cS);
	const lE = lumaOf(cE);
	const lW = lumaOf(cW);

	if (T) { T('lC', lC); T('lN', lN); T('lS', lS); T('lE', lE); T('lW', lW); }

	const maxCross = Math.max(lC, Math.max(Math.max(lN, lS), Math.max(lE, lW)));
	const minCross = Math.min(lC, Math.min(Math.min(lN, lS), Math.min(lE, lW)));
	const threshold = Math.max(contrastAbs, contrastRel * maxCross);

	if (T) T('threshold', threshold);

	if ((maxCross - minCross) < (threshold * 0.85)) {
		if (T) T('early_exit', 'cross');
		return [colorC[0], colorC[1], colorC[2]];
	}

	// ---- 2. corners, full 3x3 range ----

	const cNW = sampleColor(v2add(p, v2mul(texel, [-1.0, -1.0])));
	const cNE = sampleColor(v2add(p, v2mul(texel, [1.0, -1.0])));
	const cSW = sampleColor(v2add(p, v2mul(texel, [-1.0, 1.0])));
	const cSE = sampleColor(v2add(p, v2mul(texel, [1.0, 1.0])));
	const lNW = lumaOf(cNW);
	const lNE = lumaOf(cNE);
	const lSW = lumaOf(cSW);
	const lSE = lumaOf(cSE);

	const maxL = Math.max(maxCross, Math.max(Math.max(lNW, lNE), Math.max(lSW, lSE)));
	const minL = Math.min(minCross, Math.min(Math.min(lNW, lNE), Math.min(lSW, lSE)));
	const range = maxL - minL;

	if (T) T('range', range);

	if (range < threshold) {
		if (T) T('early_exit', 'range');
		return [colorC[0], colorC[1], colorC[2]];
	}

	// ---- 3. structure tensor from four sub-quad gradients ----
	// quad centers at (+-0.5, +-0.5); gradients by 2x2 block differences

	const g1 = v2scale([((lN + lC) - lNW) - lW, ((lW + lC) - lNW) - lN], 0.5);
	const g2 = v2scale([((lNE + lE) - lN) - lC, ((lC + lE) - lN) - lNE], 0.5);
	const g3 = v2scale([((lC + lS) - lW) - lSW, ((lSW + lS) - lW) - lC], 0.5);
	const g4 = v2scale([((lE + lSE) - lC) - lS, ((lS + lSE) - lC) - lE], 0.5);

	const jxx = ((g1[0] * g1[0] + g2[0] * g2[0]) + g3[0] * g3[0]) + g4[0] * g4[0];
	const jyy = ((g1[1] * g1[1] + g2[1] * g2[1]) + g3[1] * g3[1]) + g4[1] * g4[1];
	const jxy = ((g1[0] * g1[1] + g2[0] * g2[1]) + g3[0] * g3[1]) + g4[0] * g4[1];

	const tr = jxx + jyy;
	const halfDiff = (jxx - jyy) * 0.5;
	const disc = Math.sqrt(Math.max(halfDiff * halfDiff + jxy * jxy, 0.0));
	const coh = (disc * 2.0) / (tr + 1e-7);

	if (T) T('coh', coh);

	if (coh < cohMin) {
		if (T) T('early_exit', 'coh');
		return [colorC[0], colorC[1], colorC[2]]; // texture / noise / corner
	}

	const lam1 = tr * 0.5 + disc;

	// principal eigenvector (gradient direction across the edge)
	const nA = [jxy, lam1 - jxx];
	const nB = [lam1 - jyy, jxy];
	const nRaw = (dot2(nA, nA) > dot2(nB, nB)) ? nA : nB;
	const gMean = v2add(v2add(v2add(g1, g2), g3), g4);
	const nDir = v2scale(normalize2(nRaw), (dot2(nRaw, gMean) < 0.0) ? -1.0 : 1.0);

	// results of the line estimation, shared by both presets
	let u0 = 0.0;
	let m = 0.0;
	let rms = 0.0;
	let Sw = 0.0;
	let lMid = 0.0;
	let amp2 = 0.0;
	let wFlat = 1.0;
	let nBase = [0.0, 0.0]; // normal axis of the fit frame (unit)
	let tBase = [0.0, 0.0]; // tangent axis of the fit frame (unit)

	if (fast) {

		// ============================ FAST PRESET =========================

		const horz = Math.abs(nDir[1]) >= Math.abs(nDir[0]); // horizontal-ish edge
		const sy = horz
			? ((nDir[1] >= 0.0) ? 1.0 : -1.0)
			: ((nDir[0] >= 0.0) ? 1.0 : -1.0);

		nBase = horz ? [0.0, sy] : [sy, 0.0];
		tBase = horz ? [1.0, 0.0] : [0.0, 1.0];

		// grid lumas mapped into (station s = tangent offset, tap v' = grid
		// normal offset)
		const t00 = lNW;                     // s=-1 v'=-1
		const t01 = horz ? lW : lN;          // s=-1 v'= 0
		const t02 = horz ? lSW : lNE;        // s=-1 v'=+1
		const t10 = horz ? lN : lW;          // s= 0 v'=-1
		const t11 = lC;                      // s= 0 v'= 0
		const t12 = horz ? lS : lE;          // s= 0 v'=+1
		const t20 = horz ? lNE : lSW;        // s=+1 v'=-1
		const t21 = horz ? lE : lS;          // s=+1 v'= 0
		const t22 = lSE;                     // s=+1 v'=+1

		// plateau estimates from the two outer rows (free)
		const rowP = ((t02 + t12) + t22) * (1.0 / 3.0); // v' = +1
		const rowN = ((t00 + t10) + t20) * (1.0 / 3.0); // v' = -1
		// orient v along +nBase: sy>0 keeps v'=+1 as the light side
		const lPlus = (sy > 0.0) ? rowP : rowN;
		const lMinus = (sy > 0.0) ? rowN : rowP;
		lMid = (lPlus + lMinus) * 0.5;
		amp2 = Math.abs(lPlus - lMinus);

		// plateau-flatness confirmation (2 taps): veto gratings/thin strokes
		const lP2 = lumaAt(v2add(p, v2scale(v2mul(nBase, texel), 2.0)));
		const lN2 = lumaAt(v2sub(p, v2scale(v2mul(nBase, texel), 2.0)));
		const unflat = (Math.abs(lP2 - lPlus) + Math.abs(lN2 - lMinus)) / Math.max(amp2, 1e-4);
		wFlat = 1.0 - smoothstep(0.28, 0.65, unflat);

		// three grid stations
		let Sws = 0.0;
		let Swss = 0.0;
		let Swu = 0.0;
		let Swsu = 0.0;
		const stationList = [];

		const gridStation = (s, a0, b0, c0, wBase) => {

			// order taps along +nBase
			const a = ((sy > 0.0) ? a0 : c0) - lMid;
			const b = b0 - lMid;
			const c = ((sy > 0.0) ? c0 : a0) - lMid;

			const pAB = pairCross(a, b, -1.0, 0.0);
			const pBC = pairCross(b, c, 0.0, 1.0);
			const pAC = pairCross(a, c, -1.0, 1.0);
			const wSum = (pAB.w + pBC.w) + pAC.w * 0.5;
			const vStar = clamp(
				((pAB.w * pAB.v + pBC.w * pBC.v) + (pAC.w * 0.5) * pAC.v)
					/ Math.max(wSum, 1e-6),
				-1.2, 1.2);

			const w = ((wBase
				* ((pAB.has !== pBC.has) ? 1.0 : 0.0))
				* (1.0 - smoothstep(0.75, 1.02, Math.abs(vStar))))
				* smoothstep(0.0, Math.max(amp2 * 0.25, 1e-4), c - a);

			Sw += w;
			Sws += w * s;
			Swss += w * (s * s);
			Swu += w * vStar;
			Swsu += (w * vStar) * s;
			stationList.push({ s, u: vStar, w });

		};

		gridStation(-1, t00, t01, t02, 0.85);
		gridStation(0, t10, t11, t12, 1.0);
		gridStation(1, t20, t21, t22, 0.85);

		// two staggered bilinear outrigger stations
		const outrigger = (s, dStag, wBase) => {

			const base = v2add(v2add(p, v2scale(v2mul(tBase, texel), s)), v2scale(v2mul(nBase, texel), dStag));
			const a = lumaAt(v2sub(base, v2mul(nBase, texel))) - lMid;
			const b = lumaAt(base) - lMid;
			const c = lumaAt(v2add(base, v2mul(nBase, texel))) - lMid;

			const pAB = pairCross(a, b, -1.0, 0.0);
			const pBC = pairCross(b, c, 0.0, 1.0);
			const pAC = pairCross(a, c, -1.0, 1.0);
			const wSum = (pAB.w + pBC.w) + pAC.w * 0.5;
			const vStar = clamp(
				((pAB.w * pAB.v + pBC.w * pBC.v) + (pAC.w * 0.5) * pAC.v)
					/ Math.max(wSum, 1e-6),
				-1.2, 1.2);
			const u = vStar + dStag;

			const w = ((wBase
				* ((pAB.has !== pBC.has) ? 1.0 : 0.0))
				* (1.0 - smoothstep(0.75, 1.02, Math.abs(vStar))))
				* smoothstep(0.0, Math.max(amp2 * 0.25, 1e-4), c - a);

			Sw += w;
			Sws += w * s;
			Swss += w * (s * s);
			Swu += w * u;
			Swsu += (w * u) * s;
			stationList.push({ s, u, w });

		};

		outrigger(-1.4, -0.28, 0.8);
		outrigger(1.4, 0.28, 0.8);

		// slope prior from the tensor eigenvector (sub-pixel orientation)
		const nT = dot2(nDir, tBase);
		const nN = dot2(nDir, nBase);
		const mT = clamp(-(nT / nN), -1.2, 1.2);

		// LSQ with a slope prior m -> mT (weight LP); no prior on the level
		const LP = 1.2;
		const D = Math.max(Sw * (Swss + LP) - Sws * Sws, 1e-5);
		const SwsuP = Swsu + mT * LP;
		u0 = ((Swss + LP) * Swu - Sws * SwsuP) / D;
		m = clamp((Sw * SwsuP - Sws * Swu) / D, -1.2, 1.2);

		let Sr = 0.0;
		for (const st of stationList) {

			const e = (st.u - u0) - m * st.s;
			Sr = Sr + (st.w * e) * e;

		}

		rms = Math.sqrt(Sr / Math.max(Sw, 1e-4));

	} else {

		// ========================== QUALITY PRESET ========================

		const tDir = [-nDir[1], nDir[0]];
		nBase = nDir;
		tBase = tDir;

		// mid-level isoline value from the two side plateaus
		const lPlus = lumaAt(v2add(p, v2mul(nDir, texel)));
		const lMinus = lumaAt(v2sub(p, v2mul(nDir, texel)));
		lMid = (lPlus + lMinus) * 0.5;
		amp2 = Math.abs(lPlus - lMinus);

		// plateau-flatness confirmation: veto gratings/thin strokes
		const lPlus2 = lumaAt(v2add(p, v2scale(v2mul(nDir, texel), 2.0)));
		const lMinus2 = lumaAt(v2sub(p, v2scale(v2mul(nDir, texel), 2.0)));
		const unflat = (Math.abs(lPlus2 - lPlus) + Math.abs(lMinus2 - lMinus)) / Math.max(amp2, 1e-4);
		wFlat = 1.0 - smoothstep(0.28, 0.65, unflat);

		// isoline crossings at stations along the tangent, LSQ line fit
		let Sws = 0.0;
		let Swss = 0.0;
		let Swu = 0.0;
		let Swsu = 0.0;

		// separate accumulators for the inner (local) fit
		let Tw = 0.0;
		let Tws = 0.0;
		let Twss = 0.0;
		let Twu = 0.0;
		let Twsu = 0.0;

		const stations = [];

		// 3-tap window (v = -1,0,+1): three pair estimates blended by pair
		// contrast; s === 0 is a build-time constant in the TSL source
		const stationEstimate = (s, uPred) => {

			let a, b, c;
			if (s === 0) {

				a = lMinus - lMid;
				b = lC - lMid;
				c = lPlus - lMid;

			} else {

				const base = v2add(v2add(p, v2scale(v2mul(tDir, texel), s)), v2scale(v2mul(nDir, texel), uPred));
				a = lumaAt(v2sub(base, v2mul(nDir, texel))) - lMid;
				b = lumaAt(base) - lMid;
				c = lumaAt(v2add(base, v2mul(nDir, texel))) - lMid;

			}

			const pAB = pairCross(a, b, -1.0, 0.0);
			const pBC = pairCross(b, c, 0.0, 1.0);
			const pAC = pairCross(a, c, -1.0, 1.0);

			const wSum = (pAB.w + pBC.w) + pAC.w * 0.5;
			const vStar = clamp(
				((pAB.w * pAB.v + pBC.w * pBC.v) + (pAC.w * 0.5) * pAC.v)
					/ Math.max(wSum, 1e-6),
				-1.2, 1.2);

			const wRaw = (((pAB.has !== pBC.has) ? 1.0 : 0.0)
				* (1.0 - smoothstep(0.75, 1.02, Math.abs(vStar))))
				* smoothstep(0.0, Math.max(amp2 * 0.25, 1e-4), c - a);

			return { u: uPred + vStar, wRaw };

		};

		const accumulate = (s, u, w, inner) => {

			Sw += w;
			Sws += w * s;
			Swss += w * (s * s);
			Swu += w * u;
			Swsu += (w * u) * s;

			if (inner) {

				Tw += w;
				Tws += w * s;
				Twss += w * (s * s);
				Twu += w * u;
				Twsu += (w * u) * s;

			}

			stations.push({ s, u, w });

		};

		const emitStation = (s, wBase, uPred, inner = false) => {

			const est = stationEstimate(s, uPred);
			accumulate(s, est.u, est.wRaw * wBase, inner);

		};

		const S1 = [-2.4, -1.8, -1.2, -0.6, 0, 0.6, 1.2, 1.8, 2.4];
		const W1 = [0.5, 0.68, 0.82, 0.94, 1.0, 0.94, 0.82, 0.68, 0.5];
		// fractional normal-phase stagger
		const D1 = [-0.42, 0.32, -0.21, 0.1, 0.0, -0.1, 0.21, -0.32, 0.42];
		for (let i = 0; i < S1.length; i++) {

			emitStation(S1[i], W1[i], D1[i], Math.abs(S1[i]) <= 0.7);

		}

		const LAMBDA = 0.25; // ridge prior on the slope

		const fit = (sw, sws, swss, swu, swsu) => {

			const D = Math.max(sw * (swss + LAMBDA) - sws * sws, 1e-5);
			const u0f = ((swss + LAMBDA) * swu - sws * swsu) / D;
			const mf = clamp((sw * swsu - sws * swu) / D, -1.2, 1.2);
			return { u0f, mf };

		};

		const residual = (u0r, mr, list, swr) => {

			let Sr = 0.0;
			for (const st of list) {

				const e = (st.u - u0r) - mr * st.s;
				Sr = Sr + (st.w * e) * e;

			}

			return Math.sqrt(Sr / Math.max(swr, 1e-4));

		};

		// pass-1 fit over the 9 staggered stations
		const f1 = fit(Sw, Sws, Swss, Swu, Swsu);
		const u01 = f1.u0f;
		const m1 = f1.mf;
		const rms1 = residual(u01, m1, stations, Sw);

		// tracked wide stations for shallow edges
		const wWide = (((1.0 - smoothstep(0.20, 0.38, rms1))
			* (1.0 - smoothstep(0.35, 0.55, Math.abs(m1))))
			* smoothstep(0.9, 1.4, Sw));

		const SWIDE = [-8.4, -6.0, -3.6, 3.6, 6.0, 8.4];
		const DWIDE = [0.25, -0.25, 0.08, -0.08, 0.25, -0.25];
		const wideVars = SWIDE.map(() => ({ u: 0.0, w: 0.0 }));

		if (wWide > 0.01) {

			for (let k = 0; k < SWIDE.length; k++) {

				const uPred = clamp(u01 + m1 * SWIDE[k], -2.5, 2.5) + DWIDE[k];
				const est = stationEstimate(SWIDE[k], uPred);
				wideVars[k].u = est.u;
				wideVars[k].w = est.wRaw * (wWide * 0.35);

			}

		}

		for (let k = 0; k < SWIDE.length; k++) {

			accumulate(SWIDE[k], wideVars[k].u, wideVars[k].w, false);

		}

		// full fit (all stations) and inner-only fit (locally straight: curves)
		const fF = fit(Sw, Sws, Swss, Swu, Swsu);
		const u0F = fF.u0f;
		const mF = fF.mf;
		const rmsF = residual(u0F, mF, stations, Sw);

		const fI = fit(Tw, Tws, Twss, Twu, Twsu);
		const u0I = fI.u0f;
		const mI = fI.mf;
		const innerStations = stations.slice(0, S1.length).filter((st) => Math.abs(st.s) <= 0.7);
		const rmsI = Math.max(residual(u0I, mI, innerStations, Tw), 0.06);

		// lean on the local fit when the straight-line model misfits (curvature)
		const wLoc = smoothstep(0.10, 0.35, rmsF);
		u0 = mix(u0F, u0I, wLoc);
		m = mix(mF, mI, wLoc);
		rms = mix(rmsF, rmsI, wLoc);

	}

	// ---- fitted edge line -> analytic pixel coverage (shared) ----

	const invLen = 1.0 / Math.sqrt(m * m + 1.0);
	const n2 = v2scale(v2sub(nBase, v2scale(tBase, m)), invLen); // unit normal of fitted line
	const d0 = u0 * invLen; // signed center-to-line distance (px)

	// uncertainty-adaptive sharpness
	const sigma = rms * (1.0 / Math.sqrt(Math.max(Sw, 0.5)));
	const width = clamp(sigma * widthK + widthBase, 1.0, 2.5);

	const aLE = areaLE(n2, d0 / width); // dark-side area
	const aLight = 1.0 - aLE;

	// ---- side colors, linear-space coverage mix ----

	let cDark, cLight;
	if (fast) {

		// side colors are the 3-texel row/column averages of the already
		// sampled 3x3 grid along the dominant axis
		const syPos = dot2(nBase, [1.0, 1.0]) > 0.0;
		const horz2 = Math.abs(nBase[1]) > 0.5;
		const third = 1.0 / 3.0;
		const rowN3 = v3scale(v3add(v3add(cNW, cN), cNE), third);
		const rowS3 = v3scale(v3add(v3add(cSW, cS), cSE), third);
		const colW3 = v3scale(v3add(v3add(cNW, cW), cSW), third);
		const colE3 = v3scale(v3add(v3add(cNE, cE), cSE), third);
		cLight = horz2 ? (syPos ? rowS3 : rowN3) : (syPos ? colE3 : colW3);
		cDark = horz2 ? (syPos ? rowN3 : rowS3) : (syPos ? colW3 : colE3);

	} else {

		// fixed pixel-relative side offsets
		const offD = clamp(d0 * 0.35 - 1.05, -1.6, 1.6);
		const offL = clamp(d0 * 0.35 + 1.05, -1.6, 1.6);
		cDark = sampleColor(v2add(p, v2scale(v2mul(n2, texel), offD)));
		cLight = sampleColor(v2add(p, v2scale(v2mul(n2, texel), offL)));

	}

	// coverage mix in linear light. The fast preset uses the gamma-2.0
	// square approximation (mul/sqrt) instead of the exact sRGB pow curves.
	let rgbNew;
	if (fast) {

		rgbNew = [
			Math.sqrt(mix(cDark[0] * cDark[0], cLight[0] * cLight[0], aLight)),
			Math.sqrt(mix(cDark[1] * cDark[1], cLight[1] * cLight[1], aLight)),
			Math.sqrt(mix(cDark[2] * cDark[2], cLight[2] * cLight[2], aLight)),
		];

	} else {

		rgbNew = [
			sRGBOETF(mix(sRGBEOTF(cDark[0]), sRGBEOTF(cLight[0]), aLight)),
			sRGBOETF(mix(sRGBEOTF(cDark[1]), sRGBEOTF(cLight[1]), aLight)),
			sRGBOETF(mix(sRGBEOTF(cDark[2]), sRGBEOTF(cLight[2]), aLight)),
		];

	}

	// ---- confidence-gated blend with the original ----

	const wCoh = smoothstep(cohMin, cohMin + 0.15, coh);
	const fitQ = 1.0 - smoothstep(0.25, 0.55, rms);
	const wSw = smoothstep(0.2, 0.7, Sw);
	const wRange = smoothstep(threshold, threshold * 1.3, range);
	const wAmp = smoothstep(range * 0.15, range * 0.35, amp2);

	const conf = (((((wCoh * fitQ) * wSw) * wRange) * wAmp) * wFlat) * strength;

	// small phase-stable tent admixture
	const tentRGB = v3add(v3scale(v3add(v3add(v3add(cN, cS), cE), cW), 0.15), v3scale(colorC, 0.4));
	const outRGB = v3mix(colorC, v3mix(rgbNew, tentRGB, tentMix), conf);

	// ---- debug views ----

	let finalRGB = outRGB;
	if (debug > 0) {

		const dbg1 = [conf, conf, conf];
		const dbg2 = [aLight, aLight, aLight];
		const dv3 = clamp(d0 * 0.5 + 0.5, 0.0, 1.0);
		const dbg3 = [dv3, dv3, dv3];
		const dbg4 = [n2[0] * 0.5 + 0.5, n2[1] * 0.5 + 0.5, coh];
		const dv5 = clamp(rms * 2.0, 0.0, 1.0);
		const dbg5 = [dv5, dv5, dv5];

		finalRGB = (debug === 1) ? dbg1
			: ((debug === 2) ? dbg2
				: ((debug === 3) ? dbg3
					: ((debug === 4) ? dbg4 : dbg5)));

	}

	if (T) {
		T('u0', u0); T('m', m); T('rms', rms); T('Sw', Sw); T('lMid', lMid);
		T('amp2', amp2); T('wFlat', wFlat);
		T('nBase', `${nBase[0]},${nBase[1]}`);
		T('tBase', `${tBase[0]},${tBase[1]}`);
		T('d0', d0); T('width', width); T('aLE', aLE); T('aLight', aLight);
		T('conf', conf);
		T('out.r', finalRGB[0]); T('out.g', finalRGB[1]); T('out.b', finalRGB[2]);
	}

	return finalRGB; // alpha (colorC.a = 1.0) is dropped on write

}

// -------------------------------------------------------------------- main

function usage() {
	process.stderr.write('usage: node icaa-cpu.mjs <in.ppm> <out.ppm> <quality|fast> [debug N] [probe X Y]\n');
	process.exit(2);
}

function main() {
	const args = process.argv.slice(2);
	if (args.length < 3) usage();
	const [inPath, outPath, preset] = args;
	if (preset !== 'quality' && preset !== 'fast') usage();

	let debug = 0;
	let probe = null;
	let i = 3;
	while (i < args.length) {
		if (args[i] === 'debug' && i + 1 < args.length) {
			debug = parseInt(args[i + 1], 10);
			if (!Number.isInteger(debug)) usage();
			i += 2;
		} else if (args[i] === 'probe' && i + 2 < args.length) {
			probe = [parseInt(args[i + 1], 10), parseInt(args[i + 2], 10)];
			if (!Number.isInteger(probe[0]) || !Number.isInteger(probe[1])) usage();
			i += 3;
		} else {
			usage();
		}
	}

	const img = readPPM(inPath);
	const W = img.w;
	const H = img.h;
	const sample = makeSampler(img);
	const sampleColor = (q) => sample(q[0], q[1]);
	const texel = [1.0 / W, 1.0 / H];

	const fast = preset === 'fast';
	const cfg = {
		fast,
		// uniform defaults from ICAANode's constructor
		contrastAbs: 0.02,
		contrastRel: 0.08,
		cohMin: 0.48,
		strength: 1.0,
		widthK: 0.0,
		widthBase: fast ? 1.3 : 1.2,
		tentMix: fast ? 0.3 : 0.25,
		debug,
	};

	if (probe) {
		const [X, Y] = probe;
		if (X < 0 || X >= W || Y < 0 || Y >= H) {
			process.stderr.write(`probe (${X},${Y}) out of bounds for ${W}x${H} image\n`);
			process.exit(2);
		}
		const uv = [(X + 0.5) / W, (Y + 0.5) / H];
		const lines = [];
		applyICAA(sampleColor, uv, texel, cfg, (name, value) => lines.push(`${name}=${value}`));
		process.stdout.write(lines.join('\n') + '\n');
		return;
	}

	const out = new Float64Array(W * H * 3);
	for (let py = 0; py < H; py++) {
		for (let px = 0; px < W; px++) {
			const uv = [(px + 0.5) / W, (py + 0.5) / H];
			const rgb = applyICAA(sampleColor, uv, texel, cfg, null);
			const o = (py * W + px) * 3;
			out[o + 0] = rgb[0];
			out[o + 1] = rgb[1];
			out[o + 2] = rgb[2];
		}
	}
	writePPM(outPath, W, H, out);
}

main();
