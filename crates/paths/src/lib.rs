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

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

static HOME_DIR: OnceLock<PathBuf> = OnceLock::new();

/// A custom data directory override, set only by `set_custom_data_dir`.
/// This is used to override the default data directory location.
/// The directory will be created if it doesn't exist when set.
static CUSTOM_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The resolved data directory, combining custom override or platform defaults.
/// This is set once and cached for subsequent calls.
/// On macOS, this is `~/Library/Application Support/rsketch`.
/// On Linux/FreeBSD, this is `$XDG_DATA_HOME/rsketch`.
static CURRENT_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The resolved config directory, combining custom override or platform
/// defaults. This is set once and cached for subsequent calls.
/// On macOS, this is `~/.config/rsketch`.
/// On Linux/FreeBSD, this is `$XDG_CONFIG_HOME/rsketch`.
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Returns the path to the user's home directory.
///
/// # Panics
///
/// Panics if the current user's home directory cannot be determined.
pub fn home_dir() -> &'static PathBuf {
    HOME_DIR.get_or_init(|| dirs::home_dir().expect("failed to determine home directory"))
}

/// Returns the path to the configuration directory used by rsketch.
///
/// # Panics
///
/// Panics if the platform configuration directory cannot be determined.
pub fn config_dir() -> &'static PathBuf {
    CONFIG_DIR.get_or_init(|| {
        CUSTOM_DATA_DIR.get().map_or_else(
            || {
                if cfg!(target_os = "windows") {
                    dirs::config_dir()
                        .expect("failed to determine RoamingAppData directory")
                        .join("rsketch")
                } else if cfg!(any(target_os = "linux", target_os = "freebsd")) {
                    std::env::var("FLATPAK_XDG_CONFIG_HOME")
                        .map_or_else(
                            |_| {
                                dirs::config_dir()
                                    .expect("failed to determine XDG_CONFIG_HOME directory")
                            },
                            PathBuf::from,
                        )
                        .join("rsketch")
                } else {
                    home_dir().join(".config").join("rsketch")
                }
            },
            |custom_dir| custom_dir.join("config"),
        )
    })
}

/// Returns the path to the data directory used by rsketch.
///
/// # Panics
///
/// Panics if the platform data directory cannot be determined.
pub fn data_dir() -> &'static PathBuf {
    CURRENT_DATA_DIR.get_or_init(|| {
        CUSTOM_DATA_DIR.get().map_or_else(
            || {
                if cfg!(any(target_os = "linux", target_os = "freebsd")) {
                    std::env::var("FLATPAK_XDG_DATA_HOME")
                        .map_or_else(
                            |_| {
                                dirs::data_local_dir()
                                    .expect("failed to determine XDG_DATA_HOME directory")
                            },
                            PathBuf::from,
                        )
                        .join("rsketch")
                } else {
                    dirs::data_local_dir()
                        .expect("failed to determine LocalAppData directory")
                        .join("rsketch")
                }
            },
            Clone::clone,
        )
    })
}

/// Sets a custom directory for all user data, overriding the default data
/// directory.
///
/// This function must be called before any other path operations that depend on
/// the data directory. The directory's path will be canonicalized to an
/// absolute path by a blocking FS operation. The directory will be created if
/// it doesn't exist.
///
/// # Arguments
///
/// * `dir` - The path to use as the custom data directory. This will be used as
///   the base directory for all user data, including databases, extensions, and
///   logs.
///
/// # Returns
///
/// A reference to the static `PathBuf` containing the custom data directory
/// path.
///
/// # Panics
///
/// Panics if:
/// * Called after the data directory has been initialized (e.g., via `data_dir`
///   or `config_dir`)
/// * The directory's path cannot be canonicalized to an absolute path
/// * The directory cannot be created
pub fn set_custom_data_dir<P: ?Sized + AsRef<Path>>(dir: &P) -> &'static PathBuf {
    assert!(
        !(CURRENT_DATA_DIR.get().is_some() || CONFIG_DIR.get().is_some()),
        "set_custom_data_dir called after data_dir or config_dir was initialized"
    );
    CUSTOM_DATA_DIR.get_or_init(|| {
        let mut path = dir.as_ref().to_path_buf();
        if path.is_relative()
            && let Ok(abs) = path.canonicalize()
        {
            path = abs;
        }

        std::fs::create_dir_all(&path).unwrap_or_else(|e| {
            panic!(
                "failed to create custom data directory {}: {e}",
                path.display()
            )
        });

        path
    })
}

