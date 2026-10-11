#!/usr/bin/env sh
# Builds the browser package into target/web/dist: the loader page and its installation reader,
# the wasm-bindgen glue and the engine (docs/web-port/README.md). Upload that folder to any static host.
#
# The page links to the source of the exact commit it was built from, as GPL-3.0 requires for
# the served .wasm, so a tree with uncommitted changes refuses to package. SKATE_WEB_ALLOW_DIRTY=1
# overrides that for local testing only: never publish such a build.
# Needs the wasm32-unknown-unknown target and wasm-bindgen-cli 0.2.127 (the version in Cargo.lock).
#
# Function names (the module's "name" section, about 45 % of it) only make console stack traces
# readable, so they are left out; SKATE_WEB_KEEP_NAMES=1 keeps them for debugging. Each served file
# also gets gzip and brotli copies (compress.mjs, Node) that skate-web-server sends to browsers that
# accept them; static hosts compress by themselves. Brotli runs at quality 11 (about three minutes
# for the engine), or 9 for a SKATE_WEB_ALLOW_DIRTY test build.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
out="$root/target/web/dist"

revision=$(git rev-parse HEAD)
if [ -n "$(git status --porcelain)" ]; then
    if [ "${SKATE_WEB_ALLOW_DIRTY:-0}" != 1 ]; then
        echo "Uncommitted changes: the source link would not match this build. Commit first." >&2
        exit 1
    fi
    echo "warning: packaging a dirty tree (SKATE_WEB_ALLOW_DIRTY=1); do not publish it" >&2
fi
# The repository the page points to; defaults to this checkout's origin.
source=${SKATE_SOURCE_URL:-$(git remote get-url origin | sed -e 's#^git@github\.com:#https://github.com/#' -e 's#\.git$##')}

# zstd's C sources need a clang with a wasm32 target. Apple's has none; Homebrew's LLVM does.
if [ "$(uname -s)" = Darwin ] && [ -z "${CC_wasm32_unknown_unknown:-}" ] && command -v brew >/dev/null 2>&1; then
    llvm=$(brew --prefix llvm 2>/dev/null || true)
    if [ -x "$llvm/bin/clang" ]; then
        export CC_wasm32_unknown_unknown="$llvm/bin/clang" AR_wasm32_unknown_unknown="$llvm/bin/llvm-ar"
    fi
fi
cargo build --locked --profile web -p skate-game --bin skate3rust --target wasm32-unknown-unknown --no-default-features
rm -rf "$out"
mkdir -p "$out"
names=--remove-name-section
if [ "${SKATE_WEB_KEEP_NAMES:-0}" = 1 ]; then names=; fi
wasm-bindgen --target web --no-typescript $names --remove-producers-section --out-dir "$out" target/wasm32-unknown-unknown/web/skate3rust.wasm
sed -e "s#__SKATE_SOURCE__#$source#g" \
    -e "s#__SKATE_REVISION_SHORT__#$(echo "$revision" | cut -c1-12)#g" \
    -e "s#__SKATE_REVISION__#$revision#g" \
    web/index.html > "$out/index.html"
cp web/installation.js "$out/"
quality=11
if [ "${SKATE_WEB_ALLOW_DIRTY:-0}" = 1 ]; then quality=9; fi
node tools/web-port/compress.mjs "$quality" "$out"/index.html "$out"/installation.js "$out"/skate3rust.js "$out"/skate3rust_bg.wasm
ls -l "$out"
