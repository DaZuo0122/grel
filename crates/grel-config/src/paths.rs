//! Platform-aware path resolution.
//!
//! Uses platform-specific base directories:
//!
//! | Platform | Data dir          | Config dir         |
//! |----------|-------------------|--------------------|
//! | Linux    | `~/.local/share/grel` | `~/.config/grel` |
//! | Windows  | `%LOCALAPPDATA%\grel` | `%APPDATA%\grel` |
//! | macOS    | `~/Library/Application Support/grel` | same |

use std::path::PathBuf;

/// Project data root (packages, database, cache).
///
/// Uses `directories::ProjectDirs` with empty qualifier so paths
/// resolve to `<base>/grel` rather than `<base>/rs/grel/grel`.
pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "grel")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(fallback_data_dir)
}

/// Project config root.
pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "grel")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(fallback_config_dir)
}

/// System user download directory.
pub fn download_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir().join("grel-downloads"))
}

/// Resolve a path string, expanding `~` to the home directory.
pub fn resolve_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            if stripped.is_empty() {
                return home;
            }
            let stripped = stripped
                .strip_prefix(std::path::MAIN_SEPARATOR_STR)
                .unwrap_or(stripped);
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}

/// Default install root: `<data_dir>/installs`
pub fn default_install_root() -> PathBuf {
    data_dir().join("installs")
}

/// Default binary directory: `<data_dir>/bin`
pub fn default_bin_dir() -> PathBuf {
    data_dir().join("bin")
}

fn fallback_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join(".local/share/grel")
}

fn fallback_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join(".config/grel")
}

/// Default config file path: `<config_dir>/config.toml`
pub fn default_config_file() -> PathBuf {
    config_dir().join("config.toml")
}
