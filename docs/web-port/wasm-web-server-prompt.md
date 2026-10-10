# Prompt: port the Skate 3 Rust Engine to WebAssembly and serve it from a web server

Copy everything below the line into a coding agent that has this repository checked out.

---

## Your role and goal

You are a senior Rust, WebAssembly and WebGPU engineer working in the `skate-3-rust-engine` repository, a
GPL-3.0-only Rust and Bevy reimplementation of Skate 3 built from reverse-engineering research. Today it ships only
as a native Windows executable (`skate3rust.exe`, Vulkan, SDL3 gamepads).

Your goal: make the game playable in a desktop browser by

1. compiling the engine to `wasm32-unknown-unknown` with Bevy's WebGPU backend, and
2. adding a small Rust web server that serves the WebAssembly build and its loader page, hosts multiplayer lobbies and
   relays, and (opt-in, for personal use only) serves the operator's own converted game data.

The native Windows build must keep working exactly as it does now. Every web change goes behind
`cfg(target_arch = "wasm32")`, a target-specific dependency, or a Cargo feature.

Work in the milestones at the end of this prompt. Finish each one, verify it, commit it and report it before you start
the next.

## Constraints you must not break

1. **No Electronic Arts data is ever bundled, committed or served publicly.** The README states that Skate 3 assets
   are not included and that the GPL licence grants no rights to EA's code, data or trademarks. Each player supplies
   their own converted installation, which the setup tool produces from their own disc. The server distributes only
   engine code and original project content, such as `maps/format-demo.skate`, which is procedural. The only exception
   is the opt-in operator asset mount in the server section: it is off by default, binds to localhost, and requires a
   token.
2. **GPL-3.0-only applies to the served `.wasm` file.** Serving it is binary distribution. The loader page must link
   to the corresponding source for the exact commit. `crates/skate-game/build.rs` already embeds
   `SKATE_RELEASE_REVISION`, so use it to build the link. Keep all licence headers and the vendored crates' licences.
3. **Do not change retail parity.** The physics in `skate-core` and the audio DSP in `skate-audio` reproduce the
   console bit for bit. Keep every `f32::mul_add` (there are about 1,245 in `skate-core` and `skate-audio`):
   - Do not rewrite it as `a * b + c`.
   - Do not use relaxed-SIMD `madd`, which may or may not fuse.

   On wasm, `mul_add` compiles to a software `fmaf`. That is exactly rounded, so the results stay identical; only the
   speed changes. Measure the cost; don't trade correctness for speed.
4. **Native behaviour stays the same.** Leave these unchanged: the Windows release workflow
   (`.github/workflows/release.yml`), `scripts/Build*.ps1`, the static-CRT release build, the SDL3/XInput input path
   and the Vulkan backend selection. Re-run the native checks listed below after every milestone.
5. **Match the code around you.** Use the repository's comment density, naming and idioms: terse `//!` module docs
   that explain why, `Result<_, String>` errors that name the file, and no new `unsafe` in crates that forbid it
   (`skate-data` has `#![forbid(unsafe_code)]`).
6. **Know where docs go.** `.gitignore` ignores `/docs/*.md` at the top level, so put new documentation in
   `docs/web-port/`. Pin new dependencies exactly (`=x.y.z`), the way the workspace already pins Bevy, and keep
   `Cargo.lock` committed.

## What the codebase looks like

This was surveyed from the current tree. Verify it before you rely on it, and keep this map up to date in
`docs/web-port/README.md` as you work.

### Workspace (`Cargo.toml`, edition 2024, resolver 3, Bevy pinned at `=0.18.1`)

