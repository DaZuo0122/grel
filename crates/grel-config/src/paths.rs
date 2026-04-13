//! Path resolution utilities.

use std::path::PathBuf;

/// Resolve a path string, expanding `~` to the home directory
pub fn resolve_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            if stripped.is_empty() {
                return home;
            }
            let stripped = stripped.strip_prefix(std::path::MAIN_SEPARATOR_STR).unwrap_or(stripped);
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}
