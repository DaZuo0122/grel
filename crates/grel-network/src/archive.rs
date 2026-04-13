//! Archive extraction and installation utilities.
//!
//! ## File layout for managed packages
//! ```text
//! <install_root>/<forge>/<owner>/<repo>/
//! ├── <archive>           # downloaded archive (kept for checksum verification)
//! └── <extracted tree>/   # contents of the archive
//!
//! <bin_dir>/
//! ├── <binary1>           # linked or copied from extracted tree
//! └── <binary2>
//! ```
//!
//! ## Removal
//! - Delete `<install_root>/<forge>/<owner>/<repo>/` recursively
//! - Delete each linked binary from `<bin_dir>/`

use std::path::{Path, PathBuf};

use sanitize_filename::sanitize;

use crate::NetworkError;

/// Result of an extraction + install operation
#[derive(Debug)]
pub struct InstallResult {
    /// The package-specific install directory (contains archive + extracted files)
    pub install_dir: PathBuf,
    /// Paths of symlinks that were created in `bin_dir`
    pub installed_binaries: Vec<PathBuf>,
    /// Whether the asset was a plain binary (not an archive)
    pub is_plain_binary: bool,
}

/// Install an asset archive into the package-managed directory.
///
/// 1. Creates `<install_dir>` if needed
/// 2. Downloads/copies the archive there
/// 3. Extracts into `<install_dir>/extracted/`
/// 4. Detects binaries and copies/link to `bin_dir`
/// 5. For plain binaries: just chmod +x on Unix
pub fn install_asset(
    archive_path: &Path,
    install_dir: &Path,
    bin_dir: &Path,
    asset_filename: &str,
) -> Result<InstallResult, NetworkError> {
    std::fs::create_dir_all(install_dir).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to create install dir: {e}"))
    })?;

    let ext = extract_extension(asset_filename);

    match ext {
        ArchiveType::Zip => {
            extract_zip(archive_path, install_dir)?;
            link_binaries(install_dir, bin_dir, asset_filename)
        }
        ArchiveType::TarGz | ArchiveType::Tgz => {
            extract_tar_gz(archive_path, install_dir)?;
            link_binaries(install_dir, bin_dir, asset_filename)
        }
        ArchiveType::TarXz => {
            extract_tar_xz(archive_path, install_dir)
        }
        ArchiveType::Plain => install_plain_binary(archive_path, install_dir, bin_dir, asset_filename),
    }
}

/// Determine the archive type from filename
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveType {
    Zip,
    TarGz,
    Tgz,
    TarXz,
    Plain,
}

fn extract_extension(filename: &str) -> ArchiveType {
    let lower = filename.to_lowercase();
    if lower.ends_with(".zip") {
        ArchiveType::Zip
    } else if lower.ends_with(".tar.gz") {
        ArchiveType::TarGz
    } else if lower.ends_with(".tgz") {
        ArchiveType::Tgz
    } else if lower.ends_with(".tar.xz") {
        ArchiveType::TarXz
    } else {
        ArchiveType::Plain
    }
}

/// Extract a zip archive into `<install_dir>/extracted/`
fn extract_zip(archive_path: &Path, install_dir: &Path) -> Result<(), NetworkError> {
    let out_dir = install_dir.join("extracted");
    std::fs::create_dir_all(&out_dir).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to create extract dir: {e}"))
    })?;

    let archive_file = std::fs::File::open(archive_path).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to open archive: {e}"))
    })?;

    let mut archive = zip::ZipArchive::new(archive_file).map_err(|e| {
        NetworkError::OperationFailed(format!("Invalid zip archive: {e}"))
    })?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to read archive entry: {e}"))
        })?;

        let raw_name = file.name().to_string();
        let sanitized = sanitize_path_components(&raw_name)?;

        if sanitized.is_empty() || sanitized.contains("..") {
            continue;
        }

        let out_path = out_dir.join(&sanitized);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| {
                NetworkError::OperationFailed(format!("Failed to create directory: {e}"))
            })?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    NetworkError::OperationFailed(format!("Failed to create directory: {e}"))
                })?;
            }

            let mut out_file = std::fs::File::create(&out_path).map_err(|e| {
                NetworkError::OperationFailed(format!("Failed to create file: {e}"))
            })?;

            std::io::copy(&mut file, &mut out_file).map_err(|e| {
                NetworkError::OperationFailed(format!("Failed to extract file: {e}"))
            })?;
        }
    }

    Ok(())
}