| Crate | Role | Web relevance |
|---|---|---|
| `skate-core` (~93k lines) | Physics, state graphs, input conversion, maths. Pure Rust, no I/O. | Should compile to wasm almost unchanged. Prove determinism there first. |
| `skate-data` | Stock data loaders, the `.skate` map format and XEX reading. Depends on `flate2` and `zstd`. | `zstd` is a C library. Runtime use is decode only (`src/skate_map.rs` ~line 340); encoding appears only in tests. Has a threaded texture decode. |
| `skate-audio` + `skate-audio-fma` | Native port of retail AEMS audio: evaluator, voice graph, MixMap, DSP. | Pure Rust. The FMA crate already gates its AVX path with `cfg(target_arch = "x86_64")` and falls back to `Plain`. |
| `skate-dynamics` | Prop and deformation dynamics on `rapier3d =0.35.3`. | Works on wasm. |
| `skate-mods` | Lua 5.4 mod VM: `mlua` with `vendored` and `send`, plus `zip` and `gltf::import`. | `mlua`'s vendored C Lua does not target `wasm32-unknown-unknown`. Uses temp dirs and `std::process::id`. |
| `skate-net` | Transport-neutral snapshot protocol: `lobby` (10-player star), `directory` (lobby commands), `packed` (MTU), prediction and interpolation. | `socket2` won't compile for wasm. Uses `std::time::Instant`. |
| `skate-steam-relay` | Steamworks helper executable. | Exclude it from web builds. |
| `skate-game` (~121k lines) | The Bevy app. Binaries `skate3rust`, `skate3-multiplayer` and `skate3-custom-models`. All modules are declared in `src/main.rs`. | The main porting target. |

Patched Bevy crates live in `vendor/bevy_pbr`, `vendor/bevy_core_pipeline` and `vendor/sdl3-sys`. They're wired in
through `[patch.crates-io]` and explained in `vendor/README.md`. The Bevy patches must still compile and behave the
same on the WebGPU backend.

### Startup path (`crates/skate-game/src/main.rs`, `app.rs`, `config.rs`, `setup.rs`)

- `main` loads everything synchronously from disk before it builds the `App`:
  - `config::Config::from_env` reads CLI args and env vars, finds the asset root through `setup::asset_root()`, and
    loads the saved default map.
  - `StockGameplayConfig::load`, `GameAssets::load` (`private/game.json`), `StockGraphs::load`,
    `GamePhysics::load_with_difficulty`, `SkaterRuntime::load` and `PlayerControls::load`.
- `setup::asset_root()` finds the install in one of three ways: `--assets`, `SKATE3_ASSETS`, or
  `current_exe()/data/installation.json`, which can spawn `support/skate3setup.exe`.
- An installation is `data/installations/<32-hex>/` and holds `assets/` (the asset root), `maps/*.skate`, and
  `settings/*.json` (`default-map`, `gameplay`, `graphics`, `audio`, `controller`, `character`, `player`). Code reaches
  `maps/` and `settings/` through `asset_root.parent()`.
- `app::build` has three things the web build must change:
  - it forces `Backends::VULKAN` and `InstanceFlags::empty()`;
  - it sets a fixed 1280×800 window;
  - it disables `LogPlugin` (`profiling.rs` owns tracing) and `GilrsPlugin`.

  It also registers two `FileAssetReader` asset sources, `mods` and custom models (`modding::register_source`,
  `custom_models::register_source`). `FileAssetReader` doesn't exist on wasm.
- Windows-only or process-level modules:
  - `updater.rs`;
  - `crash_report.rs` and `crash_native.rs` (a supervisor process and `ntdll`);
  - `profiling.rs` (Chrome trace written to a file);
  - `verification.rs` (`--verify` PNG capture);
  - `frame_timing/log.rs` (log-writer thread);
  - the setup tool modes `--extract-ocean-pca` and `--extract-menu-tables`;
  - `build.rs` (the Windows `.rc` icon step, which already returns early for other targets).

### Blockers for `wasm32-unknown-unknown`, with locations

- **Filesystem.** About 450 `std::fs` call sites outside tests, across `skate-game` (~58 files), `skate-data`,
  `skate-audio` and `skate-mods`. All of them are synchronous.
