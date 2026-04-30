spec: task
name: "issue-2007-tape-codec-zig-poc"
inherits: project
tags: []
---

## Intent

Phase 0 of evaluating Zig as a second implementation language for rara: a
proof-of-concept that re-implements the JSONL line codec for `TapEntry` in
Zig 0.16, behind a default-off cargo feature flag, so we can answer three
questions with data instead of speculation.

The questions:

1. **Build integration.** Can a `build.rs` invoke `zig build-lib`,
   produce a static `.a`, and link it into a Rust crate cleanly on both
   macOS (developer laptops) and Linux (CI ARC runner)? What is the
   developer-onboarding cost?
2. **Output parity.** Can Zig 0.16 `std.json` produce byte-identical
   output to `serde_json::to_vec` for our actual `TapEntry` shape, which
   includes a `serde_json::Value` payload (arbitrary nested JSON), a
   `Timestamp`, and conditional `skip_serializing_if = "Option::is_none"`
   fields? And in the reverse direction, does parsing a serde-emitted
   line yield a structurally equal `TapEntry`?
3. **Compile-time signal.** Does compiling the codec in Zig instead of
   Rust measurably reduce `cargo build --timings` for `rara-kernel`?
   Even a small effect on a small codec is informative for whether a
   future, larger Zig migration is worth pursuing.

The codec entry points to swap, located in `crates/kernel/src/memory/store.rs`:

- Decode at line 259: `serde_json::from_slice::<TapEntry>(trimmed)`
  inside the per-line loop of `read_chunk`. Single newline-trimmed JSON
  object to `TapEntry`.
- Encode at line 337: `serde_json::to_vec(&entry)` inside the
  `append_batch` loop. Single `TapEntry` to JSON object bytes; the
  caller appends the `\n`.

Everything around those two calls (the `TapeFile` cache, `IoWorker`,
fork/merge/discard logic, `mmap`, position tracking, fsync) stays in
Rust. Zig is purely an in-memory bytes-to-struct codec.

Reproducer for the question "what concrete bad outcome do we avoid by
doing this PoC?" Without this PoC, any future "rewrite hot path X in
Zig" decision is guesswork. Concrete failure mode if we skip straight
to a larger migration: 1) a bigger PR ships re-implementing memory in
Zig; 2) it builds on macOS but not on the Linux ARC runner because
nobody verified `zig build` in that container; 3) `std.json` quietly
emits keys in declaration order while serde emits them in BTreeMap
(lexicographic) order, so old tapes round-trip-decode but new tapes
written through the Zig path are byte-different (silent split-brain on
disk); 4) we revert under pressure with no clean rollback because the
code paths are no longer feature-gated. This PoC is the cheap fence
that prevents that sequence by isolating the question to a 200-LOC
codec with a feature flag and a differential test.

Goal alignment. This work does not advance any "What working rara
looks like" signal in `goal.md` directly; it is dev-velocity
infrastructure. It does not cross any "What rara is NOT" line.
Tension with `goal.md` to call out explicitly: the bet in `goal.md` is
"Rust plus boring technology plus kernel discipline". Adding Zig is
*less* boring, not more. This PoC's job is to produce evidence that
quantifies the trade. If the data is unconvincing, the answer is to
not introduce Zig, and the PoC artifact is then a tombstone, not a
foundation.

Prior art surveyed. No prior Zig integration discussion in this repo
(verified via `gh issue list --search zig`, `gh pr list --search zig`,
`git log --all --grep zig`). The only existing Zig touch-points are:
a) the `zlob` crate dependency, which itself uses Zig 0.16 (issue 1665
and PR 1666), confirming the Zig 0.16 toolchain is already required by
the workspace transitively; b) `crates/cmd/build.rs` exists as
precedent for non-trivial build-script work in this repo;
c) `crates/cmd/src/setup/boxlite.rs` and PR 1881 show how this repo
already handles native-binary builds gated on platform.

External OSS prior art (4 projects; recommendation in Decisions):

- TigerBeetle (`tigerbeetle/tigerbeetle`). Rust client links against a
  Zig static lib. Approach: Rust `build.rs` shells out to `zig build
  clients:c`, then moves the `.a` artifacts into the Rust source tree
  and emits `cargo:rustc-link-lib=static=...`. Production-grade, used
  by their official Rust client. Closest match for our shape.
