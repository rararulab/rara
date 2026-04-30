// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Proof-of-concept JSONL codec for `TapEntry` implemented in Zig 0.16.
//!
//! See `specs/issue-2007-tape-codec-zig-poc.spec.md` and the colocated
//! `POC_RESULTS.md` for context. This crate exposes a thin Rust wrapper
//! over a Zig static library compiled by `build.rs`. The Rust API is
//! byte-oriented: callers hand in JSON bytes and a buffer, and the Zig
//! side parses + re-emits the value with object keys sorted to match
//! `serde_json`'s default output.
//!
//! Two cargo features:
//!
//! - `zig-codec` (default): build and link the Zig static lib. Requires `zig`
//!   0.16 on `PATH`.
//! - feature absent: every entry point returns `Error::FeatureDisabled`. This
//!   lets `rara-kernel` depend on this crate unconditionally and gate just the
//!   codec swap behind its own `zig-codec` feature.
//!
//! NOT a production codec. The `PoC`'s goal is to answer build-integration
//! and output-parity questions; see `differential.rs` for the parity
//! test and the spec for the larger context.

use snafu::Snafu;

/// Result alias for codec operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by the Zig codec wrapper.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    /// The crate was built without the `zig-codec` feature; no static
    /// archive was linked, so the FFI entry points are unavailable.
    #[snafu(display("tape-codec-zig was built without the `zig-codec` feature"))]
    FeatureDisabled,

    /// Input bytes did not parse as JSON.
    #[snafu(display("input is not valid JSON"))]
    InvalidJson,

    /// Zig allocator failed (out of memory inside the codec arena).
    #[snafu(display("zig codec ran out of memory"))]
    OutOfMemory,

    /// The caller-provided output buffer was too small. `required` is
    /// the number of bytes the codec needed.
    #[snafu(display("output buffer too small (need {required} bytes)"))]
    BufferTooSmall {
        /// Bytes the codec needed but could not write.
        required: usize,
    },

    /// Unknown return code — should not happen unless the FFI contract
    /// drifts.
    #[snafu(display("zig codec returned unknown code {code}"))]
    UnknownCode {
        /// The unrecognized i32 the static lib returned.
        code: i32,
    },
}

#[cfg(feature = "zig-codec")]
unsafe extern "C" {
    fn tape_codec_zig_decode(
        in_ptr: *const u8,
        in_len: usize,
        out_ptr: *mut u8,
        out_capacity: usize,
        out_written: *mut usize,
    ) -> i32;

    fn tape_codec_zig_encode(
        in_ptr: *const u8,
        in_len: usize,
        out_ptr: *mut u8,
        out_capacity: usize,
        out_written: *mut usize,
    ) -> i32;
}

const RC_OK: i32 = 0;
const RC_INVALID_JSON: i32 = 1;
const RC_OUT_OF_MEMORY: i32 = 2;
const RC_BUFFER_TOO_SMALL: i32 = 3;

/// Decode JSON bytes through the Zig codec.
///
/// In this `PoC` the FFI surface is symmetric (decode and encode both
/// round-trip through `std.json.Value`); the names exist to match the
/// future production split where `decode` validates input and `encode`
/// produces canonical output. The differential test (`tests/`) calls
/// both back-to-back and asserts byte-equality with `serde_json`.
pub fn decode(input: &[u8]) -> Result<Vec<u8>> { call_codec(input, CodecOp::Decode) }

/// Encode JSON bytes through the Zig codec. See [`decode`] for the
/// shared round-trip semantics.
pub fn encode(input: &[u8]) -> Result<Vec<u8>> { call_codec(input, CodecOp::Encode) }

#[derive(Copy, Clone)]
enum CodecOp {
    Decode,
    Encode,
}

#[cfg(feature = "zig-codec")]
fn call_codec(input: &[u8], op: CodecOp) -> Result<Vec<u8>> {
    // Heuristic initial capacity: the round-tripped JSON is rarely
    // larger than 2x the input. On `BufferTooSmall` we resize to the
    // exact `required` value the codec reports and retry once.
    let mut capacity = input.len().saturating_mul(2).max(64);
    let mut buf: Vec<u8> = vec![0; capacity];
    loop {
        let mut written: usize = 0;
        // SAFETY: `buf` and `input` are valid for `capacity` and
        // `input.len()` bytes respectively; `&mut written` is a valid
        // pointer to a `usize` for the duration of the call. The Zig
        // side reads `in_len` bytes, writes at most `out_capacity`
        // bytes, and stores the actual write count via `out_written`.
        #[allow(unsafe_code)]
        let rc = unsafe {
            match op {
                CodecOp::Decode => tape_codec_zig_decode(
                    input.as_ptr(),
                    input.len(),
                    buf.as_mut_ptr(),
                    capacity,
                    &raw mut written,
                ),
                CodecOp::Encode => tape_codec_zig_encode(
                    input.as_ptr(),
                    input.len(),
                    buf.as_mut_ptr(),
                    capacity,
                    &raw mut written,
                ),
            }
        };
        match rc {
            RC_OK => {
                buf.truncate(written);
                return Ok(buf);
            }
            RC_INVALID_JSON => return Err(Error::InvalidJson),
            RC_OUT_OF_MEMORY => return Err(Error::OutOfMemory),
            RC_BUFFER_TOO_SMALL => {
                if written <= capacity {
                    // Codec reported BufferTooSmall but the recorded
                    // requirement does not exceed our buffer — treat as
                    // protocol drift rather than retry forever.
                    return Err(Error::BufferTooSmall { required: written });
                }
                capacity = written;
                buf.resize(capacity, 0);
            }
            other => return Err(Error::UnknownCode { code: other }),
        }
    }
}

#[cfg(not(feature = "zig-codec"))]
fn call_codec(_input: &[u8], _op: CodecOp) -> Result<Vec<u8>> { Err(Error::FeatureDisabled) }

/// Whether this build linked the Zig static library.
#[must_use]
pub const fn is_enabled() -> bool { cfg!(feature = "zig-codec") }
