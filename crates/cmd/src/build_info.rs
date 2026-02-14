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

use shadow_rs::shadow;

shadow!(build);

/// Package author information from Cargo.toml
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");

/// Returns true if this is an official release build (`KISEKI_RELEASE` env var
/// is set)
const fn is_official_release() -> bool { option_env!("KISEKI_RELEASE").is_some() }

/// Dirty status suffix derived from git state
const DIRTY_SUFFIX: &str = if build::GIT_CLEAN { "" } else { "-dirty" };

/// Full version string with optional development suffix
///
/// For official releases: uses `PKG_VERSION` as-is
/// For development builds: appends "-unofficial" or
/// "-unofficial+{hash}{-dirty}"
#[allow(clippy::const_is_empty)]
pub const FULL_VERSION: &str = {
    if is_official_release() {
        build::PKG_VERSION
    } else if build::SHORT_COMMIT.is_empty() {
        shadow_rs::formatcp!("{}-unofficial", build::PKG_VERSION)
    } else {
        shadow_rs::formatcp!(
            "{}-unofficial+{}{}",
            build::PKG_VERSION,
            build::SHORT_COMMIT,
            DIRTY_SUFFIX
        )
    }
};