/// Returns the path to the temp directory used by rsketch.
///
/// # Panics
///
/// Panics if the platform cache directory cannot be determined.
pub fn temp_dir() -> &'static PathBuf {
    static TEMP_DIR: OnceLock<PathBuf> = OnceLock::new();
    TEMP_DIR.get_or_init(|| {
        if cfg!(target_os = "macos") {
            return dirs::cache_dir()
                .expect("failed to determine cachesDirectory directory")
                .join("rsketch");
        }

        if cfg!(target_os = "windows") {
            return dirs::cache_dir()
                .expect("failed to determine LocalAppData directory")
                .join("rsketch");
        }

        if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return std::env::var("FLATPAK_XDG_CACHE_HOME")
                .map_or_else(
                    |_| dirs::cache_dir().expect("failed to determine XDG_CACHE_HOME directory"),
                    PathBuf::from,
                )
                .join("rsketch");
        }

        home_dir().join(".cache").join("rsketch")
    })
}

/// Returns the path to the hang traces directory.
pub fn hang_traces_dir() -> &'static PathBuf {
    static LOGS_DIR: OnceLock<PathBuf> = OnceLock::new();
    LOGS_DIR.get_or_init(|| data_dir().join("hang_traces"))
}

/// Returns the path to the logs directory.
pub fn logs_dir() -> &'static PathBuf {
    static LOGS_DIR: OnceLock<PathBuf> = OnceLock::new();
    LOGS_DIR.get_or_init(|| {
        if cfg!(target_os = "macos") {
            home_dir().join("Library/Logs/rsketch")
        } else {
            data_dir().join("logs")
        }
    })
}

/// Returns the path to the `rsketch.log` file.
pub fn log_file() -> &'static PathBuf {
    static LOG_FILE: OnceLock<PathBuf> = OnceLock::new();
    LOG_FILE.get_or_init(|| logs_dir().join("rsketch.log"))
}

/// Returns the path to the database directory.
pub fn database_dir() -> &'static PathBuf {
    static DATABASE_DIR: OnceLock<PathBuf> = OnceLock::new();
    DATABASE_DIR.get_or_init(|| data_dir().join("db"))
}

/// Returns the path to the crashes directory, if it exists for the current
/// platform.
pub fn crashes_dir() -> &'static Option<PathBuf> {
    static CRASHES_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    CRASHES_DIR.get_or_init(|| {
        cfg!(target_os = "macos").then_some(home_dir().join("Library/Logs/DiagnosticReports"))
    })
}

/// Returns the path to the retired crashes directory, if it exists for the
/// current platform.
pub fn crashes_retired_dir() -> &'static Option<PathBuf> {
    static CRASHES_RETIRED_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    CRASHES_RETIRED_DIR.get_or_init(|| crashes_dir().as_ref().map(|dir| dir.join("Retired")))
}

/// Returns the path to the `settings.json` file.
pub fn settings_file() -> &'static PathBuf {
    static SETTINGS_FILE: OnceLock<PathBuf> = OnceLock::new();
    SETTINGS_FILE.get_or_init(|| config_dir().join("settings.json"))
}

/// Returns the path to the global settings file.
pub fn global_settings_file() -> &'static PathBuf {
    static GLOBAL_SETTINGS_FILE: OnceLock<PathBuf> = OnceLock::new();
    GLOBAL_SETTINGS_FILE.get_or_init(|| config_dir().join("global_settings.json"))
}