- `zigc` crate (lib.rs/crates/zigc). Generic helper:
  `zigc::Build::new().file("src/main.zig").finish()` in `build.rs`.
  Compiles to `.so` (dynamic), not static. Adds a transitive dep on a
  small (<50 stars) single-maintainer crate. Not recommended for a PoC
  where minimizing risk surface matters.
- `cargo-zigbuild` (`rust-cross/cargo-zigbuild`). Uses Zig as the
  *linker* for cross-compilation. Does not compile Zig source. Not
  applicable to this PoC.
- `jeremyBanks/zig_with_cargo`. Small example repo demonstrating
  manual `build.rs` invoking `zig` directly. Same shape as
  TigerBeetle, simpler. Useful as a reference snippet.

Recommended approach: TigerBeetle's pattern, minimized. A hand-written
`build.rs` that runs `Command::new("zig").args(["build-lib", "-O",
"ReleaseSafe", "src/codec.zig", "-femit-bin=...", "-target",
"<host>"])`, then prints `cargo:rustc-link-lib=static=...` and
`cargo:rustc-link-search=native=...`. No `zigc` dep. About 30 LOC of
build script. If a future migration grows this past 100 LOC, revisit
and consider TigerBeetle's heavier `zig build` orchestration.

## Decisions

- Lane 1. Acceptance binds to a real `#[test]` differential test
  (round-trip parity on N=1000 random `TapEntry` values).
- Zig version 0.16 (confirmed current; matches the version required
  transitively by `zlob` per issue 1665). Pin via a top-level
  `.zig-version` file in `crates/tape-codec-zig/` and document the
  install path in the new crate's `AGENT.md`.
- Build glue: hand-written `build.rs` invoking `zig build-lib`
  directly. No dep on `zigc`. Modeled on TigerBeetle's pattern,
  simplified. The `build.rs` MUST emit `cargo:rerun-if-changed=...`
  for every `.zig` file under the crate's `src/` so cargo invalidates
  correctly.
- Feature flag: a new cargo feature `zig-codec` on `rara-kernel`,
  default OFF. The Zig path is opt-in for this PoC. Default builds,
  default CI, and all existing tests continue to use the serde path
  unchanged.
- FFI surface: two `extern "C"` functions, both byte-oriented and
  caller-allocated, no Zig allocator crossing the boundary. Names:
  `tape_codec_zig_encode(in_ptr, in_len, out_ptr, out_capacity,
  out_written) -> i32` and `tape_codec_zig_decode(in_ptr, in_len,
  out_ptr, out_capacity, out_written) -> i32`. Rationale:
  `TapEntry`'s `payload: serde_json::Value` is impractical to mirror
  as a Zig struct, so the harness passes serde-emitted JSON bytes
  across the boundary, and the Zig side parses then re-emits. The
  differential test then becomes "given a `TapEntry`, serde encodes
  to bytes A, Zig round-trips bytes A to bytes B, assert A == B" —
  the byte-equality criterion the user asked for.
- JSON key ordering: workspace `serde_json` is at version 1.0.143
  with no `preserve_order` feature (verified `Cargo.toml` line 203).
  Default `serde_json` emits map keys in lexicographic order for
  `Value::Object` (BTreeMap) and in declaration order for derived
  structs. The Zig encoder MUST emit keys in the same order. The
  differential test will catch any drift.
- Random `TapEntry` generation: use `proptest` (already in the
  workspace) with a strategy that exercises every `TapEntryKind`
  variant, both `Some` and `None` for `metadata`, and nested
  `payload: Value` shapes (object, array, primitives, escaped
  strings, unicode, control chars, embedded `/`). N=1000 cases,
  fixed seed for reproducibility.
- Toolchain install story: document Zig 0.16 install via Homebrew
  (`brew install zig`) on macOS and via the official tarball or
  `mise` pin on Linux. Add a `just zig-toolchain-check` step that
  runs `zig version` and asserts `0.16.x`. Hook it into `init.sh`
  only when the `zig-codec` feature is active, so default
  contributors are not forced to install Zig for this PoC.