- **Threads.** `std::thread` spawns fail at run time on wasm. They're in:
  - `map_transition.rs` (~112 `map-loader`, ~128 scoped `map-render-loader`);
  - `skate-data/src/skate_map/texture_decode.rs` (scoped decode workers);
  - `game_audio/native/prefetch.rs` (bank prefetch);
  - `game_audio/state_log.rs`;
  - `frame_timing/log.rs`;
  - `input/platform.rs` (the SDL pump thread);
  - `crash_report.rs`.
- **Time.** `std::time::Instant::now()` and `SystemTime::now()` panic on wasm. They're in about 45 files across
  `skate-game`, `skate-data`, `skate-audio`, `skate-dynamics`, `skate-mods` and `skate-net`.
- **Processes and environment.** About 40 files use `std::process` or `Command::new`: setup, updater, Steam relay,
  crash supervisor, and the temp-dir tests in `skate-mods`. About 100 files read `env::var` or `env::args`. Those
  return nothing on wasm, so config has to come from somewhere else.
- **Native C dependencies.**
  - `sdl3` (`build-from-source-static`) in `skate-game`;
  - `mlua` (vendored Lua) in `skate-mods`;
  - `zstd` in `skate-data`;
  - `steamworks`;
  - `socket2` in `skate-net`.
- **Rendering.** WebGPU is required; WebGL2 can't run this renderer:
  - `retail_material_bindings.wgsl` binds `var<storage, read>` arrays in the material bind group;
  - `retail_exposure.wgsl` is a `@compute` shader with a `read_write` storage buffer;
  - `retail_tone.wgsl` reads a storage buffer;
  - the vendored `bevy_pbr` uses bindless material slabs, which Bevy disables automatically when binding arrays are
    missing (WebGPU has none);
  - textures are decoded to uncompressed `Rgba8Unorm(Srgb)`;
  - MSAA is forced off (`render_capacity.rs`);
  - `SKATE_GPU_TIMING` asks for timestamp and pipeline-statistics queries, which the web doesn't have.
- **Input.** `input/platform.rs` has a `Backend` enum (`Sdl`, `XInput` on Windows, `Unavailable`). Every pad state
  is re-expressed as the XInput-shaped `skate_core::input::xbox::XboxState` through `xinput_trigger` and `xinput_y`,
  and paddles are folded in through `set_paddles`. Gameplay reads raw pad states, not Bevy gamepad events. There is no
  keyboard gameplay path.
- **Audio.**
  - Bevy `bevy_audio` (rodio/cpal) plays an endless 48 kHz stereo `NativeStream` (`game_audio/native.rs`). Its pull
    callback renders 256-frame blocks of the native runtime under an `Arc<Mutex<Runtime>>`.
  - Clips are `AudioSource { bytes }` built from decoded WAVs (`game_audio/library.rs`). Banks load on first use and
    unload on map change.
  - Decoded audio is large: setup docs cite hundreds of MB to more than 2 GB of PCM for some groups.
- **Networking.** `multiplayer/transport.rs` defines `trait Transport: Send + Sync` with `send`, `receive`, `status`,
  `command` and `events`, and implements `Direct` (UDP) and `Steam` (spawns the relay exe, then UDP on loopback).
  `skate_net::directory::{Request, Command, Response, Event}` is the lobby browse, host and join protocol. That trait
  is the right seam for a browser transport.

## Target architecture

### 1. Make `skate-game` a library

- Move the module tree and the startup logic from `src/main.rs` into `src/lib.rs`, with entry points
  `pub fn run_native() -> AppExit` and `pub fn build_app(boot: Boot) -> App`, or something equivalent.
- `src/main.rs` becomes a thin native wrapper. Keep the setup tool modes, the crash supervisor and the updater there,
  or in native-only modules.
- Keep the binary names and the outputs of `scripts/Build.ps1` unchanged.

### 2. New crate `crates/skate-vfs`

Add a small virtual filesystem and route every runtime file access in the game crates through it.

