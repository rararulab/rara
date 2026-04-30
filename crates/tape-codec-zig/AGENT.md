# tape-codec-zig — Agent Guidelines

## Purpose

Proof-of-concept JSONL codec for `TapEntry` re-implemented in Zig 0.16
behind a default-off cargo feature on `rara-kernel`. The crate's job is
to answer three questions with data — see
`specs/issue-2007-tape-codec-zig-poc.spec.md` Intent and the colocated
`POC_RESULTS.md` for the answers.

This is not production code. If you are reading this AGENT.md to extend
the crate, first re-read `POC_RESULTS.md` — the PoC may have answered
"no, do not adopt Zig", in which case the right action is to delete the
crate, not extend it.

## Architecture

- Zig source lives in the top-level `zig/` project — see `zig/AGENT.md`.
  The codec module is `zig/src/tape_codec.zig`, declared as the
  `tape_codec_zig_static` static library in `zig/build.zig`. Two `export`
  functions (`tape_codec_zig_decode`, `tape_codec_zig_encode`) with
  caller-allocated byte buffers; no Zig allocator state crosses the FFI.
  Each call uses a fresh `ArenaAllocator` that is freed before the
  function returns.
- `src/lib.rs` — Rust wrapper. `decode(&[u8]) -> Result<Vec<u8>>` and
  `encode(&[u8])`. Behind `feature = "zig-codec"`; absent that feature,
  every call returns `Error::FeatureDisabled`.
- `build.rs` — invokes `zig build` against `../../zig` when
  `CARGO_FEATURE_ZIG_CODEC` is set, then emits cargo link directives.
  All Zig codegen flags (PIC, optimize mode) live in `zig/build.zig`,
  not here.
- `tests/differential.rs` — N=1000 random `TapEntry` byte-equality test
  vs `serde_json::to_vec`. Uses a local mirror of the kernel's `TapEntry`
  shape to avoid a circular dev-dep on `rara-kernel`.
- `scripts/measure_build.sh` — runs `cargo build --timings` twice (once
  per feature state) and writes results into the spec's `timings/` dir.

## Critical Invariants

- **Output parity with `serde_json` rests on two settings, both
  documented in `zig/src/tape_codec.zig`:**
  1. Zig's `std.json.ObjectMap` preserves insertion order. The codec
     therefore relies on the *input bytes* already having the right
     order (serde's struct-derived shapes emit declaration order;
     `Value::Object` BTreeMap emits lexicographic). Do NOT add a sort
     step "to be safe" — sorting breaks struct-shape parity.
  2. `parse_numbers = false` keeps numeric values as original byte
     slices, so `722.0` does not collapse to `722`. Removing this
     option will silently diverge on integer-valued floats.
- **No Zig allocator across the FFI.** The Zig side parses + emits into
  an arena; the Rust side hands in a buffer and reads back the byte
  count. If you change the FFI contract, also update the i32 return-code
  table in both `lib.rs` and `codec.zig` — they must agree.
- **Feature-flag asymmetry.** `zig-codec` is a default feature of *this*
  crate (so its own tests link the static lib), but a *non-default*
  feature on `rara-kernel`. Default kernel builds and CI must remain
  Zig-toolchain-free.

## What NOT To Do

- Do NOT add new FFI functions without updating the i32 return-code
  table in both `zig/src/tape_codec.zig` and `lib.rs` — drift here
  produces `UnknownCode` at runtime instead of compile-time errors.
- Do NOT duplicate Zig toolchain config (`pic`, optimize mode,
  `.zig-version`) in this crate — those live in `zig/`. Per-crate
  duplication is exactly the trap PR #2008's first revision fell into.
- Do NOT replace the `ArenaAllocator` with `DebugAllocator` in
  `zig/src/tape_codec.zig` — `DebugAllocator` checks every free, and
  the JSON parser produces nested allocations whose ownership is hard
  to track without the arena. The original prototype hit double-free
  panics until the arena was introduced.
- Do NOT add a sort step to `roundTrip` / `tape_codec_zig_*` —
  `std.json.ObjectMap` preserves insertion order, and serde emits
  struct fields in declaration order (not lexicographic). Sorting
  matches `Value::Object` (BTreeMap) but breaks `TapEntry`
  struct-shape parity. The first prototype hit this; the differential
  test caught it. Rely on insertion-order preservation and do nothing.
- Do NOT remove the local `TapEntry` mirror in
  `tests/differential.rs` and depend on `rara-kernel` instead — it
  creates a cycle the moment the kernel's `zig-codec` feature is on.
- Do NOT promote this crate to a kernel default without a follow-up
  spec. The whole point of the feature flag is reversibility.

## Dependencies

- Upstream: none in the workspace (intentionally — keeps the PoC
  isolated). The `build.rs` shells out to the system `zig` binary.
- Downstream: `rara-kernel` (optional, behind `feature = "zig-codec"`).
- External: Zig 0.16.x toolchain, install via `brew install zig` (macOS)
  or `mise` / official tarball (Linux). See
  `docs/guides/zig-toolchain.md`.
