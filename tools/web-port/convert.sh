#!/usr/bin/env sh
# Sets up Skate 3 from your own Xbox 360 ISO on macOS or Linux, for the browser build
# (docs/web-port/setup.md). Windows uses the game's own setup instead.
#
#   tools/web-port/convert.sh [ISO] [--out FOLDER] [--without-audio]
#
# Prepares what the Windows package ships ready-made, then runs convert.py, which asks for the
# ISO when none is given:
# - a Python environment with setup's pinned packages (tools/requirements-setup.txt), in
#   target/setup/venv;
# - a native engine build (target/release/skate3rust): setup reads tables from default.xex and
#   checks every converted map with it;
# - the native RefPack decoder, about ten times faster than setup's Python one. fast_refpack.py
#   loads target/native/refpack.dll by that name; here it holds this system's shared library.
# Needs Python 3.11 to 3.13, CMake (for SDL), Rust, and vgmstream for the game's audio
# (macOS: brew install python@3.13 cmake vgmstream).
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

python=
for candidate in python3.13 python3.12 python3.11 python3; do
    if command -v "$candidate" >/dev/null 2>&1 &&
        "$candidate" -c 'import sys; sys.exit(not (3, 11) <= sys.version_info[:2] <= (3, 13))' 2>/dev/null; then
        python=$candidate
        break
    fi
done
if [ -z "$python" ]; then
    echo "Setup needs Python 3.11 to 3.13 (macOS: brew install python@3.13)." >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "Setup builds the engine with Rust: install it from https://rustup.rs" >&2
    exit 1
fi
if ! command -v cmake >/dev/null 2>&1; then
    echo "The engine build needs CMake (macOS: brew install cmake)." >&2
    exit 1
fi

# The setup's packages, as pinned for the Windows setup (PyInstaller only packages it there).
venv=target/setup/venv
requirements=$(grep -v -i '^pyinstaller' tools/requirements-setup.txt)
stamp="$("$python" --version) $requirements"
if [ ! -x "$venv/bin/python" ] || [ "$(cat "$venv/.stamp" 2>/dev/null)" != "$stamp" ]; then
    echo "Preparing Python for setup ($("$python" --version))"
    rm -rf "$venv"
    "$python" -m venv "$venv"
    # shellcheck disable=SC2086 # one argument per pinned package
    "$venv/bin/python" -m pip install --quiet --disable-pip-version-check --only-binary=:all: $requirements
    printf '%s' "$stamp" > "$venv/.stamp"
fi

echo "Building the engine for setup's checks (the first build takes a while)"
# Static Bevy, as release packages build it (the default feature links it dynamically, for
# development, and the binary would not start outside cargo).
cargo build --locked --release -p skate-game --bin skate3rust --no-default-features
refpack=target/native/refpack.dll
if [ ! -f "$refpack" ] || [ tools/asset_pipeline/refpack_native.rs -nt "$refpack" ]; then
    mkdir -p target/native
    rustc --edition 2024 --crate-type cdylib -C opt-level=3 -C panic=abort tools/asset_pipeline/refpack_native.rs -o "$refpack"
fi

# A Mac left alone would sleep partway through the conversion: caffeinate keeps it awake until
# setup ends.
awake=
if [ "$(uname -s)" = Darwin ]; then awake="caffeinate -i"; fi
# shellcheck disable=SC2086 # empty, or the command and its option
exec $awake "$venv/bin/python" tools/web-port/convert.py --game-exe target/release/skate3rust "$@"