- **Native:** a zero-cost passthrough to `std::fs`, with the same paths and errors as today.
- **wasm32:** an in-memory store of path → bytes, mounted at the installation root. Its rules:
  - Synchronous `read` succeeds only for files that were prefetched. A missing file returns an error that names the
    path, so a missing preload entry shows up immediately.
  - Async `prefetch(paths)` and `prefetch_group(name)` are driven from JavaScript or from an async task.
  - `write` and `rename` persist to OPFS (Origin Private File System) through a JS bridge. Only `settings/` and
    per-user state use them.
  - Memory accounting with an explicit `evict(group)`, so map and audio data can be dropped on map change.

On wasm, register a Bevy `AssetSource` backed by the VFS as the default source, and use the same one for the `mods`
and custom-model sources in place of `FileAssetReader`.

Convert call sites mechanically, crate by crate, with no behaviour change on native. Run each crate's tests after
converting it.

### 3. Asset manifest and grouping

Add `tools/web/make_web_manifest.py`, or a Rust bin under `crates/skate-web-tools`. It reads a converted installation
and writes `web-manifest.json` listing every file's relative path, size and BLAKE3 hash, tagged with a load group:

- `boot`: what `main` reads before the app starts, plus the character scene, graphs, controls and camera data.
- `menu`: menu movies, HUD, fonts.
- `map:<name>`: one `.skate` file plus its sidecars, such as trigger volumes and map audio.
- `audio:<bank>`
- `customiser`

Derive the group membership by instrumenting the VFS in a native run (a `SKATE_VFS_TRACE=1` mode that logs every path
read, per phase). Don't guess the groups by hand. The manifest is generated on the player's machine, or in the
browser during import; it never ships with the server.

### 4. New crate `crates/skate-web` (cdylib, `wasm-bindgen`)

- It exports `start(options)`, called by the loader once the `boot` group has been prefetched. It builds the same
  `Config` from URL query parameters and saved settings in place of argv and env: `map`, `difficulty`, `mute`,
  `teleport`, `start_paused`, and multiplayer join codes.
- Install `console_error_panic_hook`. Route `eprintln!` and the `REPORT_META` and `SKATE_*` diagnostic lines to
  `console` through a tracing layer, because there is no stderr. A panic shows an in-page overlay with the message and
  a "copy report" button. This replaces the crash supervisor.
- The Bevy window binds to `#skate-canvas` with `fit_canvas_to_parent: true` and
  `prevent_default_event_handling: true`. On wasm the backend is `Backends::BROWSER_WEBGPU`. Enable Bevy's `webgpu`
  feature for the wasm target only.
- Build with `--no-default-features`, because the default `dev-dynamic` feature turns on Bevy `dynamic_linking`,
  which is impossible on wasm.
- Make sure `getrandom` uses its `wasm_js` backend: the feature, plus the `getrandom_backend` cfg in
  `.cargo/config.toml` for `wasm32-unknown-unknown` if the resolved version needs it.
- Pin `wasm-bindgen-cli` to the exact `wasm-bindgen` version in `Cargo.lock`, and run `wasm-opt -O` on release
  builds. Set the linker's initial and maximum memory deliberately. wasm32 tops out at 4 GiB, so log the
  `WebAssembly.Memory` size at each stage.

### 5. Per-subsystem port plan

**Time**
- Replace `std::time::{Instant, SystemTime}` with `web_time`, which is a re-export of `std` on native. Inside
  `skate-game` you can use `bevy::platform::time::Instant` instead.
- One mechanical commit per crate.

**Threads**
- Add a small `spawn_background` helper. On native it keeps today's `std::thread::Builder` with the same thread
  names. On wasm it runs the job inline after the needed VFS groups are prefetched.
- `map_transition.rs`: on wasm, `start()` first awaits `prefetch_group("map:<name>")`, then runs the loader on the
  main thread behind the existing progress UI. The game shows a loading screen, so a short stall is acceptable.
- `texture_decode.rs`: use `workers = 1` on wasm. The function already handles one worker.
- Audio prefetch becomes an async VFS prefetch. The state-log and frame-timing log threads and the crash thread are
  native only.
