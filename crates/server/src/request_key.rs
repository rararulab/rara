// Copyright 2025 Crrow
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

//! Request key extraction for HTTP request deduplication.
//!
//! Produces a deterministic string key from the HTTP method, path, query
//! string, and request body. Two identical requests always produce the same
//! key.

use std::hash::{DefaultHasher, Hash, Hasher};

use http::Method;

/// Build a deterministic cache key from an HTTP request's identity fields.
///
/// The key is a hex-encoded 64-bit SipHash of `method | path | query | body`.
/// Using a hash keeps the key compact regardless of body size.
#[must_use]
pub fn request_key(method: &Method, path: &str, query: Option<&str>, body: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    method.as_str().hash(&mut hasher);
    path.hash(&mut hasher);
    query.unwrap_or("").hash(&mut hasher);
    body.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_input_produces_same_key() {
        let a = request_key(&Method::POST, "/api/v1/jobs/discover", None, b"hello");
        let b = request_key(&Method::POST, "/api/v1/jobs/discover", None, b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn different_method_produces_different_key() {
        let a = request_key(&Method::GET, "/path", None, b"");
        let b = request_key(&Method::POST, "/path", None, b"");
        assert_ne!(a, b);
    }

    #[test]
    fn different_path_produces_different_key() {
        let a = request_key(&Method::GET, "/a", None, b"");
        let b = request_key(&Method::GET, "/b", None, b"");
        assert_ne!(a, b);
    }

    #[test]
    fn different_query_produces_different_key() {
        let a = request_key(&Method::GET, "/p", Some("q=1"), b"");
        let b = request_key(&Method::GET, "/p", Some("q=2"), b"");
        assert_ne!(a, b);
    }

    #[test]
    fn query_none_vs_empty_string() {
        // None and Some("") both hash the empty string, so they should match.
        let a = request_key(&Method::GET, "/p", None, b"");
        let b = request_key(&Method::GET, "/p", Some(""), b"");
        assert_eq!(a, b);
    }

    #[test]
    fn different_body_produces_different_key() {
        let a = request_key(&Method::POST, "/p", None, b"body1");
        let b = request_key(&Method::POST, "/p", None, b"body2");
        assert_ne!(a, b);
    }

    #[test]
    fn key_is_16_hex_chars() {
        let k = request_key(&Method::GET, "/", None, b"");
        assert_eq!(k.len(), 16);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
