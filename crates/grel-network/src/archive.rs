//! Archive extraction utilities for tar.gz, zip, and plain binaries.

use std::path::{Path, PathBuf};

use sanitize_filename::sanitize;

use crate::NetworkError;

/// Result of an extraction operation
#[derive(Debug)]
pub struct ExtractionResult {
    /// Binaries found after extraction
    pub binaries: Vec<PathBuf>,
    /// Whether the archive was a plain binary (not an archive)
    pub is_plain_binary: bool,
}

/// Extract an archive to the target directory
pub fn extract_archive(
    archive_path: &Path,
    target_dir: &Path,
    filename: &str,
) -> Result<ExtractionResult, NetworkError> {
    // Create target directory
    std::fs::create_dir_all(target_dir).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to create target directory: {e}"))
    })?;

    // Determine archive type by extension
    if filename.ends_with(".zip") {
        extract_zip(archive_path, target_dir)
    } else if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
        extract_tar_gz(archive_path, target_dir)
    } else if filename.ends_with(".tar.xz") {
        extract_tar_xz(archive_path, target_dir)
    } else if filename.ends_with(".exe") || filename.ends_with(".bin") {
        // Plain binary - requires async for copy on Windows with tokio
        // Use sync version for simplicity
        std::fs::copy(archive_path, target_dir.join(filename)).map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to copy binary: {e}"))
        })?;
        Ok(ExtractionResult {
            binaries: vec![target_dir.join(filename)],
            is_plain_binary: true,
        })
    } else {
        // Unsupported format - treat as plain file
        std::fs::copy(archive_path, target_dir.join(filename)).map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to copy file: {e}"))
        })?;
        Ok(ExtractionResult {
            binaries: vec![target_dir.join(filename)],
            is_plain_binary: true,
        })
    }
}

/// Extract a zip archive
fn extract_zip(
    archive_path: &Path,
    target_dir: &Path,
) -> Result<ExtractionResult, NetworkError> {
    let archive_file = std::fs::File::open(archive_path).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to open archive: {e}"))
    })?;

    let mut archive = zip::ZipArchive::new(archive_file).map_err(|e| {
        NetworkError::OperationFailed(format!("Invalid zip archive: {e}"))
    })?;

    let mut binaries = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to read archive entry: {e}"))
        })?;

        let raw_name = file.name().to_string();
        let sanitized = sanitize(&raw_name);

        if sanitized.is_empty() || sanitized.contains("..") {
            continue; // Skip suspicious paths
        }

        let out_path = target_dir.join(&sanitized);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| {
                NetworkError::OperationFailed(format!("Failed to create directory: {e}"))
            })?;
        } else {
            // Ensure parent directory exists
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

            // Check if it's a binary
            if is_likely_binary(&sanitized) {
                binaries.push(out_path);
            }
        }
    }

    Ok(ExtractionResult {
        binaries,
        is_plain_binary: false,
    })
}

/// Extract a tar.gz archive
fn extract_tar_gz(
    archive_path: &Path,
    target_dir: &Path,
) -> Result<ExtractionResult, NetworkError> {
    let tar_gz_file = std::fs::File::open(archive_path).map_err(|e| {
        NetworkError::OperationFailed(format!("Failed to open archive: {e}"))
    })?;

    let decoder = flate2::read::GzDecoder::new(tar_gz_file);
    let mut archive = tar::Archive::new(decoder);

    let mut binaries = Vec::new();

    // SAFETY: We validate all paths before extracting
    let entries = archive.entries().map_err(|e| {
        NetworkError::OperationFailed(format!("Invalid tar archive: {e}"))
    })?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to read tar entry: {e}"))
        })?;

        let path = entry.path().map_err(|e| {
            NetworkError::OperationFailed(format!("Invalid path in archive: {e}"))
        })?.to_path_buf();

        // Sanitize and validate path
        let safe_path = sanitize_tar_path(&path, target_dir)?;
        let full_path = target_dir.join(&safe_path);

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

            let filename = safe_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            if is_likely_binary(&filename) {
                binaries.push(full_path);
            }
        }
    }

    Ok(ExtractionResult {
        binaries,
        is_plain_binary: false,
    })
}

/// Extract a tar.xz archive
fn extract_tar_xz(
    _archive_path: &Path,
    _target_dir: &Path,
) -> Result<ExtractionResult, NetworkError> {
    // TODO: Implement with xz2 crate
    Err(NetworkError::OperationFailed(
        "tar.xz extraction not yet implemented".into(),
    ))
}

/// Check if a filename is likely a binary executable
fn is_likely_binary(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    // Executable extensions
    if lower.ends_with(".exe")
        || lower.ends_with(".bat")
        || lower.ends_with(".cmd")
        || lower.ends_with(".ps1")
        || lower.ends_with(".bin")
        || lower.ends_with(".appimage")
    {
        return true;
    }

    // Heuristic: if no extension and not a known text type, likely a binary
    if !lower.contains('.') {
        let text_extensions = ["md", "txt", "rst", "json", "yaml", "yml", "toml", "cfg", "conf", "sh", "bash"];
        return !text_extensions.iter().any(|ext| lower.ends_with(ext));
    }

    false
}

/// Sanitize a tar archive entry path to prevent path traversal attacks
fn sanitize_tar_path(
    entry_path: &Path,
    _target_dir: &Path,
) -> Result<PathBuf, NetworkError> {
    // Reject absolute paths
    if entry_path.is_absolute() || entry_path.starts_with("/") {
        return Err(NetworkError::OperationFailed(
            "Absolute path in archive".into(),
        ));
    }

    // Reject paths with ..
    for component in entry_path.components() {
        if let std::path::Component::ParentDir = component {
            return Err(NetworkError::OperationFailed(
                "Path traversal detected in archive".into(),
            ));
        }
    }

    // Sanitize the filename
    if let Some(file_name) = entry_path.file_name() {
        let sanitized = sanitize(&file_name.to_string_lossy());
        if let Some(parent) = entry_path.parent() {
            Ok(parent.join(sanitized))
        } else {
            Ok(PathBuf::from(sanitized))
        }
    } else {
        Ok(entry_path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_likely_binary() {
        assert!(is_likely_binary("tool.exe"));
        assert!(is_likely_binary("script.bat"));
        assert!(is_likely_binary("tool")); // No extension, likely binary on Unix
        assert!(!is_likely_binary("readme.md"));
        assert!(!is_likely_binary("config.json"));
    }

    #[test]
    fn test_path_traversal_rejected() {
        let target = PathBuf::from("/tmp/test");
        assert!(sanitize_tar_path(Path::new("../evil.txt"), &target).is_err());
        assert!(sanitize_tar_path(Path::new("/etc/passwd"), &target).is_err());
        assert!(sanitize_tar_path(Path::new("safe/file.txt"), &target).is_ok());
    }
}
