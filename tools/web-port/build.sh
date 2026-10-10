#!/usr/bin/env sh
# Builds the browser package into target/web/dist: the loader page, the wasm-bindgen glue and the
# engine (docs/web-port/README.md). Upload that folder to any static host.
#
# The page links to the source of the exact commit it was built from, as GPL-3.0 requires for
# the served .wasm, so a tree with uncommitted changes refuses to package. SKATE_WEB_ALLOW_DIRTY=1
# overrides that for local testing only: never publish such a build.
# Needs the wasm32-unknown-unknown target and wasm-bindgen-cli 0.2.127 (the version in Cargo.lock).
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

cargo build --locked --profile web -p skate-game --bin skate3rust --target wasm32-unknown-unknown --no-default-features
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" target/wasm32-unknown-unknown/web/skate3rust.wasm
sed -e "s#__SKATE_SOURCE__#$source#g" \
    -e "s#__SKATE_REVISION_SHORT__#$(echo "$revision" | cut -c1-12)#g" \
    -e "s#__SKATE_REVISION__#$revision#g" \
    web/index.html > "$out/index.html"
ls -l "$out"
