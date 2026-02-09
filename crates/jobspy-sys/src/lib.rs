mod setup;
pub mod types;

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use pyo3::{prelude::*, types::PyDict};
use types::{ScrapedJob, ScrapeParams, SiteName};

/// A wrapper around the Python `JobSpy` library for scraping job boards.
pub struct JobSpy;

impl JobSpy {
    /// Create a new `JobSpy` instance, ensuring the Python environment is ready.
    pub fn new() -> Result<Self, String> {
        setup::ensure_runtime_env()?;
        Ok(Self)
    }

    /// Scrape jobs from the specified job boards using `python-jobspy`.
    ///
    /// Returns a list of [`ScrapedJob`] results parsed from the Python `DataFrame`.
    pub fn scrape_jobs(&self, params: &ScrapeParams) -> Result<Vec<ScrapedJob>, String> {
        with_interpreter(|py| {
            let jobspy = py
                .import("jobspy")
                .map_err(|e| format!("Failed to import jobspy: {e}"))?;
            let scrape_fn = jobspy
                .getattr("scrape_jobs")
                .map_err(|e| format!("Failed to get scrape_jobs: {e}"))?;

            // Build kwargs
            let kwargs = PyDict::new(py);

            // site_name as list of strings
            let sites: Vec<&str> = params.site_name.iter().map(SiteName::as_python_str).collect();
            kwargs
                .set_item("site_name", sites)
                .map_err(|e| e.to_string())?;

            kwargs
                .set_item("search_term", &params.search_term)
                .map_err(|e| e.to_string())?;

            if let Some(ref loc) = params.location {
                kwargs.set_item("location", loc).map_err(|e| e.to_string())?;
            }
            if let Some(dist) = params.distance {
                kwargs
                    .set_item("distance", dist)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(jt) = params.job_type {
                kwargs
                    .set_item("job_type", jt.as_python_str())
                    .map_err(|e| e.to_string())?;
            }
            if let Some(remote) = params.is_remote {
                kwargs
                    .set_item("is_remote", remote)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(n) = params.results_wanted {
                kwargs
                    .set_item("results_wanted", n)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(h) = params.hours_old {
                kwargs
                    .set_item("hours_old", h)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(ea) = params.easy_apply {
                kwargs
                    .set_item("easy_apply", ea)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(ref c) = params.country_indeed {
                kwargs
                    .set_item("country_indeed", c)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(fetch) = params.linkedin_fetch_description {
                kwargs
                    .set_item("linkedin_fetch_description", fetch)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(enforce) = params.enforce_annual_salary {
                kwargs
                    .set_item("enforce_annual_salary", enforce)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(ref proxies) = params.proxies {
                kwargs
                    .set_item("proxies", proxies)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(v) = params.verbose {
                kwargs
                    .set_item("verbose", v)
                    .map_err(|e| e.to_string())?;
            }

            // Call scrape_jobs(**kwargs) -> DataFrame
            let df = scrape_fn
                .call((), Some(&kwargs))
                .map_err(|e| format!("scrape_jobs failed: {e}"))?;

            // Convert DataFrame to JSON records
            let to_json_kwargs = PyDict::new(py);
            to_json_kwargs
                .set_item("orient", "records")
                .map_err(|e| e.to_string())?;

            let json_str: String = df
                .call_method("to_json", (), Some(&to_json_kwargs))
                .map_err(|e| format!("DataFrame.to_json failed: {e}"))?
                .extract()
                .map_err(|e: PyErr| format!("Failed to extract JSON string: {e}"))?;

            // Parse JSON into Vec<ScrapedJob>
            let jobs: Vec<ScrapedJob> = serde_json::from_str(&json_str)
                .map_err(|e| format!("Failed to parse scraped jobs JSON: {e}"))?;

            Ok(jobs)
        })
    }
}

fn with_interpreter<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(Python<'_>) -> Result<T, String>,
{
    let venv = setup::ensure_runtime_env()?;
    configure_python_env(venv)?;

    Python::attach(|py| {
        configure_sys_path(py, venv)?;
        f(py)
    })
}

/// Configure environment variables for the Python runtime.
///
/// # Safety
///
/// Uses `env::set_var` which is unsafe in Rust 2024 edition. This is called
/// once before the first Python interpreter initialization via `OnceLock`.
#[allow(unsafe_code)]
fn configure_python_env(venv: &ytmusicapi_uv::VenvInfo) -> Result<(), String> {
    static PYTHON_ENV_INIT: OnceLock<Result<(), String>> = OnceLock::new();
    PYTHON_ENV_INIT
        .get_or_init(|| {
            if let Some(home) = python_home_from_venv(venv) {
                // SAFETY: initialized once before first Python interpreter initialization.
                unsafe {
                    env::set_var("PYTHONHOME", &home);

                    // Set library path so the runtime can find libpython
                    let lib_dir = home.join("lib");
                    if lib_dir.exists() {
                        #[cfg(target_os = "macos")]
                        {
                            let current = env::var("DYLD_LIBRARY_PATH").unwrap_or_default();
                            let new_path = if current.is_empty() {
                                lib_dir.to_string_lossy().to_string()
                            } else {
                                format!("{}:{}", lib_dir.to_string_lossy(), current)
                            };
                            env::set_var("DYLD_LIBRARY_PATH", new_path);
                        }

                        #[cfg(target_os = "linux")]
                        {
                            let current = env::var("LD_LIBRARY_PATH").unwrap_or_default();
                            let new_path = if current.is_empty() {
                                lib_dir.to_string_lossy().to_string()
                            } else {
                                format!("{}:{}", lib_dir.to_string_lossy(), current)
                            };
                            env::set_var("LD_LIBRARY_PATH", new_path);
                        }
                    }
                }
            }
            Ok(())
        })
        .clone()
}

fn configure_sys_path(py: Python<'_>, venv: &ytmusicapi_uv::VenvInfo) -> Result<(), String> {
    static SYS_PATH_INIT: OnceLock<Result<(), String>> = OnceLock::new();
    SYS_PATH_INIT
        .get_or_init(|| {
            let sys = py.import("sys").map_err(|e| e.to_string())?;
            let path = sys.getattr("path").map_err(|e| e.to_string())?;

            if let Ok(override_path) = env::var("JOBSPY_PYTHONPATH") {
                path.call_method1("insert", (0, override_path))
                    .map_err(|e| format!("Failed to set JOBSPY_PYTHONPATH: {e}"))?;
            }

            // Add venv site-packages to sys.path
            let site_packages = if cfg!(windows) {
                venv.path.join("Lib").join("site-packages")
            } else {
                detect_venv_site_packages(&venv.path).unwrap_or_else(|| {
                    venv.path
                        .join("lib")
                        .join(format!("python{}", setup::compiled_python_version()))
                        .join("site-packages")
                })
            };

            if site_packages.exists() {
                path.call_method1("insert", (0, site_packages.to_string_lossy().to_string()))
                    .map_err(|e| format!("Failed to add site-packages to sys.path: {e}"))?;
            }

            Ok(())
        })
        .clone()
}

fn python_home_from_venv(venv: &ytmusicapi_uv::VenvInfo) -> Option<PathBuf> {
    let cfg_path = venv.path.join("pyvenv.cfg");
    let text = fs::read_to_string(cfg_path).ok()?;
    let mut home_bin: Option<PathBuf> = None;
    let mut version_mm: Option<String> = None;

    for line in text.lines() {
        let mut parts = line.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();
        if key.eq_ignore_ascii_case("home") {
            home_bin = Some(PathBuf::from(value));
        } else if key.eq_ignore_ascii_case("version_info") {
            // e.g. "3.14.2" -> "3.14"
            let mut seg = value.split('.');
            let major = seg.next().unwrap_or_default();
            let minor = seg.next().unwrap_or_default();
            if !major.is_empty() && !minor.is_empty() {
                version_mm = Some(format!("{major}.{minor}"));
            }
        }
    }

    let home_bin = home_bin?;
    let base = home_bin.parent()?;

    // Homebrew framework Python stores stdlib under:
    // <prefix>/Frameworks/Python.framework/Versions/X.Y
    if let Some(mm) = version_mm {
        let framework_home = base
            .join("Frameworks")
            .join("Python.framework")
            .join("Versions")
            .join(mm);
        if framework_home.exists() {
            return Some(framework_home);
        }
    }

    Some(base.to_path_buf())
}

fn detect_venv_site_packages(venv_path: &Path) -> Option<PathBuf> {
    let lib_dir = venv_path.join("lib");
    let entries = fs::read_dir(lib_dir).ok()?;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name()?.to_str()?;
        if name.starts_with("python3.") {
            return Some(path.join("site-packages"));
        }
    }

    None
}
