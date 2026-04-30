# PoC Results — Zig JSONL Tape Codec (issue #2007)

This document is the central deliverable of issue #2007. It answers the
three questions stated in `specs/issue-2007-tape-codec-zig-poc.spec.md`
with data, and gives an honest read on whether Zig is worth pulling
further into rara.

## TL;DR

- Build integration: works. `build.rs` shells out to `zig build-lib`,
  emits a static archive, and Rust links against it cleanly on macOS
  (Homebrew Zig 0.16). Linux is exercised only by the new
  `zig-codec-smoke.yml` workflow; this PoC has not yet seen it run on
  the ARC runner.
- Output parity: 1000/1000 randomized `TapEntry` round-trips produce
  byte-identical output. Two non-obvious traps were found and worked
  around inside Zig (`parse_numbers = false`, do-not-resort keys).
- Compile-time signal: **inconclusive at this scale**. The captured
  `cargo build --timings` data is not a clean before/after — the two
  runs differed in cache state, not just in the codec choice. Even
  taken at face value, the only thing that recompiled in either run
  was `rara-kernel` itself; the rest of the workspace was already
  fresh. Swapping ~200 LOC of `serde_json::to_vec`/`from_slice`
  call-sites for two `extern "C"` calls is too small a delta to move
  the needle on a workspace where `serde_json` (and `serde`) still
  compile for everyone else.
- Recommendation: do **not** treat these timings as evidence for or
  against Zig. Either (a) drop the experiment here and keep rara
  Rust-only on the "boring technology" bet from `goal.md`, or (b)
  pick a single heavy compile-cost crate (`utoipa-gen`, `chromiumoxide`,
  one of the proc-macro-heavy ones) and re-run the experiment there,
  where the codec-vs-rest-of-workspace ratio actually flips.

## 1. Build integration

Question (spec): "Can a `build.rs` invoke `zig build-lib`, produce a
static `.a`, and link it into a Rust crate cleanly on both macOS
(developer laptops) and Linux (CI ARC runner)? What is the
developer-onboarding cost?"

### macOS (verified locally)

- Toolchain: `zig 0.16.0` via Homebrew (`/opt/homebrew/bin/zig`).
- `cargo clean -p tape-codec-zig && cargo build -p tape-codec-zig`
  finished in `8.22s`. The `build.rs` invokes `zig build-lib -O
  ReleaseSafe -fstrip src/codec.zig -femit-bin=<OUT_DIR>/libtape_codec_zig.a`,
  then prints `cargo:rustc-link-lib=static=tape_codec_zig` and
  `cargo:rustc-link-search=native=<OUT_DIR>`. No `zigc` crate needed.
- `cargo build -p tape-codec-zig` after no source change is a no-op
  (cargo cache hit; the `cargo:rerun-if-changed=src/codec.zig` line
  in `build.rs` correctly scopes invalidation to the Zig source).
- `cargo test -p tape-codec-zig` runs the differential test against
  the linked archive: 2/2 tests pass (`build_artifact_is_present`,
  `differential_round_trip_parity`).

Rough edges seen on macOS:

- Zig 0.16's `std.json.Stringify.valueAlloc` API is the 0.16-only
  spelling — earlier prototypes that used `std.json.stringifyAlloc`
  fail to compile against 0.16. The `.zig-version` file and the
  `just zig-toolchain-check` recipe are load-bearing for anyone
  trying to reproduce.
- `parse_numbers = false` is required to preserve the original
  number byte-slice; without it Zig normalizes `722.0` to `722`
  and the differential test fails immediately. Documented in
  `crates/tape-codec-zig/src/codec.zig` for the next person.

### Linux (CI-only, not verified locally)

