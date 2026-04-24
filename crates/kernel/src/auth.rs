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

//! Shared authentication primitives.
//!
//! These helpers are consumed by every surface that terminates an owner-token
//! check — the legacy WebSocket query-string path in `rara-channels::web`
//! and the new Bearer-token middleware in `rara-backend-admin::auth`. Keeping
//! the comparison in one place guarantees a single, reviewed timing-safe
//! implementation.

use subtle::ConstantTimeEq;

/// Compare an expected owner token against a caller-provided value using a
/// constant-time byte comparison.
///
/// Both arguments are treated as opaque byte strings; length differences do
/// not short-circuit. Callers are responsible for rejecting empty `provided`
/// values before invocation if that is desired behaviour at the transport
/// layer.
#[must_use]
pub fn verify_owner_token(expected: &str, provided: &str) -> bool {
    expected.as_bytes().ct_eq(provided.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::verify_owner_token;

    #[test]
    fn matching_tokens_accepted() {
        assert!(verify_owner_token("s3cret", "s3cret"));
    }

    #[test]
    fn different_tokens_rejected() {
        assert!(!verify_owner_token("s3cret", "s3cret-nope"));
        assert!(!verify_owner_token("s3cret", "other"));
    }

    #[test]
    fn empty_provided_rejected_when_expected_nonempty() {
        assert!(!verify_owner_token("s3cret", ""));
    }

    #[test]
    fn empty_expected_accepts_empty_provided() {
        // Policy decision: callers must reject empty inputs before calling;
        // here we only guarantee bytewise equality.
        assert!(verify_owner_token("", ""));
    }
}
