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

//! Content hashing for skill change detection.

use std::path::Path;

use sha2::{Digest, Sha256};
use snafu::ResultExt;

use crate::error::{IoSnafu, Result};

/// Compute hex-encoded SHA-256 hash of a file's contents.
pub fn file_hash(path: &Path) -> Result<String> {
    let content = std::fs::read(path).context(IoSnafu)?;
    let digest = Sha256::digest(&content);
    Ok(format!("{digest:x}"))
}
