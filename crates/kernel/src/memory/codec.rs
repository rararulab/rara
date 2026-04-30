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

//! Pluggable JSONL line codec for tape entries.
//!
//! Behind the default feature set, both [`encode_entry`] and
//! [`decode_entry`] call straight into `serde_json`. When the
//! `zig-codec` feature is enabled, [`encode_entry`] additionally routes
//! the bytes through the Zig static library
//! (`tape_codec_zig::encode`) so we can validate parity end-to-end.
//! [`decode_entry`] still hands off to `serde_json::from_slice` because
//! the Rust struct is the canonical in-memory shape — the Zig codec's
//! job is producing on-disk bytes, not Rust values.
//!
//! See `crates/tape-codec-zig/AGENT.md` and
//! `specs/issue-2007-tape-codec-zig-poc.spec.md` for the larger context.

use snafu::ResultExt;

use crate::memory::{TapEntry, TapResult, error};

/// Decode a single JSONL line into a [`TapEntry`].
pub(super) fn decode_entry(line: &[u8]) -> TapResult<TapEntry> {
    serde_json::from_slice::<TapEntry>(line).context(error::JsonDecodeSnafu)
}

/// Encode a single [`TapEntry`] into JSON bytes (no trailing newline;
/// callers append `\n`).
pub(super) fn encode_entry(entry: &TapEntry) -> TapResult<Vec<u8>> {
    let serde_bytes = serde_json::to_vec(entry).context(error::JsonEncodeSnafu)?;

    #[cfg(feature = "zig-codec")]
    {
        // Route through Zig's `std.json` round-trip. The differential
        // test in `tape-codec-zig` is what guarantees byte parity with
        // the serde path; here we just propagate any FFI failure as a
        // tape-local error so the rest of the kernel sees a normal
        // `TapResult`.
        tape_codec_zig::encode(&serde_bytes).map_err(|e| error::TapError::ZigCodec {
            message: e.to_string(),
        })
    }

    #[cfg(not(feature = "zig-codec"))]
    Ok(serde_bytes)
}
