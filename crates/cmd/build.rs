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

fn main() {
    shadow_rs::ShadowBuilder::builder()
        .build()
        .expect("Failed to acquire build-time information");

    // Sync bundled prompt files to config directory.
    sync_prompts();
}

/// Copy all prompt files from the source `prompts/` directory into the
/// user's config directory (`~/.config/job/prompts/` on macOS,
/// `$XDG_CONFIG_HOME/job/prompts/` on Linux).
///
/// Files are always overwritten so that the source code remains the single
/// source of truth for compiled-in defaults.
fn sync_prompts() {
    let prompts_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");

    if !prompts_src.exists() {
        return;
    }

    // Determine config dir (simplified -- matches rara_paths logic).
    let config_dir = if cfg!(target_os = "macos") {
        dirs::home_dir()
            .expect("home dir")
            .join(".config/job")
    } else {
        dirs::config_dir()
            .expect("config dir")
            .join("job")
    };
    let prompt_dir = config_dir.join("prompts");

    for entry in walkdir::WalkDir::new(&prompts_src)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
    {
        let rel = entry.path().strip_prefix(&prompts_src).unwrap();
        let target = prompt_dir.join(rel);
        // Always overwrite -- source code is single source of truth.
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::copy(entry.path(), &target).ok();
    }
}
