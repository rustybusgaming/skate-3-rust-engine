# Web port tools

Building and checking the browser build ([`docs/web-port/`](../../docs/web-port/README.md)). They use no game data.

| File | What it does |
|---|---|
| `parity.sh` | Builds `skate-wasm-parity` natively and for `wasm32-unknown-unknown`, runs it natively twice (audio on its hardware-FMA copy, then `SKATE_AUDIO_FMA=0`) and as wasm in V8, and compares every digest. Exit 1 on any engine or cost mismatch. |
| `parity.mjs` | The comparison and report half of `parity.sh`: runs the wasm build, reads the native runs, counts libm differences in ulps. |
| `build.sh` | Builds the browser package into `target/web/dist`: the `web` cargo profile, then `wasm-bindgen` without the function-name section (`SKATE_WEB_KEEP_NAMES=1` keeps it). Adds the loader page with the build's commit and source link, its installation reader, and gzip (and brotli) copies for `skate-web-server`. Refuses uncommitted changes, so the GPL source link always matches. |
| `compress.mjs` | Writes the gzip and brotli copies `build.sh` puts beside each served file. |
| `smoke.mjs` | Serves that package on localhost and drives it in headless Chromium with WebGPU: the folder picker, a folder with no installation, and a synthetic installation (no game data) that startup must load up to its placeholder input config, retrying once for a withheld file. On that page, a texture pack in the package must replace the demo map's checker (read on a retry once the map is parsed), a fake standard pad must reach controller slot 0 through the Gamepad API, and audio must keep its schedule across a main-thread stall. Fails on a panic, console error or page error. Needs Playwright. |
| `installation.test.mjs` | `node --test` checks of the page's installation finder and store (`web/installation.js`). |

The server itself is `crates/skate-web-server` ([docs/web-port/server.md](../../docs/web-port/server.md)): `cargo run --release -p skate-web-server`.
| `wasi-test-runner.mjs` | Cargo runner that executes `wasm32-wasip1` test binaries on Node's built-in WASI, so the stock test suites run as wasm. |

## Requirements

- `rustup target add wasm32-unknown-unknown wasm32-wasip1`
- `cargo install wasm-bindgen-cli --version =0.2.127 --locked` (the version in `Cargo.lock`)
- Node 22, and `clang` for zstd-sys's C sources on wasm32

## Usage

```
tools/web-port/build.sh            # the browser package, in target/web/dist
cargo run --release -p skate-web-server   # serve it at http://127.0.0.1:8080/
node tools/web-port/smoke.mjs      # drive it in headless Chromium (and through the server, once built)
node --test tools/web-port/installation.test.mjs

tools/web-port/parity.sh

export CARGO_TARGET_WASM32_WASIP1_RUNNER="node --no-warnings $PWD/tools/web-port/wasi-test-runner.mjs"
cargo test --target wasm32-wasip1 -p skate-core --lib --tests -- --test-threads=1
```

On Windows, run `parity.sh` from Git Bash. Outputs go to `target/web-parity/`.

## Example output

```
wasm32 (V8 12.4.254.21-node.33) against native x86_64 audio-fma=fma | native x86_64 audio-fma=plain
section                  kind      fma ms  plain ms  wasm ms  wasm/fma  result
core/trigonometry        engine    135.7   128.9     127.9    0.94      match cd7e0ee54bee0e05
core/board-ramp          engine    294.3   283.4     324.5    1.10      match 354a54b3c6b4ea96
audio/voice-chain        engine    41.8    150.9     140.9    3.37      match f5b8e418154618e8
...
libm/f32-tan             libm-f32  5.8     6.2       4.9      0.84      37887/262144 differ (14.453%), max 1 ulp
All engine and cost sections match bit for bit.
```
