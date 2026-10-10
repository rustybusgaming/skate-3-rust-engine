# Browser build

`skate-game` compiles for `wasm32-unknown-unknown` and renders through WebGPU. In a browser it
starts, brings up a WebGPU adapter and the canvas, and stops at the configuration: the player's own
converted installation can't be chosen in the page yet (milestone 3), so it says so on screen.
Everything below is behind `cfg(target_arch = "wasm32")` or a wasm-only dependency; the native
build is unchanged.

## Building and checking

```
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version =0.2.127 --locked
tools/web-port/build.sh            # -> target/web/dist: index.html, skate3rust.js, skate3rust_bg.wasm
node tools/web-port/smoke.mjs      # boots it in headless Chromium with WebGPU
```

`build.sh` uses the `web` cargo profile (release code without debug info) and stamps the commit and
the source repository (this checkout's `origin`, or `SKATE_SOURCE_URL`) into the loader page's
GPL source link. It refuses a tree with uncommitted changes, since the link would not match the
build; `SKATE_WEB_ALLOW_DIRTY=1` is for local testing only.

`smoke.mjs` serves the package on `localhost`, a secure context, so WebGPU is exposed. It needs
`SKATE_WEB_READY` (an adapter is up) and `SKATE_WEB_STOPPED` (why startup stopped) in the console,
and fails on a panic, a console error or a page error. It writes a screenshot to `target/web/smoke.png`.

Results at this milestone (Rust 1.97.0; Playwright's Chromium with SwiftShader WebGPU):

| | |
|---|---|
| `web` profile build of the engine (4 cores, dependencies cold) | 9 min 19 s |
| `skate3rust_bg.wasm` | 130.5 MB raw, **21.4 MB gzip** (what a static host sends) |
| Headless boot to `SKATE_WEB_READY` + `SKATE_WEB_STOPPED` | 3.7 s, no console error, panic or page error |
| Native `skate-game` tests | identical outcome for all 674 tests (the setup fingerprint failure predates the port) |
| Native `skate-mods` tests | 106 passed (`skyline_physics` needs `sdk/examples/skyline/skyline.glb`, which `.gitignore` keeps out of every checkout) |

What the smoke test cannot show: pixels. Headless Chromium here composites no WebGPU canvas into
screenshots or `drawImage` copies (even a canvas cleared to red reads back blank), so the stop
screen's look was not checked. Its pipelines compiled and ran without a WebGPU error, which the
console check would have caught.

## Hosting

The package is static files, so any static host works. WebGPU only runs on HTTPS pages (or
`localhost`). GitHub Pages fits (1 GB per site) when deployed from Actions; a 130 MB file is over
git's 100 MB push limit, so it is not committed. Cloudflare Pages caps a single file at 25 MiB:
put `skate3rust_bg.wasm` on R2 or use another host. Shrinking the module (`wasm-opt`, size-tuned
codegen) belongs with the server and hosting work (milestone 4). Never upload a converted
installation anywhere public.

## What is gated, and why

| Area | Browser build | Native |
|---|---|---|
| Rendering | `Backends::BROWSER_WEBGPU` on the page's `#skate3` canvas, sized to the page; Bevy's `webgpu` feature (wasm-only dependency) | Vulkan, 1280×800 window |
| Logging | Bevy's `LogPlugin` → browser console | `profiling.rs` owns tracing (Chrome trace file); `LogPlugin` off |
| Process-level startup | none: no updater recovery, setup tool modes, crash supervisor or trace file | unchanged |
| Installation | `setup::asset_root` reports that none is loaded (milestone 3) | `--assets`, `SKATE3_ASSETS`, `data/installation.json` |
| Controllers | no backend yet; the Gamepad API arrives in milestone 6 | SDL3 (XInput fallback on Windows) |
| Lua mods | none: `mlua`'s vendored C Lua does not target wasm32-unknown-unknown. `skate_mods::Vm` is uninhabited there and package validation says why | unchanged |
| Asset sources | `mods`, `characters`, `online-characters` are not registered (no file system) | `FileAssetReader` sources |
| Multiplayer | UDP sockets and the Steam relay helper fail to open (browsers have neither); the WebSocket relay is milestone 5 | unchanged |

## Not yet safe at runtime

These compile, and nothing in the browser reaches them until the game itself boots (milestone 3),
where each needs a browser path:

- `std::time::Instant::now` panics on wasm32-unknown-unknown: 99 uses across 26 files. Bevy's
  `bevy::platform::time::Instant` is the same type natively and `web-time` in the browser.
- Threads: `std::thread::spawn` fails there (the map build worker, the frame-timing log writer).
- Path-based loading (`skate_data` loaders, settings files) needs the installation in memory.
- `skate-dynamics` wraps convex decomposition in `catch_unwind`, which cannot catch on wasm
  (panics abort), so bad input must be rejected before it.