- Real wasm threads (atomics, nightly `build-std`, COOP/COEP) are a separate, optional milestone. Bevy's task pools
  are single-threaded on wasm, so don't assume they give you parallelism.

**Native C dependencies**
- `sdl3`: move it to `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.
- `socket2`: same in `skate-net`; on wasm `socket::configure` doesn't exist.
- `zstd`: either build it for wasm with clang (`zstd-sys` ships a wasm shim), or decode with the pure-Rust `ruzstd`
  on wasm only. Runtime use is decode only.
- `mlua`: put modding behind a `mods` Cargo feature that is on by default for native and off for web in the first
  milestones. The menus must say "Mods are not available in the browser build yet" rather than crash.
- Exclude `skate-steam-relay` and the `skate3-multiplayer` and `skate3-custom-models` binaries from the web build.

**Rendering**
- Keep every WGSL shader as it is. Confirm that each pipeline passes validation in Chrome's WebGPU with the browser
  console open.
- Check these limits against the default WebGPU limits, and request higher limits only where the adapter reports
  them:
  - storage buffers per stage;
  - bind groups (4);
  - `maxTextureDimension2D`;
  - `maxBufferSize`.
- Fail with a clear in-page message when `navigator.gpu` is missing, rather than falling back to WebGL2.
- Optional later work: BC texture compression when the adapter has `texture-compression-bc`, which cuts VRAM.

**Input**
- Add `Backend::Web` to `input/platform.rs`. It polls `navigator.getGamepads()` once per frame (use only pads with
  `mapping === "standard"`) and converts each pad to `XboxState` with the same semantics as the SDL path:
  - Buttons 0–3 map to A, B, X and Y. Then LB, RB, LT and RT as analog, Back, Start, LS, RS, and D-pad 12–15.
  - Axes −1..1 scale to `i16`. Y is inverted to XInput's positive-up convention.
  - Triggers go from `buttons[6|7].value` (0..1) to a `u8`.
- Write round-trip unit tests in the same style as `every_trigger_byte_survives_the_sdl_round_trip`.
- Pads appear only after a button press, so show "Press any button on your controller".
- An optional keyboard mapping may come later, behind a setting. It must never change gamepad behaviour.

**Audio**
- Keep `bevy_audio`; rodio and cpal use Web Audio on wasm.
- Browsers start audio only after a user gesture, so the loader's "Click to play" button must create or resume the
  context before Bevy starts.
- The `NativeStream` pull callback runs on the main thread on wasm. Measure underruns, and if needed raise the buffer
  size. Moving to an AudioWorklet is a later option.
- Load banks lazily through the VFS. If the audio group is missing, run silent with one logged error, the way the
  native game already does when audio data is missing.

**Multiplayer**
- Add `transport::Web` that implements `Transport` over a browser WebSocket connected to the server's `/net`
  endpoint:
  - binary frames, each one datagram, at most `skate_net::packed::MTU`;
  - `receive()` drains a queue that `onmessage` fills;
  - `command()` and `events()` speak `skate_net::directory` JSON to the server's lobby directory, which replaces
    Steam lobbies;
  - `status()` reports the connection state.
- Leave `skate_net::lobby` (host admits and forwards, `MAX_PLAYERS = 10`) and prediction and interpolation
  unchanged.
- The `Transport: Send + Sync` bound may need `cfg`-specific handling, because `web_sys` handles are `!Send`. On wasm
  it's fine to keep the socket in a non-Send resource or a `thread_local`.

**Persistence**
- Writes to `settings/*.json` and mod preferences go through the VFS to OPFS.
- Logs go to the console, plus an optional in-memory ring buffer that the page can download.

### 6. New crate `crates/skate-web-server` (native binary; axum + tokio, versions pinned)

Static serving:
- Serve `index.html`, the loader JS, the wasm-bindgen glue and `skate_web_bg.wasm`, with content-hashed filenames
  and long-lived immutable caching for hashed files.
- Use `Content-Type: application/wasm` so streaming compilation works, and serve precompressed `.br` and `.gz`.
- Default headers: `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`, so
  the threads milestone works without a redeploy.
- Support HTTP `Range` requests.

`/source`:
- Redirects to the repository at the embedded revision, so each deployment meets the GPL source offer.

`/net` (WebSocket):
- **Lobby directory.** Implements `skate_net::directory` Browse, Host and Join, keyed by map and physics
  fingerprints, `PAGE_SIZE` = 5, and the namespace check in `is_game_lobby`.
- **Relay.** Forwards opaque datagrams between a lobby's host and its members, tagged with a stable per-connection
  `u64` peer id.
- **Bounds.** Enforce `MAX_PLAYERS`, the maximum frame size, per-connection token-bucket rate limits that start from
  `lobby.rs`'s link budgets, idle timeouts, and lobby cleanup when the host leaves.
- **Trust.** The relay never parses gameplay payloads beyond length and magic, and never trusts client-supplied
  peer ids.
- **Later transports.** Note in the docs that WebRTC DataChannels (unordered, `maxRetransmits: 0`) or WebTransport
  datagrams are the next step to avoid TCP head-of-line blocking. Keep the WebSocket path as the fallback.

Optional operator asset mount, `--assets-dir <installation>`:
- Off by default. When enabled, it requires `--access-token` and binds to `127.0.0.1` unless `--bind` is given
  explicitly. It serves only paths listed in that installation's `web-manifest.json`.
- It exists so someone can play their own install from another device on their own network. Print a startup warning
  that the operator is responsible for not exposing copyrighted data publicly.

Configuration, operations and tests:
- Configure through CLI flags with env-var equivalents. Provide `/healthz`, structured logs and graceful shutdown.
- Add a Dockerfile, and a `scripts/build-web.sh` plus a `scripts/Build-Web.ps1` that build the wasm, run
  wasm-bindgen and wasm-opt, precompress, and stage into `target/web/`.
- Test the server with integration tests: lobby lifecycle, relay fan-out, rate limits, oversize frames, header
  checks, Range requests and token enforcement.

### 7. Browser loader (`web/`: `index.html`, a small dependency-free ES module, CSS)

The page runs these steps in order:

1. **Capability check.**
   - WebGPU (`navigator.gpu.requestAdapter()`), OPFS and the Gamepad API.
   - A clear, specific message for each missing piece.
2. **Asset import, the first time only.**
   - The player picks their converted installation folder (`data/installations/<id>`) with `showDirectoryPicker()`,
     falling back to `<input webkitdirectory>`. Explain where to find that folder: run the Windows release once with
     your own disc and setup creates it.
   - Copy the files into OPFS with progress and resume support, and generate or verify the `web-manifest.json`
     hashes.
   - Store only the user's own files. Never upload them anywhere.
   - If the server advertises an operator mount, offer "Use this server's library" after the token prompt.
3. **Boot.**
   - Prefetch the `boot` group into the VFS, show a "Click to play" button (it unlocks audio), then call `start()`.
4. **While running.**
   - Map changes prefetch `map:<name>` before the existing transition runs.
   - Show storage usage and offer "Clear imported data".
   - A fullscreen button.
   - A link to `/source`.

## Milestones and acceptance criteria

**M0. Compiles for wasm; native unchanged.**
- Done when:
  - `cargo check -p skate-web --target wasm32-unknown-unknown --no-default-features` passes;
  - `cargo test -p skate-core` passes, and also passes under `--target wasm32-wasip1` with a wasmtime runner. This is
    the determinism proof: compare the physics test outputs bit for bit;
  - the native Windows build (`cargo build -p skate-game --bin skate3rust --release`) and its scoped tests are
    unchanged.
- Covers the lib/bin split, the Instant swap, the target-specific dependencies, and the feature-gated mods and
  networking.

**M1. VFS on native, no behaviour change.**
- Every runtime `std::fs` use in the game crates goes through `skate-vfs`.
- `SKATE_VFS_TRACE` produces the per-phase read lists.
- Done when:
  - with a real converted install, `skate3rust.exe --assets assets --test-world --check-assets` still prints
    `SKATE_ASSETS_READY`;
  - the `regression-check` skill's map checks match the baseline.

**M2. Boots in the browser.**
- The loader, the import into OPFS and the manifest generator all work.
- Done when, in Chrome or Edge with WebGPU and a real install, the test world renders, the skater responds to a
  standard gamepad, and there are no WebGPU validation errors in the console.
- Record frame time and wasm memory.

**M3. Maps and menus.**
- University and at least one city map (San Vanelona) load through the map transition. Switching maps evicts the
  previous map's data. Retail menus and the HUD draw. Settings persist across reloads.

**M4. Audio.**
- Native audio plays after the click-to-play gesture. Banks load lazily.
- No sustained underruns while skating on University, and memory stays bounded across three map changes.

**M5. Server and multiplayer.**
- `skate-web-server` serves the build with correct headers and passes its integration tests.
- Two browser tabs can host and join a lobby through `/net` and see each other skate.
- The operator mount works only with a token, and refuses paths outside the manifest.

**M6. Hardening (optional, each item separate).**
- wasm threads for texture decode and map loading.
- AudioWorklet output.
- BC textures.
- WebRTC/WebTransport datagrams.
- A route to Lua mods on the web.
- A keyboard mapping.

For CI, add `.github/workflows/web.yml` on `ubuntu-latest`. Leave `release.yml` unchanged. The job:
- builds the wasm and the server, and runs the server tests and the `wasm32-wasip1` determinism tests;
- runs a headless Chromium smoke test (Playwright; Chromium is preinstalled in many CI images, and the WebGPU flags
  must be set). The test loads the page with no assets and asserts that the import screen appears with no console
  errors;
- uploads `target/web/` as an artifact.

CI has no game data, so never make CI depend on it.

## How to verify each change

- Native:
  - `cargo build -p skate-game --bin skate3rust --release`;
  - scoped tests with `cargo test -p <crate> --release` (for `skate-data`, use `--lib --tests`);
  - the static-CRT release configuration from `tools/claude-skills/build-and-run/SKILL.md` whenever a native
    dependency moves.
- Web:
  - `cargo check` and `cargo build --target wasm32-unknown-unknown`;
  - `wasm-bindgen`;
  - load the page in a WebGPU browser and read the console;
  - the Playwright smoke test.
- Determinism: run the same recorded input (`skate_data::input_recording`, `replay.rs`) natively and on wasm, and
  diff the per-tick board and skeleton state. Any difference is a bug. Don't wave it away as float noise.
- Read `tools/claude-skills/*/SKILL.md` (`build-and-run`, `asset-setup`, `regression-check`, `vendor-patch`) before
  you touch those areas. If you change a vendored Bevy crate, follow `vendor-patch` and update `vendor/README.md`.

## Deliverables

- Code:
  - the crates `skate-vfs`, `skate-web` and `skate-web-server`;
  - the `skate-game` lib/bin split;
  - the `cfg` and feature changes;
  - `web/`;
  - the build scripts;
  - the web CI workflow.
- `docs/web-port/README.md`:
  - the architecture, the browser requirements, and how players import their own install;
  - how operators run the server, including the security notes for the asset mount;
  - a known-limitations table (mods, threads, memory ceiling, transport) and per-milestone verification results.
- A short "Play in the browser" section in the root `README.md` that repeats that assets are not included.
- Small, focused commits, one concern each, with messages that say what and why.

## When to stop and ask

Stop and ask the maintainer before you:
- change any file under `vendor/` beyond a compile fix;
- change parity-sensitive maths in `skate-core` or `skate-audio`;
- upgrade Bevy, wgpu, or any pinned dependency;
- replace a dependency with a different one (for example `zstd` → `ruzstd`) on native as well as on wasm.

Also stop if a milestone's acceptance criteria can't be met. Report what fails, with logs, and don't weaken the
criterion.

Report honestly at each milestone: what you verified, what you couldn't verify without game data or a GPU, and
what's left.
