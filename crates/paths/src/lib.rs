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

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

static HOME_DIR: OnceLock<PathBuf> = OnceLock::new();

/// A custom data directory override, set only by `set_custom_data_dir`.
/// This is used to override the default data directory location.
/// The directory will be created if it doesn't exist when set.
static CUSTOM_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// A custom config directory override, set only by `set_custom_config_dir`.
/// This is used to override the default config directory location.
/// The directory will be created if it doesn't exist when set.
static CUSTOM_CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The resolved data directory, combining custom override or platform defaults.
/// This is set once and cached for subsequent calls.
/// On macOS, this is `~/Library/Application Support/rara`.
/// On Linux/FreeBSD, this is `$XDG_DATA_HOME/rara`.
static CURRENT_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The resolved config directory, combining custom override or platform
/// defaults. This is set once and cached for subsequent calls.
/// On macOS, this is `~/.config/rara`.
/// On Linux/FreeBSD, this is `$XDG_CONFIG_HOME/rara`.
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Returns the path to the user's home directory.
///
/// # Panics
///
/// Panics if the current user's home directory cannot be determined.
pub fn home_dir() -> &'static PathBuf {
    HOME_DIR.get_or_init(|| dirs::home_dir().expect("failed to determine home directory"))
}

/// Returns the path to the configuration directory used by rara.
///
/// # Panics
///
/// Panics if the platform configuration directory cannot be determined.
pub fn config_dir() -> &'static PathBuf {
    CONFIG_DIR.get_or_init(|| {
        if let Some(custom) = CUSTOM_CONFIG_DIR.get() {
            return custom.clone();
        }
        CUSTOM_DATA_DIR.get().map_or_else(
            || {
                if cfg!(target_os = "windows") {
                    dirs::config_dir()
                        .expect("failed to determine RoamingAppData directory")
                        .join("rara")
                } else if cfg!(any(target_os = "linux", target_os = "freebsd")) {
                    std::env::var("FLATPAK_XDG_CONFIG_HOME")
                        .map_or_else(
                            |_| {
                                dirs::config_dir()
                                    .expect("failed to determine XDG_CONFIG_HOME directory")
                            },
                            PathBuf::from,
                        )
                        .join("rara")
                } else {
                    home_dir().join(".config").join("rara")
                }
            },
            |custom_dir| custom_dir.join("config"),
        )
    })
}

/// Returns the path to the data directory used by rara.
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
                        .join("rara")
                } else {
                    dirs::data_local_dir()
                        .expect("failed to determine LocalAppData directory")
                        .join("rara")
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

/// Sets a custom directory for configuration, overriding the default config
/// directory.
///
/// This function must be called before any other path operations that depend on
/// the config directory. The directory's path will be canonicalized to an
/// absolute path by a blocking FS operation. The directory will be created if
/// it doesn't exist.
///
/// # Arguments
///
/// * `dir` - The path to use as the custom config directory. This will be used
///   as the base directory for all configuration files, including settings,
///   prompts, and skills.
///
/// # Returns
///
/// A reference to the static `PathBuf` containing the custom config directory
/// path.
///
/// # Panics
///
/// Panics if:
/// * Called after the config directory has been initialized (e.g., via
///   `config_dir`)
/// * The directory's path cannot be canonicalized to an absolute path
/// * The directory cannot be created
pub fn set_custom_config_dir<P: ?Sized + AsRef<Path>>(dir: &P) -> &'static PathBuf {
    assert!(
        CONFIG_DIR.get().is_none(),
        "set_custom_config_dir called after config_dir was initialized"
    );
    CUSTOM_CONFIG_DIR.get_or_init(|| {
        let mut path = dir.as_ref().to_path_buf();
        if path.is_relative()
            && let Ok(abs) = path.canonicalize()
        {
            path = abs;
        }

        std::fs::create_dir_all(&path).unwrap_or_else(|e| {
            panic!(
                "failed to create custom config directory {}: {e}",
                path.display()
            )
        });

        path
    })
}

/// Returns the path to the temp directory used by rara.
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
                .join("rara");
        }

        if cfg!(target_os = "windows") {
            return dirs::cache_dir()
                .expect("failed to determine LocalAppData directory")
                .join("rara");
        }

        if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return std::env::var("FLATPAK_XDG_CACHE_HOME")
                .map_or_else(
                    |_| dirs::cache_dir().expect("failed to determine XDG_CACHE_HOME directory"),
                    PathBuf::from,
                )
                .join("rara");
        }

        home_dir().join(".cache").join("rara")
    })
}

/// Returns the path to the logs directory.
pub fn logs_dir() -> &'static PathBuf {
    static LOGS_DIR: OnceLock<PathBuf> = OnceLock::new();
    LOGS_DIR.get_or_init(|| {
        if cfg!(target_os = "macos") {
            home_dir().join("Library/Logs/rara")
        } else {
            data_dir().join("logs")
        }
    })
}

