#!/usr/bin/env node
// ===========================================================================
// compare.mjs — compare two plain-text P3 PPM images by 8-bit RGB values.
//
// Usage: node compare.mjs a.ppm b.ppm
//
// Prints: max_diff=N mismatched=M total=T
//   max_diff   — maximum absolute difference over all RGB components
//   mismatched — number of RGB components that differ
//   total      — total number of RGB components compared (w*h*3)
// Exit status is 0 iff max_diff == 0.
// ===========================================================================

import { readFileSync } from 'node:fs';

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
	const data = new Int32Array(n);
	for (let i = 0; i < n; i++) {
		const v = Number(tokens[4 + i]);
		if (!Number.isFinite(v)) throw new Error(`${path}: bad pixel value '${tokens[4 + i]}'`);
		data[i] = v;
	}
	return { w, h, data };
}

const args = process.argv.slice(2);
if (args.length !== 2) {
	process.stderr.write('usage: node compare.mjs a.ppm b.ppm\n');
	process.exit(2);
}

const A = readPPM(args[0]);
const B = readPPM(args[1]);

if (A.w !== B.w || A.h !== B.h) {
	process.stderr.write(`size mismatch: ${A.w}x${A.h} vs ${B.w}x${B.h}\n`);
	process.exit(1);
}

const total = A.w * A.h * 3;
let maxDiff = 0;
let mismatched = 0;
for (let i = 0; i < total; i++) {
	const d = Math.abs(A.data[i] - B.data[i]);
	if (d !== 0) {
		mismatched++;
		if (d > maxDiff) maxDiff = d;
	}
}

process.stdout.write(`max_diff=${maxDiff} mismatched=${mismatched} total=${total}\n`);
process.exit(maxDiff === 0 ? 0 : 1);
