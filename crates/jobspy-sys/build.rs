use std::{env, fs, path::{Path, PathBuf}};

use pyo3_build_config::get as pyo3_get;
use ytmusicapi_uv::UvManager;

fn main() {
    println!("cargo:rerun-if-env-changed=JOBSPY_PYTHON_VERSION");
    println!("cargo:rerun-if-env-changed=JOBSPY_SKIP_SETUP");

    // Allow skipping auto-setup for CI or special scenarios
    if env::var("JOBSPY_SKIP_SETUP").is_ok() {
        return;
    }

    let cache_dir = dirs::cache_dir()
        .expect("Unable to get cache directory")
        .join("jobspy");

    // Initialize UV manager
    let uv = UvManager::new(cache_dir.clone()).expect("Failed to initialize UV");

    // Install Python
    let python_version =
        env::var("JOBSPY_PYTHON_VERSION").unwrap_or_else(|_| "3.10".to_string());
    let python = uv
        .ensure_python(&python_version)
        .expect("Failed to install Python");

    // Create virtual environment
    let venv_path = cache_dir.join(format!("venv-py{python_version}"));
    let venv = uv
        .create_venv(&venv_path, &python)
        .expect("Failed to create virtual environment");

    // Install python-jobspy
    if !uv
        .is_package_installed(&venv, "jobspy")
        .unwrap_or(false)
    {
        println!("cargo:warning=Installing python-jobspy...");
        uv.install_package(&venv, "python-jobspy")
            .expect("Failed to install python-jobspy");
    }

    // Set pyo3 compilation environment variables
    println!("cargo:rustc-env=PYO3_PYTHON={}", venv.python_exe.display());
    let compiled = pyo3_get().version;
    let compiled_python_version = format!("{}.{}", compiled.major, compiled.minor);
    println!("cargo:rustc-env=JOBSPY_COMPILED_PYTHON_VERSION={compiled_python_version}");

    if let Some(home) = venv.python_exe.parent().and_then(|p| p.parent()) {
        println!("cargo:rustc-env=PYTHON_HOME={}", home.display());

        // Find Python shared library location
        // uv-installed Python has libs in lib/pythonX.Y/config-X.Y-<arch>-linux-gnu/
        let lib_dir = home.join("lib");
        if lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());

            // Also search in lib/pythonX.Y/config-* for the actual shared library
            if let Ok(entries) = std::fs::read_dir(&lib_dir) {
                for entry in entries.filter_map(Result::ok) {
                    let path = entry.path();
                    if path.is_dir()
                        && path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("python"))
                    {
                        // Found pythonX.Y directory
                        if let Ok(config_entries) = std::fs::read_dir(&path) {
                            for config_entry in config_entries.filter_map(Result::ok) {
                                let config_path = config_entry.path();
                                if config_path.is_dir()
                                    && config_path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .is_some_and(|n| n.starts_with("config-"))
                                {
                                    println!(
                                        "cargo:rustc-link-search=native={}",
                                        config_path.display()
                                    );
                                }
                            }
                        }
                    }
                }
            }

            #[cfg(unix)]
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
        }
    }

    // Save build configuration for runtime use
    let config = BuildConfig {
        python_version: python.version,
        venv_path,
        python_exe: venv.python_exe,
    };
    write_build_config(&cache_dir, &config);
}

#[derive(serde::Serialize)]
struct BuildConfig {
    python_version: String,
    venv_path:      PathBuf,
    python_exe:     PathBuf,
}

// IMPORTANT: Keep BuildConfig fields in sync with setup.rs BuildConfig
fn write_build_config(cache_dir: &Path, config: &BuildConfig) {
    let config_path = cache_dir.join("build-config.json");
    let json = serde_json::to_string_pretty(config).unwrap();
    fs::create_dir_all(cache_dir).ok();
    fs::write(config_path, json).ok();
}
