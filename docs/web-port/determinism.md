# Determinism on wasm32

The physics in `skate-core` and the DSP in `skate-audio` reproduce the console bit for bit natively.
The browser build has to give the same bits, or the web game is a different game. This page records
how that was checked, the results, and the one platform dependence found.

## Why `mul_add` is safe

`f32::mul_add` is the exactly rounded `a·b + c` on every target: natively it is the CRT's `fmaf`
(the release build targets baseline x86-64, no `+fma`) or, in `skate-audio-fma`'s FMA copy, a
`vfmadd` instruction; on wasm32 it is compiler-builtins' software `fmaf`. Exactly rounded means one
possible answer, so the bits cannot differ. Rust never contracts `a * b + c` into an FMA by itself, and
wasm has no fused instruction that could be substituted (relaxed-SIMD `madd` may or may not fuse,
which is why it is ruled out). Only the speed changes; it is measured below.

## Method

Two independent checks, both against the same native build:

1. **The stock test suites, run as wasm.** `wasm32-wasip1` under Node's built-in WASI
   ([`tools/web-port/wasi-test-runner.mjs`](../../tools/web-port/wasi-test-runner.mjs)); libtest
   cannot run on `wasm32-unknown-unknown`, where its timeout clock panics. Same wasm instructions,
   same software `fmaf`.
2. **A bit-exact digest probe on the shipping target.** `crates/skate-wasm-parity` drives production
   code over fixed synthetic inputs and hashes every output: float bits for sample streams, `Debug`
   text (shortest round-trip, so also exact) for structured state. NaNs are canonicalised, because wasm
   may change a NaN's sign and payload and nothing else. The native binary runs twice (audio on its
   hardware-FMA copy, then `SKATE_AUDIO_FMA=0` for the plain copy); the `wasm32-unknown-unknown` build
   runs in V8 through `wasm-bindgen`. [`tools/web-port/parity.sh`](../../tools/web-port/parity.sh)
   does all of it.

No EA data is involved: inputs are generated, and the map is the procedural `maps/format-demo.skate`.

## Results

x86-64 Linux, Rust 1.97.0, Node 22.22.0 (V8 12.4), base commit `2e16697` plus the milestone 1 changes.

### Test suites

| Crates | Tests | Native vs wasm |
|---|---|---|
| `skate-core`, `skate-audio`, `skate-audio-fma`, `skate-dynamics` | 1,176 (1,106 run, 70 ignored for private data) | Identical outcome for every test |
| `skate-net` | 37 (the three real-UDP tests are native-only) | Identical |
| `skate-data` | — | Not run under WASI: zstd-sys needs a WASI sysroot there. The probe covers it on the shipping target. |

