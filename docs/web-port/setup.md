# Setting up from your ISO on macOS or Linux

The browser build plays a converted installation of your own copy of Skate 3. On Windows the game's
setup makes one from your Xbox 360 ISO. On macOS and Linux, `tools/web-port/convert.sh` runs the
same conversion from the same ISO, so you can then play it in the browser. Setup itself
downloads nothing (the tools come from Homebrew, Python's package index and crates.io) and uploads
nothing.

## macOS

```
brew install python@3.13 cmake vgmstream llvm node
tools/web-port/convert.sh
```

A Finder window asks for your Skate 3 ISO (it can open behind Terminal). You can also name it:
`tools/web-port/convert.sh ~/Downloads/Skate3.iso`. Setup converts it into `~/Skate 3 Rust
Engine`, printing what it does, and ends with the command that plays it:

```
tools/web-port/build.sh                  # the browser package, once (browser-build.md)
cargo run --release -p skate-web-server -- --installation "$HOME/Skate 3 Rust Engine"
```

Open the `http://localhost:8080/#mount=…` address the server prints, in Chrome or Edge
([server.md](server.md)). Or serve the package without `--installation` and choose the
`Skate 3 Rust Engine` folder on the page ([installation.md](installation.md)).

`llvm` and `node` are for `build.sh`, which finds Homebrew's clang itself (Apple's cannot build
zstd's C sources for wasm32) and compresses the package with Node: see
[`tools/web-port/README.md`](../../tools/web-port/README.md).

| Option | |
|---|---|
| `ISO` | Your Skate 3 Xbox 360 ISO, or `default.xex` in a copy you extracted yourself. Without it, a file dialog asks. |
| `--out FOLDER` | Where to set up (default `~/Skate 3 Rust Engine`). Keep it outside the repository. |
| `--without-audio` | Set up without vgmstream. The game then has no audio. |

`SKATE_SETUP_MAP_WORKERS`, `SKATE_SETUP_THREADS` and `SKATE_SETUP_PRIORITY` work as on Windows
(`tools/asset_pipeline/setup_budget.py`).

Allow about 12 GB of free space: the disc's files while converting (about the ISO's size; removed
afterwards) and the installation (1.5 to 3 GB). The engine build needs a few more GB in the
repository's `target` folder. The first run builds the engine natively: about a quarter of an hour
on the Linux machine this was tested on.

Running it again after an update updates the installation, converting only the asset groups whose
code changed, as the Windows game does when it starts. Use the same disc.

## Linux

The same command, with Python 3.11 to 3.13, CMake, Rust and `vgmstream-cli` on the `PATH`
(Homebrew on Linux, or a distribution package built with FFmpeg, which Xbox audio needs). Without
`zenity` or `kdialog` there is no file dialog: name the ISO.

## What it does

`convert.sh` prepares what the Windows package ships ready-made, then runs `convert.py`:

1. A Python environment (`target/setup/venv`) with setup's pinned packages from
   `tools/requirements-setup.txt` (numpy, Pillow; not PyInstaller, which only packages the
   Windows setup).
2. A native engine build (`target/release/skate3rust`). Setup reads two tables from `default.xex`
   with it and checks every converted map with it (`--validate-maps`), as on Windows.
3. The native RefPack decoder, about ten times faster than setup's Python one: the same
   `tools/asset_pipeline/refpack_native.rs`, compiled for this system. `fast_refpack.py` loads
   `target/native/refpack.dll` by that name, so it keeps the name.
4. `convert.py`: checks the disc's file names, extracts the disc beside the installation, runs
   the Windows setup's conversion (`tools/asset_pipeline/customiser_setup.install`) and removes the
   extracted files. On macOS it runs under `caffeinate`, so the Mac stays awake until it ends.

The conversion is the Windows setup's own code. Only its Windows-only parts are replaced:

| Windows setup | macOS and Linux |
|---|---|
| Extracts the ISO with extract-xiso, downloaded | `tools/web-port/xiso.py`, a built-in reader. It extracts exactly what extract-xiso does: both read each other's images to identical files, at all four disc layouts (rebuilt, XGD1, XGD2, XGD3) |
| vgmstream r2117, downloaded | Homebrew's vgmstream, the same release. Its FFmpeg may be a different version, so decoded audio can differ in the last bits |
| `skate3rust.exe` from the package | The native build from step 2 |
| `refpack.dll` from the package | The same source, built for this system (step 3) |
| Below-normal process priority | `nice` 10 (`SKATE_SETUP_PRIORITY`: `normal` 0, `idle` 19) |
| Free memory from Windows | `vm_stat` (macOS) or `/proc/meminfo` (Linux), to bound the map workers |

`tools/asset_pipeline/install.py` picks the system's tools and reads the memory outside Windows.
Those functions are not part of the asset pipeline fingerprints, so no Windows installation is
converted again because of them.

**File names.** Windows finds a file whatever the case of its name; macOS and Linux matching
is exact. Setup's code names the disc's files as the disc does. To make sure, `convert.py` checks
the names setup reads before extracting anything, and stops with the name it found instead of
failing partway through the conversion.

## When it stops

`convert.py` prints why. A failure during the conversion leaves its details in
`data/setup-error.log` and the new installation's `setup.log` and `*-conversion.log` files. Run it
again once fixed: it starts the conversion over, and an existing installation stays until the new
one succeeds, as with the Windows setup.

This was built and tested on Linux with synthetic discs, since the repository has no game data:
the extraction (against extract-xiso), the name checks, the native RefPack decoder (against the
Python one) and the start of the conversion. The conversion itself passes its own tests on Linux.
A real ISO has not yet been converted on a Mac, so the first run there is the real test. If it
stops, the files above say where.

## Your data

The folder holds data converted from your disc. Keep it to yourself: never upload it, publish it
or commit it. The default location keeps it out of the repository.
