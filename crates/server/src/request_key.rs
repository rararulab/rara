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