The one test that fails natively on the base commit (see the [baseline](README.md#native-baseline))
fails as wasm at the same assertion.

### Bit-exact probe

| Section | What it drives | Native (FMA) ms | Native (plain) ms | wasm ms | Result |
|---|---|---|---|---|---|
| `core/trigonometry` | Retail XNA sin/cos/acos/asin polynomials, 2^19 angles | 135.7 | 128.9 | 127.9 | match |
| `core/board-ramp` | Stock board assembly, 20 s at 60 Hz up a ramp and back: world wheel contacts, pushes, steering, 25-iteration solver, integration; every contact and body state each tick | 294.3 | 283.4 | 324.5 | match |
| `audio/dsp-kernels` | Resampler, biquad kernel, FSS polynomials (the per-sample `mul_add` loops) | 30.1 | 72.3 | 69.4 | match |
| `audio/voice-chain` | Resampler → low/high-pass → shelf → peaking → delay → gain → frequency shift / reverb, moving parameters, 10.7 s stereo | 41.8 | 150.9 | 140.9 | match |
| `audio/mixmap-tables` | MixMap tables (generated with f64 `powf` / `cos`) | 0.1 | 0.1 | 0.5 | match |
| `data/format-demo` | `SkateMap::parse` of `maps/format-demo.skate` | 0.0 | 0.0 | 0.0 | match |
| `data/codecs` | zstd (C, through zstd-sys's wasm shim) and deflate round trips, 4 MiB | 303.1 | 298.7 | 375.5 | match |
| `cost/mul-add` | 8 chains × 2^21 `mul_add` | 52.3 | 49.1 | 48.1 | match |
| `cost/mul-then-add` | The same with a separate multiply and add (timing only) | 5.1 | 5.0 | 8.9 | match |

**Every engine section matches both native runs bit for bit.**

### The platform libm

The std transcendentals are not `mul_add`: they call the platform's libm (UCRT on Windows, glibc on
Linux, Rust's `libm` port on wasm32), which are not correctly rounded and need not agree. Swept at the
types and ranges the engine uses (2^18 inputs each):

| Function | Engine callers | Native (glibc) vs wasm |
|---|---|---|
| f32 `sin`, `cos` | `skate-audio` panner, splice envelopes | 1.3% differ, max 1 ulp |
| f32 `sin_cos` | `skate-core` manual camera; `skate-audio` player heading | 1.2% differ, max 1 ulp |
| f32 `tan` | `skate-core` offboard air selector | 14.5% differ, max 1 ulp |
| f32 `atan2` | `skate-core` `BoardAdjustAngle`; `skate-audio` panner, object azimuth | 0.27% differ, max 1 ulp |
| f32 `acos` | `skate-audio` zones | 2.3% differ, max 1 ulp |
| f64 `sin` / `cos` rounded to f32 | `skate-audio` filter design | identical |
| f64 `powf` | `skate-core` trigger volume flatness threshold | 9.2% differ, max 1 ulp |

So at these call sites the browser can be one ulp away from native. Each native platform already
links its own libm, so Linux and Windows builds can differ there too. None of these sites sits inside
a `mul_add` chain. The engine sections above stay bit-exact because they either don't reach these
sites (the board run never calls the air selector, camera or trigger code) or round the f64 result
to f32 or an integer, which absorbs the difference (filter design, MixMap tables). That is not a
guarantee for a whole play session.

**Decision needed (not taken here):** making these call sites identical everywhere means routing them
through one implementation on every target (for example the pure-Rust `libm` crate). That changes
native results by up to one ulp at those sites, which the porting rules forbid without approval. Until
then the difference is documented, not hidden. Multiplayer is unaffected because `skate-net` sends
snapshots and doesn't run lockstep; input replays recorded natively and re-simulated in a browser
(or the reverse) could drift.

Not yet measured: Windows UCRT against wasm. Run `tools/web-port/parity.sh` from Git Bash on Windows.

## Cost of software `fmaf`

- Each `mul_add` costs the same as natively, where it is already a library call (`cost/mul-add`: 48 ms
  wasm, 49–52 ms native for 16.8 M). It is about 10× a separate multiply and add on both.
- Physics: `core/board-ramp` runs at 1.10× native, the trig polynomials at 0.94×.
- Audio: wasm matches native's plain copy (69 vs 72 ms; 141 vs 151 ms). The 2.3–3.4× gap to native's
  hardware-FMA copy is that instruction alone. The full voice chain still renders 10.7 s of stereo
  in 141 ms (1.3% of real time per voice chain).

Nothing here justifies trading correctness for speed. If audio becomes the bottleneck, the
answer is fewer software `fmaf` calls in the same order (for example batching voices), not a
different rounding.

## Rerunning

```
rustup target add wasm32-unknown-unknown wasm32-wasip1
cargo install wasm-bindgen-cli --version =0.2.127 --locked
tools/web-port/parity.sh                     # probe: native ×2 vs wasm32 in V8, exit 1 on a mismatch

export CARGO_TARGET_WASM32_WASIP1_RUNNER="node --no-warnings $PWD/tools/web-port/wasi-test-runner.mjs"
cargo test --target wasm32-wasip1 -p skate-core -p skate-audio -p skate-audio-fma -p skate-dynamics \
  --lib --tests -- --test-threads=1 \
  --skip a_moving_group_8_body_reaches_native_impact_feedback_for_a_stationary_actor
```

A failing test aborts its whole wasm test binary (wasm panics abort), so the known native failure is
skipped by name.
