// Compares the wasm32 build of skate-wasm-parity, run here in V8, with the native runs that
// parity.sh wrote to <dir>: every engine and cost digest must match each native run (hardware FMA
// and SKATE_AUDIO_FMA=0); libm sections report how many outputs differ and by how many ulps.
// Times are best of three on both sides.
//
//   node tools/web-port/parity.mjs target/web-parity
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';

const dir = process.argv[2];
if (!dir) {
  console.error('usage: parity.mjs <dir with native.txt, native-plain.txt, values/ and node/>');
  process.exit(2);
}
const wasm = createRequire(import.meta.url)(resolve(dir, 'node', 'skate_wasm_parity.js'));

// `# native x86_64 audio-fma=fma` then `name kind digest ms` lines.
function readRun(file) {
  const run = { label: '', sections: new Map() };
  for (const line of readFileSync(file, 'utf8').split('\n')) {
    if (line.startsWith('# ')) run.label = line.slice(2);
    else if (line.trim()) {
      const [name, kind, digest, ms] = line.split(' ');
      run.sections.set(name, { kind, digest, ms: Number(ms) });
    }
  }
  return run;
}
const natives = ['native.txt', 'native-plain.txt']
  .map((file) => join(dir, file))
  .filter(existsSync)
  .map(readRun);
if (natives.length === 0) throw new Error(`${dir}: no native.txt; run parity.sh`);

// Distance in ulps between two floats given as ordered bit patterns (BigInt, any width).
function ordered(bits, sign) {
  return bits & sign ? sign - (bits ^ sign) : sign + bits;
}
function libmDifference(name, width) {
  const file = join(dir, 'values', `${name.replace('/', '-')}.bin`);
  const bytes = readFileSync(file);
  const native = new Uint32Array(bytes.buffer, bytes.byteOffset, bytes.length / 4);
  const ours = wasm.values(name);
  if (ours.length !== native.length) return `length ${ours.length} vs ${native.length}`;
  const words = width / 32;
  const sign = 1n << BigInt(width - 1);
  let differ = 0;
  let worst = 0n;
  for (let i = 0; i < ours.length; i += words) {
    let a = 0n;
    let b = 0n;
    for (let w = words - 1; w >= 0; w--) {
      a = (a << 32n) | BigInt(ours[i + w]);
      b = (b << 32n) | BigInt(native[i + w]);
    }
    if (a === b) continue;
    differ++;
    const d = ordered(a, sign) - ordered(b, sign);
    const ulps = d < 0n ? -d : d;
    if (ulps > worst) worst = ulps;
  }
  const values = ours.length / words;
  return differ === 0
    ? `identical (${values} values)`
    : `${differ}/${values} differ (${((100 * differ) / values).toFixed(3)}%), max ${worst} ulp`;
}

let failures = 0;
const rows = [];
for (const line of wasm.sections().trim().split('\n')) {
  const [name, kind] = line.split(' ');
  let digest = '';
  let best = Infinity;
  for (let i = 0; i < 3; i++) {
    const started = performance.now();
    digest = wasm.run(name);
    best = Math.min(best, performance.now() - started);
  }
  const native = natives[0].sections.get(name);
  let result;
  if (kind.startsWith('libm')) {
    result = libmDifference(name, kind === 'libm-f64' ? 64 : 32);
  } else {
    const mismatched = natives.filter((run) => run.sections.get(name)?.digest !== digest);
    if (mismatched.length) failures++;
    result = mismatched.length
      ? `MISMATCH vs ${mismatched.map((run) => run.label).join(', ')}`
      : `match ${digest}`;
  }
  const times = natives.map((run) => run.sections.get(name)?.ms.toFixed(1) ?? '-');
  const ratio = native ? (best / native.ms).toFixed(2) : '-';
  rows.push([name, kind, ...times, best.toFixed(1), ratio, result]);
}

// `native x86_64 audio-fma=plain` → `plain ms`: the audio loops' copy differs between runs.
const runHeaders = natives.map((run) => `${run.label.split('audio-fma=')[1] ?? 'native'} ms`);
const header = ['section', 'kind', ...runHeaders, 'wasm ms', `wasm/${runHeaders[0].split(' ')[0]}`, 'result'];
const widths = header.map((h, i) => Math.max(h.length, ...rows.map((r) => r[i].length)));
const format = (row) => row.map((cell, i) => cell.padEnd(widths[i])).join('  ').trimEnd();
console.log(`wasm32 (V8 ${process.versions.v8}) against ${natives.map((run) => run.label).join(' | ')}`);
console.log(format(header));
for (const row of rows) console.log(format(row));
console.log(failures ? `${failures} section(s) MISMATCH` : 'All engine and cost sections match bit for bit.');
process.exitCode = failures ? 1 : 0;
