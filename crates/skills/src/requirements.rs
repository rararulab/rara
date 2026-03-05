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

//! Binary requirement checking and dependency installation.

use std::path::Path;

use snafu::ResultExt;

use crate::{
    error::{InstallSnafu, InvalidInputSnafu, IoSnafu, Result},
    types::{InstallKind, InstallSpec, SkillEligibility, SkillMetadata},
};

/// Resolve install command program + args from an install spec.
pub fn install_program_and_args(spec: &InstallSpec) -> Result<(&'static str, Vec<&str>)> {
    let (program, args) = match &spec.kind {
        InstallKind::Brew => {
            let formula = spec.formula.as_deref().ok_or_else(|| {
                InvalidInputSnafu {
                    message: "brew install requires 'formula'",
                }
                .build()
            })?;
            ("brew", vec!["install", formula])
        }
        InstallKind::Npm => {
            let package = spec.package.as_deref().ok_or_else(|| {
                InvalidInputSnafu {
                    message: "npm install requires 'package'",
                }
                .build()
            })?;
            ("npm", vec!["install", "-g", "--ignore-scripts", package])
        }
        InstallKind::Go => {
            let module = spec.module.as_deref().ok_or_else(|| {
                InvalidInputSnafu {
                    message: "go install requires 'module'",
                }
                .build()
            })?;
            ("go", vec!["install", module])
        }
        InstallKind::Cargo => {
            let package = spec.package.as_deref().ok_or_else(|| {
                InvalidInputSnafu {
                    message: "cargo install requires 'package'",
                }
                .build()
            })?;
            ("cargo", vec!["install", package])
        }
        InstallKind::Uv => {
            let package = spec.package.as_deref().ok_or_else(|| {
                InvalidInputSnafu {
                    message: "uv install requires 'package'",
                }
                .build()
            })?;
            ("uv", vec!["tool", "install", package])
        }
        InstallKind::Download => {
            return InstallSnafu {
                message: "download install kind is not yet supported for automatic installation",
            }
            .fail();
        }
    };

    Ok((program, args))
}

/// Render an install spec to a user-visible command preview.
pub fn install_command_preview(spec: &InstallSpec) -> Result<String> {
    let (program, args) = install_program_and_args(spec)?;
    Ok(std::iter::once(program)
        .chain(args)
        .collect::<Vec<_>>()
        .join(" "))
}

/// Returns the current OS identifier used for platform filtering.
pub fn current_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

/// Check whether a binary exists in PATH.
pub fn check_bin(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() && is_executable(&candidate) {
                return true;
            }
        }
    }
    false
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool { true }

/// Check all requirements for a skill and return eligibility info.
pub fn check_requirements(meta: &SkillMetadata) -> SkillEligibility {
    let req = &meta.requires;

    // If no requirements declared, skill is eligible
    if req.bins.is_empty() && req.any_bins.is_empty() {
        return SkillEligibility {
            eligible:        true,
            missing_bins:    Vec::new(),
            install_options: Vec::new(),
        };
    }

    let mut missing = Vec::new();

    // All bins must exist
    for bin in &req.bins {
        if !check_bin(bin) {
            missing.push(bin.clone());
        }
    }

    // At least one of any_bins must exist
    if !req.any_bins.is_empty() && !req.any_bins.iter().any(|b| check_bin(b)) {
        // All are missing -- report all of them
        for bin in &req.any_bins {
            missing.push(bin.clone());
        }
    }

    let os = current_os();
    let install_options: Vec<InstallSpec> = req
        .install
        .iter()
        .filter(|spec| spec.os.is_empty() || spec.os.iter().any(|o| o == os))
        .cloned()
        .collect();

    SkillEligibility {
        eligible: missing.is_empty(),
        missing_bins: missing,
        install_options,
    }
}

/// Result of running an install command.
#[derive(Debug)]
pub struct InstallResult {
    pub success: bool,
    pub stdout:  String,
    pub stderr:  String,
}

/// Run an install spec command (e.g. `brew install <formula>`).
pub async fn run_install(spec: &InstallSpec) -> Result<InstallResult> {
    let (program, args) = install_program_and_args(spec)?;

    let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let output = tokio::process::Command::new(program)
        .args(&args_owned)
        .output()
        .await
        .context(IoSnafu)?;

    Ok(InstallResult {
        success: output.status.success(),
        stdout:  String::from_utf8_lossy(&output.stdout).to_string(),
        stderr:  String::from_utf8_lossy(&output.stderr).to_string(),
    })
}