/// Extract a tar.gz archive into `<install_dir>/extracted/`
fn extract_tar_gz(archive_path: &Path, install_dir: &Path) -> Result<(), NetworkError> {
    let out_dir = install_dir.join("extracted");
    std::fs::create_dir_all(&out_dir).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to create extract dir: {e}"))
    })?;

    let tar_gz_file = std::fs::File::open(archive_path).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to open archive: {e}"))
    })?;

    let decoder = flate2::read::GzDecoder::new(tar_gz_file);
    let mut archive = tar::Archive::new(decoder);

    let entries = archive.entries().map_err(|e| {
        NetworkError::OperationFailed(format!("Invalid tar archive: {e}"))
    })?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to read tar entry: {e}"))
        })?;

        let path = entry.path().map_err(|e| {
            NetworkError::OperationFailed(format!("Invalid path in archive: {e}"))
        })?
        .to_path_buf();

        let safe_path = sanitize_tar_path(&path)?;
        if safe_path.is_empty() || safe_path.contains("..") {
            continue;
        }

        let full_path = out_dir.join(&safe_path);

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&full_path).map_err(|e| {
                NetworkError::OperationFailed(format!("Failed to create directory: {e}"))
            })?;
        } else {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    NetworkError::OperationFailed(format!("Failed to create directory: {e}"))
                })?;
            }

            entry.unpack(&full_path).map_err(|e| {
                NetworkError::OperationFailed(format!("Failed to extract file: {e}"))
            })?;
        }
    }

    Ok(())
}

/// Extract tar.xz (stub)
fn extract_tar_xz(_archive_path: &Path, _install_dir: &Path) -> Result<InstallResult, NetworkError> {
    Err(NetworkError::OperationFailed(
        "tar.xz extraction not yet implemented".into(),
    ))
}

/// Install a plain binary (not an archive)
fn install_plain_binary(
    source_path: &Path,
    install_dir: &Path,
    bin_dir: &Path,
    filename: &str,
) -> Result<InstallResult, NetworkError> {
    // Copy archive to install dir
    let dest = install_dir.join(filename);
    std::fs::copy(source_path, &dest).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to copy binary: {e}"))
    })?;

    // Symlink from bin_dir to the actual binary (so it can find sibling DLLs)
    let link_path = bin_dir.join(filename);
    create_binary_link(&dest, &link_path)?;

    // Make target executable on Unix (for the actual binary in install_dir)
    make_executable(&dest)?;

    Ok(InstallResult {
        install_dir: install_dir.to_path_buf(),
        installed_binaries: vec![link_path],
        is_plain_binary: true,
    })
}

/// Detect binaries in extracted tree and symlink them to bin_dir
fn link_binaries(
    install_dir: &Path,
    bin_dir: &Path,
    _archive_name: &str,
) -> Result<InstallResult, NetworkError> {
    let extracted = install_dir.join("extracted");
    let mut installed = Vec::new();

    if extracted.exists() {
        collect_binaries(&extracted, bin_dir, &mut installed)?;
    }

    // If no binaries found in extracted/, check if the archive contained
    // a single top-level binary (common for Go releases)
    if installed.is_empty() {
        for entry in std::fs::read_dir(install_dir).map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to read install dir: {e}"))
        })? {
            let entry = entry.map_err(|e| {
                NetworkError::OperationFailed(format!("Failed to read dir entry: {e}"))
            })?;
            let path = entry.path();
            if path.is_file() && is_executable_name(&path) {
                let dest = bin_dir.join(path.file_name().unwrap());
                create_binary_link(&path, &dest)?;
                installed.push(dest);
            }
        }
    }

    Ok(InstallResult {
        install_dir: install_dir.to_path_buf(),
        installed_binaries: installed,
        is_plain_binary: false,
    })
}

/// Recursively find executable files in a directory tree and symlink to bin_dir
fn collect_binaries(
    dir: &Path,
    bin_dir: &Path,
    installed: &mut Vec<PathBuf>,
) -> Result<(), NetworkError> {
    for entry in std::fs::read_dir(dir).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to read directory: {e}"))
    })? {
        let entry = entry.map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to read dir entry: {e}"))
        })?;
        let path = entry.path();

        if path.is_dir() {
            collect_binaries(&path, bin_dir, installed)?;
        } else if path.is_file() && is_executable_name(&path) {
            let dest = bin_dir.join(path.file_name().unwrap());
            create_binary_link(&path, &dest)?;
            installed.push(dest);
        }
    }
    Ok(())
}

