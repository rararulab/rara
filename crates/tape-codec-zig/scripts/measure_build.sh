#!/usr/bin/env bash
# Measure the compile-time impact of the zig-codec feature on rara-kernel.
#
# Runs `cargo build --timings -p rara-kernel` twice — once without the
# feature, once with — and copies the HTML reports into the spec's
# timings/ directory. Does not gate on a delta; the PoC's job is to
# report data, not to enforce a threshold.
#
# Usage:
#   crates/tape-codec-zig/scripts/measure_build.sh
#
# Output:
#   specs/issue-2007-tape-codec-zig-poc/timings/cargo-timing-off.html
#   specs/issue-2007-tape-codec-zig-poc/timings/cargo-timing-on.html

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../../.." && pwd)"
OUT_DIR="$ROOT_DIR/specs/issue-2007-tape-codec-zig-poc/timings"
mkdir -p "$OUT_DIR"

cd "$ROOT_DIR"

echo "==> clean build, feature OFF"
cargo clean -p rara-kernel >/dev/null 2>&1 || true
cargo build --timings -p rara-kernel
latest_off="$(ls -t target/cargo-timings/cargo-timing-*.html | head -1)"
cp "$latest_off" "$OUT_DIR/cargo-timing-off.html"
echo "    saved -> $OUT_DIR/cargo-timing-off.html"

echo "==> clean build, feature ON"
cargo clean -p rara-kernel >/dev/null 2>&1 || true
cargo build --timings -p rara-kernel --features zig-codec
latest_on="$(ls -t target/cargo-timings/cargo-timing-*.html | head -1)"
cp "$latest_on" "$OUT_DIR/cargo-timing-on.html"
echo "    saved -> $OUT_DIR/cargo-timing-on.html"

echo
echo "Reports written. Open them in a browser to compare. Update"
echo "specs/issue-2007-tape-codec-zig-poc/POC_RESULTS.md with the"
echo "rara-kernel total times from the 'Unit time' column at the bottom"
echo "of each report."
