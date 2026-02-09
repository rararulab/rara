use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};
use ytmusicapi_uv::{UvManager, VenvInfo};

static RUNTIME_ENV: OnceLock<Result<VenvInfo, String>> = OnceLock::new();

/// Ensure runtime environment is ready (supports binary distribution scenario).
pub fn ensure_runtime_env() -> Result<&'static VenvInfo, String> {
    RUNTIME_ENV
        .get_or_init(setup_runtime_env)
        .as_ref()
        .map_err(Clone::clone)
}

fn setup_runtime_env() -> Result<VenvInfo, String> {
    let cache_dir = dirs::cache_dir()
        .ok_or("Unable to get cache directory")?
        .join("jobspy");

    // 1. Check if build-time configuration exists
    if let Some(venv) = try_load_build_config(&cache_dir)?
        && is_expected_python_version(&venv)
        && verify_venv(&venv)
    {
        return Ok(venv);
    }

    // 2. Runtime re-installation (supports binary distribution)
    eprintln!("First run, initializing Python environment...");
    install_runtime_env(&cache_dir)
}

fn try_load_build_config(cache_dir: &Path) -> Result<Option<VenvInfo>, String> {
    let config_path = cache_dir.join("build-config.json");
    if !config_path.exists() {
        return Ok(None);
    }

    let json =
        std::fs::read_to_string(&config_path).map_err(|e| format!("Failed to read config: {e}"))?;

    let config: BuildConfig =
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse config: {e}"))?;

    Ok(Some(VenvInfo {
        path:       config.venv_path,
        python_exe: config.python_exe,
    }))
}

fn verify_venv(venv: &VenvInfo) -> bool {
    // Check if Python executable exists
    if !venv.python_exe.exists() {
        return false;
    }

    // Check if jobspy is installed
    let check = std::process::Command::new(&venv.python_exe)
        .args(["-c", "import jobspy"])
        .output();

    matches!(check, Ok(output) if output.status.success())
}

fn install_runtime_env(cache_dir: &Path) -> Result<VenvInfo, String> {
    let uv = UvManager::new(cache_dir.to_path_buf())
        .map_err(|e| format!("Failed to initialize UV: {e}"))?;

    let python_version = compiled_python_version();
    let python = uv
        .ensure_python(python_version)
        .map_err(|e| format!("Failed to install Python: {e}"))?;

    let venv_path = cache_dir.join(format!("venv-py{python_version}"));
    let venv = uv
        .create_venv(&venv_path, &python)
        .map_err(|e| format!("Failed to create venv: {e}"))?;

    uv.install_package(&venv, "python-jobspy")
        .map_err(|e| format!("Failed to install python-jobspy: {e}"))?;

    // Save configuration for next time
    let config = BuildConfig {
        python_version: python.version,
        venv_path,
        python_exe: venv.python_exe.clone(),
    };
    write_build_config(cache_dir, &config);

    Ok(venv)
}

pub(crate) const fn compiled_python_version() -> &'static str {
    env!("JOBSPY_COMPILED_PYTHON_VERSION")
}

fn is_expected_python_version(venv: &VenvInfo) -> bool {
    let expected = compiled_python_version();
    let check = std::process::Command::new(&venv.python_exe)
        .args([
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
        ])
        .output();

    matches!(check, Ok(output) if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == expected)
}

// IMPORTANT: Keep BuildConfig fields in sync with build.rs BuildConfig
#[derive(Serialize, Deserialize)]
struct BuildConfig {
    python_version: String,
    venv_path:      PathBuf,
    python_exe:     PathBuf,
}

fn write_build_config(cache_dir: &Path, config: &BuildConfig) {
    let config_path = cache_dir.join("build-config.json");
    let json = serde_json::to_string_pretty(config).unwrap();
    std::fs::create_dir_all(cache_dir).ok();
    std::fs::write(config_path, json).ok();
}
