// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License").

//! Differential round-trip parity tests against `serde_json` for the
//! Zig codec `PoC`. Binds to BDD scenarios in
//! `specs/issue-2007-tape-codec-zig-poc.spec.md`:
//!
//! - `differential_round_trip_parity` — N=1000 random `TapEntry` values,
//!   byte-equality of `serde_json::to_vec(entry) ==
//!   zig_encode(zig_decode(serde_json::to_vec(entry)))`.
//! - `build_artifact_is_present` — calls a Zig-exported symbol on a simple
//!   input and asserts a non-error return code, proving the static archive
//!   linked.
//!
//! Note on randomness: the spec recommends `proptest`, but the
//! workspace does not currently depend on it. To avoid adding a new
//!  workspace dep for a `PoC`, we generate cases with `rand` (already in
//! the workspace) seeded from a fixed seed for reproducibility. This
//! gives the same coverage signal — a value is either byte-equal or
//! not — without proptest's case-shrinking machinery, which is overkill
//! for a yes/no parity check.

use jiff::Timestamp;
use rand::{RngExt, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// Local mirror of `rara_kernel::memory::TapEntry` and `TapEntryKind`.
// Inlined here to avoid a circular dev-dependency (`rara-kernel` depends
// on `tape-codec-zig` when the kernel's `zig-codec` feature is on; if
// this test crate then dev-depends on `rara-kernel`, cargo refuses).
// Field order, attributes, and `#[serde(rename_all = "snake_case")]`
// match the kernel definitions verbatim — see
// `crates/kernel/src/memory/mod.rs:210` (kind) and 348 (entry). If
// either definition drifts, this differential test will start failing
// against real on-disk tapes; the kernel-side feature-gated codec is
// where any production codec ultimately gets validated.

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TapEntryKind {
    Message,
    ToolCall,
    ToolResult,
    Event,
    System,
    Anchor,
    Note,
    Summary,
    Plan,
    TaskReport,
    FeedEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TapEntry {
    id:        u64,
    kind:      TapEntryKind,
    payload:   Value,
    timestamp: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata:  Option<Value>,
}

const ALL_KINDS: &[TapEntryKind] = &[
    TapEntryKind::Message,
    TapEntryKind::ToolCall,
    TapEntryKind::ToolResult,
    TapEntryKind::Event,
    TapEntryKind::System,
    TapEntryKind::Anchor,
    TapEntryKind::Note,
    TapEntryKind::Summary,
    TapEntryKind::Plan,
    TapEntryKind::TaskReport,
    TapEntryKind::FeedEvent,
];

/// A pool of object keys mixing lex-late and lex-early identifiers so
/// that the lexicographic-ordering invariant is actually exercised. The
/// list intentionally contains keys that sort before/after each other
/// in non-trivial ways (`zebra` < `zoom` but `zebra` > `apple`).
const KEY_POOL: &[&str] = &[
    "z",
    "a",
    "m",
    "foo",
    "bar",
    "zebra",
    "apple",
    "Apple",
    "alpha",
    "10",
    "2",
    "key/with/slash",
    "key.with.dot",
    "k\u{00e9}y",
];

/// A pool of leaf values covering primitives, escape-heavy strings,
/// unicode, and control characters — every code path serde's
/// stringifier handles differently from Zig's.
fn random_leaf(rng: &mut StdRng) -> Value {
    match rng.random_range(0..7) {
        0 => Value::Null,
        1 => Value::Bool(rng.random_bool(0.5)),
        2 => json!(rng.random_range(-1_000_000_i64..1_000_000_i64)),
        // Floats: JSON doesn't represent NaN/Infinity, and serde's
        // round-trip-through-f64 for non-trivial fractions is brittle —
        // restrict to small integer-valued floats so byte-equality
        // holds without requiring deep numeric-format compat.
        3 => json!(f64::from(rng.random_range(-1000_i32..1000_i32))),
        4 => Value::String(String::from("plain ascii string")),
        5 => Value::String(String::from(
            "escapes: \" \\ \n \t \r \u{0008} \u{000c} / control \x01",
        )),
        _ => Value::String(String::from("unicode: \u{4f60}\u{597d} \u{1f600} \u{2603}")),
    }
}

fn random_value(rng: &mut StdRng, depth: u32) -> Value {
    if depth == 0 {
        return random_leaf(rng);
    }
    match rng.random_range(0..6) {
        0 | 1 => random_leaf(rng),
        2 => {
            let n = rng.random_range(0..4);
            Value::Array((0..n).map(|_| random_value(rng, depth - 1)).collect())
        }
        _ => {
            // Object: pick keys without replacement-ish from the pool.
            let n = rng.random_range(0..5);
            let mut obj = serde_json::Map::new();
            for _ in 0..n {
                let k = KEY_POOL[rng.random_range(0..KEY_POOL.len())];
                obj.insert(k.to_string(), random_value(rng, depth - 1));
            }
            Value::Object(obj)
        }
    }
}

fn random_entry(rng: &mut StdRng, id: u64) -> TapEntry {
    let kind = ALL_KINDS[rng.random_range(0..ALL_KINDS.len())];
    let payload = random_value(rng, 3);
    let metadata = if rng.random_bool(0.5) {
        Some(random_value(rng, 2))
    } else {
        None
    };
    // Deterministic timestamp from the rng so the test is reproducible.
    let ts_ms = rng.random_range(0_i64..2_000_000_000_000_i64);
    let timestamp = Timestamp::from_millisecond(ts_ms).expect("valid timestamp");
    TapEntry {
        id,
        kind,
        payload,
        timestamp,
        metadata,
    }
}

#[test]
fn build_artifact_is_present() {
    // The simplest possible JSON value. If the static archive linked,
    // this returns Ok bytes; if anything in the FFI chain is broken
    // (build.rs didn't run, symbol missing, ABI mismatch) the test
    // fails to even link.
    let out = tape_codec_zig::decode(b"{}").expect("zig codec round-trip on `{}`");
    assert_eq!(out, b"{}");
    assert!(tape_codec_zig::is_enabled());
}

const CASES: usize = 1000;

#[test]
fn differential_round_trip_parity() {
    use std::fmt::Write as _;

    // Fixed seed so a regression reproduces deterministically.
    let mut rng = StdRng::seed_from_u64(0xCAFE_F00D_DEAD_BEEF);

    let mut mismatches: Vec<(usize, Vec<u8>, Vec<u8>)> = Vec::new();

    for i in 0..CASES {
        let entry = random_entry(&mut rng, i as u64);
        let bytes_a = serde_json::to_vec(&entry).expect("serde_json::to_vec");

        // decode then encode — both call into the same Zig round-trip.
        let decoded = tape_codec_zig::decode(&bytes_a).expect("zig decode");
        let bytes_b = tape_codec_zig::encode(&decoded).expect("zig encode");

        if bytes_a != bytes_b {
            mismatches.push((i, bytes_a, bytes_b));
            if mismatches.len() >= 3 {
                break;
            }
        }
    }

    if !mismatches.is_empty() {
        let mut report = String::new();
        for (i, a, b) in &mismatches {
            let _ = writeln!(
                report,
                "case {i}:\n  serde: {}\n  zig:   {}",
                String::from_utf8_lossy(a),
                String::from_utf8_lossy(b),
            );
        }
        panic!(
            "{} of {} differential cases mismatched (showing up to 3):\n{}",
            mismatches.len(),
            CASES,
            report
        );
    }
}