/// Returns the path to the `rara.log` file.
pub fn log_file() -> &'static PathBuf {
    static LOG_FILE: OnceLock<PathBuf> = OnceLock::new();
    LOG_FILE.get_or_init(|| logs_dir().join("rara.log"))
}

/// Returns the path to the database directory.
pub fn database_dir() -> &'static PathBuf {
    static DATABASE_DIR: OnceLock<PathBuf> = OnceLock::new();
    DATABASE_DIR.get_or_init(|| data_dir().join("db"))
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

/// Returns the path to the sessions directory used for JSONL message storage.
pub fn sessions_dir() -> &'static PathBuf {
    static SESSIONS_DIR: OnceLock<PathBuf> = OnceLock::new();
    SESSIONS_DIR.get_or_init(|| data_dir().join("sessions"))
}

/// Returns the directory containing editable markdown prompt files.
pub fn prompts_dir() -> &'static PathBuf {
    static PROMPTS_DIR: OnceLock<PathBuf> = OnceLock::new();
    PROMPTS_DIR.get_or_init(|| config_dir().join("prompts"))
}

/// Returns the path to the memory documents directory.
pub fn memory_dir() -> &'static PathBuf {
    static MEMORY_DIR: OnceLock<PathBuf> = OnceLock::new();
    MEMORY_DIR.get_or_init(|| data_dir().join("memory"))
}

/// Returns the path to the memory sessions subdirectory.
pub fn memory_sessions_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| memory_dir().join("sessions"))
}

/// Returns the path to the agent scheduled raras JSON file.
pub fn agent_raras_file() -> &'static PathBuf {
    static AGENT_RARA: OnceLock<PathBuf> = OnceLock::new();
    AGENT_RARA.get_or_init(|| data_dir().join("agent_rara.json"))
}

/// Returns the path to the user skills directory.
pub fn skills_dir() -> &'static PathBuf {
    static SKILLS_DIR: OnceLock<PathBuf> = OnceLock::new();
    SKILLS_DIR.get_or_init(|| config_dir().join("skills"))
}

/// Returns the path to the skill drafts directory.
///
/// Mita writes draft skill files here; Rara reads and archives them.
/// Location: `<data_dir>/skill-drafts/`
pub fn skill_drafts_dir() -> &'static PathBuf {
    static SKILL_DRAFTS_DIR: OnceLock<PathBuf> = OnceLock::new();
    SKILL_DRAFTS_DIR.get_or_init(|| data_dir().join("skill-drafts"))
}

/// Returns the path to the archived skill drafts directory.
///
/// Successfully created skills have their drafts moved here.
/// Location: `<data_dir>/skill-drafts/archived/`
pub fn skill_drafts_archived_dir() -> &'static PathBuf {
    static SKILL_DRAFTS_ARCHIVED_DIR: OnceLock<PathBuf> = OnceLock::new();
    SKILL_DRAFTS_ARCHIVED_DIR.get_or_init(|| skill_drafts_dir().join("archived"))
}

/// Returns the path to the resources directory for tool-produced artifacts.
pub fn resources_dir() -> &'static PathBuf {
    static RESOURCES_DIR: OnceLock<PathBuf> = OnceLock::new();
    RESOURCES_DIR.get_or_init(|| data_dir().join("resources"))
}

/// Returns the path to the images directory for avatar and media assets.
pub fn images_dir() -> &'static PathBuf {
    static IMAGES_DIR: OnceLock<PathBuf> = OnceLock::new();
    IMAGES_DIR.get_or_init(|| resources_dir().join("images"))
}

/// Returns the path to the staging directory used for gateway updates.
///
/// Resolves to `<data_dir>/staging/`. The directory is created if it
/// does not already exist.
///
/// # Panics
///
/// Panics if the directory cannot be created.
pub fn staging_dir() -> &'static PathBuf {
    static STAGING_DIR: OnceLock<PathBuf> = OnceLock::new();
    STAGING_DIR.get_or_init(|| {
        let dir = data_dir().join("staging");
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| {
            panic!("failed to create staging directory {}: {e}", dir.display())
        });
        dir
    })
}

/// Returns the path to the workspace directory used by the agent.
///
/// Resolves to `<config_dir>/workspace/`. The directory is created if it
/// does not already exist.
///
/// # Panics
///
/// Panics if the directory cannot be created.
pub fn workspace_dir() -> &'static PathBuf {
    static WORKSPACE_DIR: OnceLock<PathBuf> = OnceLock::new();
    WORKSPACE_DIR.get_or_init(|| {
        let dir = config_dir().join("workspace");
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| {
            panic!(
                "failed to create workspace directory {}: {e}",
                dir.display()
            )
        });
        dir
    })
}

/// Returns the path to the main YAML configuration file.
///
/// Resolves to `<config_dir>/config.yaml`.
pub fn config_file() -> &'static PathBuf {
    static CONFIG_FILE: OnceLock<PathBuf> = OnceLock::new();
    CONFIG_FILE.get_or_init(|| config_dir().join("config.yaml"))
}
