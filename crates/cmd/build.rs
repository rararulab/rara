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

use std::{env, fs, path::PathBuf};

fn main() {
    shadow_rs::ShadowBuilder::builder()
        .build()
        .expect("Failed to acquire build-time information");

    generate_boxlite_version();
}

/// Derive `BOXLITE_VERSION` from the workspace `Cargo.lock` and write it to
/// `$OUT_DIR/boxlite_version.rs` for `crates/cmd/src/setup/boxlite.rs` to
/// `include!`. The sandbox crate's `tag = "..."` is the single source of
/// truth; we read whatever cargo already resolved.
///
/// Failure modes (`Cargo.lock` missing, boxlite entry missing, malformed
/// `source` URL) all panic loudly — they mean the checkout is not a valid
/// rara workspace.
fn generate_boxlite_version() {
    // Build script CWD is `crates/cmd/`; workspace root is two levels up.
    let lock_path = PathBuf::from("../../Cargo.lock");
    println!("cargo:rerun-if-changed=../../Cargo.lock");

    let contents = fs::read_to_string(&lock_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", lock_path.display()));

    let tag = extract_boxlite_tag(&contents).unwrap_or_else(|e| {
        panic!(
            "failed to extract boxlite tag from {}: {e}",
            lock_path.display()
        )
    });

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));
    let dest = out_dir.join("boxlite_version.rs");
    let body = format!(
        "/// Boxlite git tag (e.g. `v0.9.4`), derived at build time from `Cargo.lock`.\nconst \
         BOXLITE_VERSION: &str = \"{tag}\";\n"
    );
    fs::write(&dest, body).unwrap_or_else(|e| panic!("failed to write {}: {e}", dest.display()));
}

/// Scan `Cargo.lock` for the `[[package]] name = "boxlite"` entry and pull
/// the `?tag=<value>` query parameter out of its `source` URL.
///
/// Cargo.lock has a stable shape — `[[package]]` blocks separated by blank
/// lines, fields one-per-line. A linear scan is simpler than pulling in a
/// TOML parser as a build-dep.
fn extract_boxlite_tag(lock: &str) -> Result<String, String> {
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() != r#"name = "boxlite""# {
            continue;
        }
        // Found the boxlite package block. The `source` line is one of the
        // next few lines (cargo emits `name`, `version`, `source` in that
        // order). Scan until the block ends (blank line) or we find it.
        for next in lines.by_ref() {
            if next.is_empty() {
                break;
            }
            let Some(rest) = next.strip_prefix("source = \"") else {
                continue;
            };
            let Some(url) = rest.strip_suffix('"') else {
                return Err(format!("malformed source line: {next}"));
            };
            // URL shape: git+https://...?tag=vX.Y.Z#<sha>
            let Some(after_tag) = url.split("?tag=").nth(1) else {
                return Err(format!("source URL has no `?tag=` parameter: {url}"));
            };
            let tag = after_tag
                .split(['#', '&'])
                .next()
                .expect("split always yields at least one element");
            if tag.is_empty() {
                return Err(format!("source URL has empty tag: {url}"));
            }
            return Ok(tag.to_owned());
        }
        return Err("boxlite package block has no `source` line".to_owned());
    }
    Err("Cargo.lock has no `[[package]] name = \"boxlite\"` entry".to_owned())
}
