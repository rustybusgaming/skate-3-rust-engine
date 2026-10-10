// Cargo runner for wasm32-wasip1 test binaries on Node's built-in WASI (no wasmtime needed).
// libtest cannot run on wasm32-unknown-unknown (its timeout clock calls `Instant::now`), so the
// stock test suites run here instead: same wasm instructions, same software `fmaf`.
//
//   CARGO_TARGET_WASM32_WASIP1_RUNNER="node --no-warnings $PWD/tools/web-port/wasi-test-runner.mjs"
//   cargo test --target wasm32-wasip1 -p skate-core
//
// The host root is preopened as `/`, so tests that read fixtures through `CARGO_MANIFEST_DIR`
// see the same absolute paths they do natively.
import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';

const [wasmPath, ...args] = process.argv.slice(2);
if (!wasmPath) {
  console.error('usage: wasi-test-runner.mjs <test.wasm> [libtest args...]');
  process.exit(2);
}
const wasi = new WASI({
  version: 'preview1',
  args: [wasmPath, ...args],
  env: process.env,
  preopens: { '/': '/' },
  returnOnExit: true,
});
const module = await WebAssembly.compile(await readFile(wasmPath));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
// A panic aborts (wasm is panic=abort), which surfaces here as a trap.
try {
  process.exitCode = wasi.start(instance);
} catch (error) {
  console.error(`${wasmPath}: ${error}`);
  process.exitCode = 101;
}