/// Create a symlink from `dest` → `target`, falling back to a copy if
/// symlink creation fails (e.g. unprivileged Windows without dev mode).
///
/// This is preferred over copying because some executables depend on
/// sibling DLLs/SOs that remain in the extracted tree.
fn create_binary_link(target: &Path, dest: &Path) -> Result<(), NetworkError> {
    // Try symlink first
    if symlink_binary(target, dest).is_ok() {
        return Ok(());
    }

    // Fall back to hard link (faster, same volume required)
    if std::fs::hard_link(target, dest).is_ok() {
        return Ok(());
    }

    // Final fallback: copy
    std::fs::copy(target, dest).map_err(|e| {
        NetworkError::OperationFailed(format!(
            "Failed to install binary '{}' (symlink/hardlink/copy all failed): {e}",
            target.display()
        ))
    })?;

    Ok(())
}

/// Platform-specific symlink creation
fn symlink_binary(target: &Path, dest: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, dest)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, dest)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, dest);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Symlinks not supported on this platform",
        ))
    }
}

/// Check if a filename looks like an executable
fn is_executable_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();

    // Known executable extensions
    if lower.ends_with(".exe")
        || lower.ends_with(".bat")
        || lower.ends_with(".cmd")
        || lower.ends_with(".ps1")
        || lower.ends_with(".com")
        || lower.ends_with(".bin")
    {
        return true;
    }

    // No extension: likely a Unix binary
    if !name.contains('.') {
        let skip = ["readme", "license", "changelog", "changes", "copying"];
        return !skip.iter().any(|&s| lower.starts_with(s));
    }

    false
}

/// Add execute permission on Unix
fn make_executable(path: &Path) -> Result<(), NetworkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path).map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to read metadata: {e}"))
        })?;
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(path, perms).map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to set permissions: {e}"))
        })?;
    }
    let _ = path; // suppress unused warning on Windows
    Ok(())
}

/// Sanitize a path, rejecting dangerous patterns
fn sanitize_path_components(path: &str) -> Result<String, NetworkError> {
    let path = Path::new(path);
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Normal(name) => {
                let sanitized = sanitize(&name.to_string_lossy());
                if sanitized.is_empty() {
                    return Err(NetworkError::OperationFailed(
                        "Empty path component after sanitization".into(),
                    ));
                }
                result.push(sanitized);
            }
            std::path::Component::ParentDir => {
                return Err(NetworkError::OperationFailed(
                    "Path traversal detected in archive".into(),
                ));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(NetworkError::OperationFailed(
                    "Absolute path in archive".into(),
                ));
            }
            std::path::Component::CurDir => {} // skip "."
        }
    }

    Ok(result.to_string_lossy().to_string())
}

/// Sanitize a tar archive entry path
fn sanitize_tar_path(entry_path: &Path) -> Result<String, NetworkError> {
    let mut result = PathBuf::new();

    for component in entry_path.components() {
        match component {
            std::path::Component::Normal(name) => {
                let sanitized = sanitize(&name.to_string_lossy());
                if !sanitized.is_empty() {
                    result.push(sanitized);
                }
            }
            std::path::Component::ParentDir => {
                return Err(NetworkError::OperationFailed(
                    "Path traversal detected in archive".into(),
                ));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(NetworkError::OperationFailed(
                    "Absolute path in archive".into(),
                ));
            }
            std::path::Component::CurDir => {} // skip "."
        }
    }

    Ok(result.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_executable_name() {
        assert!(is_executable_name(Path::new("tool.exe")));
        assert!(is_executable_name(Path::new("script.bat")));
        assert!(is_executable_name(Path::new("rg")));
        assert!(!is_executable_name(Path::new("readme.md")));
        assert!(!is_executable_name(Path::new("config.json")));
    }

    #[test]
    fn test_path_traversal_rejected() {
        assert!(sanitize_path_components("../evil.txt").is_err());
        assert!(sanitize_path_components("/etc/passwd").is_err());
        assert!(sanitize_path_components("safe/file.txt").is_ok());
    }
}
