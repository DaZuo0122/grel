//! System dependency health checks.
//!
//! Checks whether required system libraries (e.g., `libssl.so.3`)
//! are available on the host. These checks are **advisory only** —
//! grel cannot install system packages.

use std::collections::HashSet;

/// Report from a system dependency check.
#[derive(Debug, Clone)]
pub struct SystemDepReport {
    /// Libraries that were found on the system.
    pub found: Vec<String>,
    /// Libraries that were NOT found on the system.
    pub missing: Vec<String>,
}

impl SystemDepReport {
    /// Returns true if all requested libraries were found.
    pub fn all_found(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Check whether the given system libraries are available.
///
/// On Linux, this runs `ldconfig -p` and parses the output.
/// On Windows, it checks well-known system paths for DLLs.
/// On other platforms, it returns an empty report (assumes success).
pub fn check_system_deps(libraries: &[String]) -> SystemDepReport {
    let mut found = Vec::new();
    let mut missing = Vec::new();

    let available = list_available_libs();

    for lib in libraries {
        if available.contains(lib) {
            found.push(lib.clone());
        } else {
            missing.push(lib.clone());
        }
    }

    SystemDepReport { found, missing }
}

/// List libraries available on the system.
fn list_available_libs() -> HashSet<String> {
    let mut libs = HashSet::new();

    if cfg!(target_os = "linux") {
        // Try `ldconfig -p` first
        if let Ok(output) = std::process::Command::new("ldconfig").arg("-p").output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    // ldconfig -p output looks like:
                    // \tlibfoo.so.1 (libc6,x86-64) => /lib/x86_64-linux-gnu/libfoo.so.1
                    if let Some(lib_name) = line.trim().split_whitespace().next() {
                        libs.insert(lib_name.to_string());
                    }
                }
            }
        }

        // Fallback: scan /lib, /usr/lib, /usr/local/lib
        for dir in &["/lib", "/usr/lib", "/usr/local/lib"] {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_file() || meta.is_symlink() {
                            if let Some(name) = entry.file_name().to_str() {
                                if name.ends_with(".so") || name.contains(".so.") {
                                    libs.insert(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if cfg!(target_os = "windows") {
        // Check System32 for common DLLs
        if let Ok(windir) = std::env::var("SystemRoot") {
            let system32 = std::path::Path::new(&windir).join("System32");
            if let Ok(entries) = std::fs::read_dir(&system32) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.ends_with(".dll") {
                            libs.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }

    // macOS / BSD: dylibs
    if cfg!(target_os = "macos") || cfg!(target_os = "freebsd") {
        for dir in &["/usr/lib", "/usr/local/lib"] {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.ends_with(".dylib") || name.contains(".dylib.") {
                            libs.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }

    libs
}

/// Format a user-friendly message suggesting how to install missing libs.
pub fn format_missing_lib_advice(lib: &str) -> String {
    if cfg!(target_os = "linux") {
        // Try to guess the package name
        let pkg = if lib.starts_with("libssl") {
            "libssl3 | libssl1.1".into()
        } else if lib.starts_with("libcrypto") {
            "libssl3 | libssl1.1".into()
        } else if lib.starts_with("libcurl") {
            "libcurl4".into()
        } else if lib.starts_with("libz") {
            "zlib1g".into()
        } else if lib.starts_with("libsqlite") {
            "libsqlite3-0".into()
        } else {
            format!("<{lib}>")
        };
        format!("Install with: sudo apt install {pkg}  (or equivalent for your distro)")
    } else if cfg!(target_os = "macos") {
        format!("Install with: brew install {lib}")
    } else {
        format!("Ensure {lib} is installed on your system")
    }
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_system_deps_empty() {
        let report = check_system_deps(&[]);
        assert!(report.all_found());
    }

    #[test]
    fn test_check_system_deps_fictional() {
        let report = check_system_deps(&["libdefinitely_not_real.so.99".into()]);
        assert!(!report.all_found());
        assert_eq!(report.missing.len(), 1);
    }
}