- Compile-time measurement: ship a script
  `crates/tape-codec-zig/scripts/measure_build.sh` that runs
  `cargo build --timings -p rara-kernel` twice (once without and
  once with `--features zig-codec`) and writes the delta into
  `crates/tape-codec-zig/POC_RESULTS.md`. Do NOT gate the PoC on a
  specific delta; just report it.
- Out of scope explicitly enforced: the PoC does NOT touch
  `FileSessionIndex`, does NOT touch CI workflows except to add a
  smoke job that compiles `--features zig-codec` on macOS and
  Linux (no merge gating), and does NOT replace the default codec
  path.

## Boundaries

### Allowed Changes
- crates/tape-codec-zig/**
- **/crates/tape-codec-zig/**
- crates/kernel/Cargo.toml
- **/crates/kernel/Cargo.toml
- crates/kernel/src/memory/store.rs
- **/crates/kernel/src/memory/store.rs
- crates/kernel/src/memory/codec.rs
- **/crates/kernel/src/memory/codec.rs
- crates/kernel/src/memory/error.rs
- **/crates/kernel/src/memory/error.rs
- crates/kernel/tests/tape_default_codec.rs
- **/crates/kernel/tests/tape_default_codec.rs
- Cargo.toml
- **/Cargo.toml
- Cargo.lock
- **/Cargo.lock
- justfile
- **/justfile
- init.sh
- **/init.sh
- .github/workflows/zig-codec-smoke.yml
- **/.github/workflows/zig-codec-smoke.yml
- docs/guides/zig-toolchain.md
- **/docs/guides/zig-toolchain.md
- specs/issue-2007-tape-codec-zig-poc.spec.md
- **/specs/issue-2007-tape-codec-zig-poc.spec.md
- specs/issue-2007-tape-codec-zig-poc/**
- **/specs/issue-2007-tape-codec-zig-poc/**

### Forbidden
- crates/kernel/src/memory/mod.rs
- crates/kernel/src/memory/file_index.rs
- crates/kernel/src/memory/io_worker.rs
- crates/kernel/src/agent/**
- crates/kernel/src/cascade.rs
- crates/sessions/**
- crates/app/**
- crates/channels/**
- crates/acp/**
- crates/sandbox/**
- web/**
- crates/rara-model/migrations/**
- .github/workflows/ci.yml
- .github/workflows/e2e.yml

## Completion Criteria

Scenario: Differential round-trip parity on 1000 random TapEntry values
  Test:
    Package: tape-codec-zig
    Filter: differential_round_trip_parity
  Given a proptest strategy generating 1000 diverse TapEntry values covering every TapEntryKind, both Some and None metadata, and payloads with nested objects, arrays, escaped strings, unicode, and control characters
  When each entry is serialized via serde_json to bytes A, then those bytes are passed through the Zig codec via decode then encode producing bytes B
  Then bytes A equals bytes B for every one of the 1000 cases

Scenario: Feature flag default-off preserves existing behavior
  Test:
    Package: rara-kernel
    Filter: tape_store_uses_serde_codec_by_default
  Given rara-kernel is built with default features only
  When the existing tape store integration tests run
  Then no Zig symbol is linked into the binary and all existing tape tests pass unchanged

Scenario: Build integration succeeds on macOS and Linux
  Test:
    Package: tape-codec-zig
    Filter: build_artifact_is_present
  Given a developer machine with Zig 0.16 installed
  When cargo build with feature zig-codec runs on either macOS or Linux
  Then the build completes successfully and the resulting Rust binary contains the encode and decode symbols linked from the static archive produced by zig build-lib

## Out of Scope

- Replacing the default codec path on main. Feature flag stays opt-in for the duration of the PoC.
- Migrating any other module (FileSessionIndex, IoWorker, agent loop, cascade builder) to Zig.
- File I/O in Zig. The Zig side handles in-memory bytes only.
- CI workflow changes that gate merges on the Zig build passing. The new smoke workflow is observability-only.
- Replacing serde_json Value in TapEntry with a typed payload. Out of scope and explicitly forbidden by the boundaries above.
- A production decision to adopt Zig more broadly. That decision is downstream of the PoC results doc.