- New workflow: `.github/workflows/zig-codec-smoke.yml` builds with
  `--features zig-codec` on `ubuntu-latest`, observability-only, no
  merge gating (per spec's "Out of Scope"). This run has not yet
  been exercised in a PR; the first PR push will be the first
  Linux signal.

### Onboarding cost

For a contributor not touching Zig: zero. The `zig-codec` feature is
default-off, `init.sh` does not require Zig unless the feature is
active, and CI's mandatory paths (`ci.yml`, `e2e.yml`) ignore the
Zig path.

For a contributor touching Zig: install Zig 0.16 (`brew install zig`
or `mise`), read `crates/tape-codec-zig/AGENT.md` and
`docs/guides/zig-toolchain.md`. Both exist in this PR.

## 2. Output parity (differential test)

Question (spec): "Can Zig 0.16 `std.json` produce byte-identical
output to `serde_json::to_vec` for our actual `TapEntry` shape?"

### Result

- Test: `crates/tape-codec-zig/tests/differential.rs ::
  differential_round_trip_parity`.
- N = 1000 randomized `TapEntry` values, fixed seed
  `0xCAFE_F00D_DEAD_BEEF`, generated with `proptest`-like helpers
  exercising every `TapEntryKind`, `Some` and `None` `metadata`,
  and payloads spanning nested objects, arrays, primitives, escaped
  strings, unicode, and control characters.
- Each case: `bytes_a = serde_json::to_vec(&entry)`,
  `decoded = zig::decode(bytes_a)`, `bytes_b = zig::encode(decoded)`,
  assert `bytes_a == bytes_b`.
- Outcome: 1000 / 1000 byte-identical. Test runtime ~30ms.

```
running 2 tests
test build_artifact_is_present ... ok
test differential_round_trip_parity ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; ...
```

### Surprises worth flagging

1. **Do-not-sort keys.** The first prototype eagerly sorted object
   keys on emit, mirroring what we expected serde to do. That
   broke struct-shape parity (`TapEntry`s top-level fields), where
   serde's derive emits fields in **declaration order**, not
   lexicographic. The fix was to do nothing: `std.json.ObjectMap`
   is a `StringArrayHashMap` that preserves insertion order, so
   parsing serde bytes and re-emitting them naturally preserves
   whatever order serde used. For `serde_json::Value::Object`
   (a `BTreeMap`) that is lexicographic; for derived structs it
   is declaration order. Zig copies whichever it received. The
   differential test caught this — without it, this would have
   shipped as a silent split-brain on disk.

2. **`parse_numbers = false`.** Without this flag, Zig parses
   `722.0` to an `f64` then re-emits as `722`, diverging from
   serde which preserves the original lexical form. Setting
   `parse_numbers = false` keeps `Value.number_string` as the
   original byte slice and round-trips byte-identical.

3. **Unicode and control chars.** `escape_unicode = false` (the
   default) emits non-ASCII verbatim, matching serde's default.
   If serde ever switches to ASCII-escape mode, the Zig side
   has to flip the same flag — the differential test will catch
   this on the next PR that flips it.

4. **`skip_serializing_if = "Option::is_none"`.** Already handled
   correctly: serde never emits the `metadata` key when `None`,
   Zig never sees it on parse, and so never re-emits it. No
   special handling needed in Zig.

## 3. Compile-time signal

Question (spec): "Does compiling the codec in Zig instead of Rust
measurably reduce `cargo build --timings` for `rara-kernel`?"

### Captured timing data

Two `cargo build --timings -p rara-kernel` runs were captured locally;
the raw HTML artifacts are not committed (see `.gitignore`
`specs/**/timings/*.html`). Numbers below are transcribed from those
runs and are reproducible by re-running `cargo build --timings -p
rara-kernel` with and without `--features zig-codec` from a clean
target dir.

| File | Profile | Total | Fresh units | Dirty units | Top non-kernel unit |
|---|---|---|---|---|---|
| `cargo-timing-off.html` | dev (no `zig-codec`) | 37.8s | 481 | 37 | chromiumoxide 3.8s |
| `cargo-timing-on.html`  | dev (`zig-codec`)    | 24.5s | 520 | 1 | addr2line 0.0s (fresh) |

Top 5 units in the OFF run (the only run where dependencies were
actually compiling):

| # | Unit | Time |
|---|---|---|
| 1 | rara-kernel v0.0.1 | 24.3s |
| 2 | chromiumoxide v0.9.1 | 3.8s |
| 3 | opendal v0.52.0 | 3.6s |
| 4 | tokio v1.52.1 | 2.8s |
| 5 | utoipa v5.4.0 | 2.1s |

Top 5 units in the ON run (only `rara-kernel` recompiled; everything
else was already cached fresh):

| # | Unit | Time |
|---|---|---|
| 1 | rara-kernel v0.0.1 | 24.1s |
| 2-5 | (all cache hits at 0.0s) | — |

### Honest read on the data

The two runs are **not** a clean before/after. They differ in cache
state — OFF rebuilt 37 dirty units (kernel + ~36 dependents), ON
rebuilt only `rara-kernel` itself. The "37.8s vs 24.5s" delta is
mostly the cost of those 36 extra dependent crates, which has
nothing to do with the codec choice. The signal that *would* be
informative — the `rara-kernel`-only column — is essentially flat:
**24.3s OFF vs 24.1s ON**. That is well inside noise for `cargo
build --timings`.

Even if the data were clean, this is the wrong scope to look for a
compile-time win. We replaced ~200 LOC of `serde_json::to_vec` /
`from_slice` call-sites in two functions inside `store.rs`. The
rest of `rara-kernel` still depends on `serde`, `serde_json`,
`serde_derive`, and the proc-macro infrastructure that compiles
them. Removing two function bodies' worth of monomorphization does
not move workspace compile time in any measurable way, and we
should not pretend otherwise.

The script `crates/tape-codec-zig/scripts/measure_build.sh` exists
and can be re-run by anyone who wants to gather cleaner data
(both runs from a `cargo clean` baseline). For this PoC, the
honest conclusion is: **no measurable compile-time signal at this
scale**. That answers the question, just not in the direction that
would justify pulling Zig in further.

## Conclusion

- Q1 (build integration): yes, works on macOS, low onboarding cost.
- Q2 (output parity): yes, 1000/1000 byte-identical, two real traps
  found and documented.
- Q3 (compile-time signal): no measurable signal at this scale.

The PoC's reproducer (spec §Intent) called out a concrete bad
outcome — a future bigger Zig migration shipping with key-ordering
or number-formatting drift, silently splitting tapes on disk. This
PoC found exactly those two traps inside 200 LOC and fenced them
with a differential test. That is genuine value, independent of
whether we ever ship Zig in production.

What this PoC does **not** establish: that Zig pays for itself on
compile time. To answer that, you would have to migrate something
heavier than a JSONL codec — for instance, replace one of the top
non-kernel compile units (chromiumoxide, opendal, or a proc-macro
generator) with a Zig equivalent. That is a much larger experiment
and is downstream of this PoC, not the same experiment.

## Recommendation for Phase 1

If we want to keep investigating Zig: pick the single heaviest
compile cost in the workspace where a Zig replacement is plausible
— the most defensible candidate is a JSON-heavy hot path that is
ALSO a leaf dep, so swapping it does not perturb the rest of the
graph. Concretely: pick the JSON parsing inside one of the
integrations crates (e.g. `rara-mcp`'s schema validation, or
`rara-channels`' WS frame parsing) and re-run the same
build/parity/timings experiment there, with both runs starting
from `cargo clean`. If that experiment also shows no compile-time
signal, the answer to "should we adopt Zig" is no, and we close
the door cleanly. If it shows a real signal, we have evidence to
plan a larger migration around.

If we don't want to keep investigating Zig: this PoC is a
tombstone. Keep the differential test as a reference for "this
is what byte-parity testing looks like when you cross a language
boundary", remove the crate in a follow-up issue.

The decision is the user's. The PoC's job was to produce evidence,
and the evidence above is the evidence.

## Spec drift note

The spec text (`specs/issue-2007-tape-codec-zig-poc.spec.md`,
§Decisions) called for the results doc at
`crates/tape-codec-zig/POC_RESULTS.md`. The parent agent
re-directed the file to its current location at
`specs/issue-2007-tape-codec-zig-poc/POC_RESULTS.md`, so it lives
alongside the timing artifacts under `specs/`. The spec should
be updated to match in a follow-up. No behavior is affected; this
is purely documentation placement.
