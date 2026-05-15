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
    std::fs::create_dir_all(install_dir)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to create install dir: {e}")))?;

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
            extract_tar_xz(archive_path, install_dir)?;
            link_binaries(install_dir, bin_dir, asset_filename)
        }
        ArchiveType::Plain => {
            install_plain_binary(archive_path, install_dir, bin_dir, asset_filename)
        }
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
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to create extract dir: {e}")))?;

    let archive_file = std::fs::File::open(archive_path)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to open archive: {e}")))?;

    let mut archive = zip::ZipArchive::new(archive_file)
        .map_err(|e| NetworkError::OperationFailed(format!("Invalid zip archive: {e}")))?;

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
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to create extract dir: {e}")))?;

    let tar_gz_file = std::fs::File::open(archive_path)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to open archive: {e}")))?;

    let decoder = flate2::read::GzDecoder::new(tar_gz_file);
    let mut archive = tar::Archive::new(decoder);

    let entries = archive
        .entries()
        .map_err(|e| NetworkError::OperationFailed(format!("Invalid tar archive: {e}")))?;

    for entry_result in entries {
        let mut entry = entry_result
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to read tar entry: {e}")))?;

        let path = entry
            .path()
            .map_err(|e| NetworkError::OperationFailed(format!("Invalid path in archive: {e}")))?
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

/// Extract a tar.xz archive into `<install_dir>/extracted/`
fn extract_tar_xz(archive_path: &Path, install_dir: &Path) -> Result<(), NetworkError> {
    let out_dir = install_dir.join("extracted");
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to create extract dir: {e}")))?;

    let tar_xz_file = std::fs::File::open(archive_path)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to open archive: {e}")))?;

    let decoder = xz2::read::XzDecoder::new(tar_xz_file);
    let mut archive = tar::Archive::new(decoder);

    let entries = archive
        .entries()
        .map_err(|e| NetworkError::OperationFailed(format!("Invalid tar archive: {e}")))?;

    for entry_result in entries {
        let mut entry = entry_result
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to read tar entry: {e}")))?;

        let path = entry
            .path()
            .map_err(|e| NetworkError::OperationFailed(format!("Invalid path in archive: {e}")))?
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

/// Install a plain binary (not an archive)
fn install_plain_binary(
    source_path: &Path,
    install_dir: &Path,
    bin_dir: &Path,
    filename: &str,
) -> Result<InstallResult, NetworkError> {
    // Ensure both directories exist before writing into them.
    std::fs::create_dir_all(install_dir)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to create install dir: {e}")))?;
    std::fs::create_dir_all(bin_dir)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to create bin dir: {e}")))?;

    let dest = install_dir.join(filename);
    std::fs::copy(source_path, &dest)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to copy binary: {e}")))?;

    // Make target executable on Unix before linking, so the link inherits the bit.
    make_executable(&dest)?;

    // Symlink/hardlink/copy from bin_dir → install_dir so that sibling DLLs
    // (Windows) or SOs can still be found relative to the real location.
    let link_path = bin_dir.join(filename);
    create_binary_link(&dest, &link_path)?;

    Ok(InstallResult {
        install_dir: install_dir.to_path_buf(),
        installed_binaries: vec![link_path],
        is_plain_binary: true,
    })
}

/// Detect binaries in extracted tree and symlink them to bin_dir.
fn link_binaries(
    install_dir: &Path,
    bin_dir: &Path,
    _archive_name: &str,
) -> Result<InstallResult, NetworkError> {
    // Ensure bin_dir exists before we attempt to create any links inside it.
    std::fs::create_dir_all(bin_dir)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to create bin dir: {e}")))?;

    let extracted = install_dir.join("extracted");
    let mut installed = Vec::new();

    if extracted.exists() {
        collect_binaries(&extracted, bin_dir, &mut installed)?;
    }

    // If no binaries found in extracted/, check if the archive contained
    // a single top-level binary (common for Go releases).
    if installed.is_empty() {
        for entry in std::fs::read_dir(install_dir).map_err(|e| {
            NetworkError::OperationFailed(format!("Failed to read install dir: {e}"))
        })? {
            let entry = entry.map_err(|e| {
                NetworkError::OperationFailed(format!("Failed to read dir entry: {e}"))
            })?;
            let path = entry.path();
            if path.is_file() && is_executable(&path) {
                let Some(name) = path.file_name() else {
                    continue;
                };
                let dest = bin_dir.join(name);
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

/// Recursively find executable files in a directory tree and symlink to bin_dir.
fn collect_binaries(
    dir: &Path,
    bin_dir: &Path,
    installed: &mut Vec<PathBuf>,
) -> Result<(), NetworkError> {
    for entry in std::fs::read_dir(dir)
        .map_err(|e| NetworkError::OperationFailed(format!("Failed to read directory: {e}")))?
    {
        let entry = entry
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to read dir entry: {e}")))?;
        let path = entry.path();

        if path.is_dir() {
            collect_binaries(&path, bin_dir, installed)?;
        } else if path.is_file() && is_executable(&path) {
            let Some(name) = path.file_name() else {
                continue;
            };
            let dest = bin_dir.join(name);
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

/// Returns `true` if `path` should be treated as an executable binary.
///
/// **Unix** – primary check: the file has at least one execute permission
/// bit set (as preserved by `tar`/`zip` extractors).  This reliably
/// distinguishes `rg` (executable) from `UNLICENSE`, shell-completion
/// scripts, man pages, etc.  The name heuristic is used only when
/// `fs::metadata` fails.
///
/// **Windows** – there is no execute-bit concept, so we rely entirely on
/// file-extension heuristics (`.exe`, `.bat`, `.cmd`, `.ps1`, `.com`).
/// A file without a recognised extension is never treated as executable on
/// Windows (correct: plain binaries don't exist there).
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            return meta.permissions().mode() & 0o111 != 0;
        }
        // metadata unavailable: fall through to name heuristic
    }
    is_executable_name(path)
}

/// Name-based heuristic for executable detection.
///
/// Used on Windows (primary path) and as a Unix fallback when the
/// execute bit cannot be read.
fn is_executable_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();

    // Recognised executable extensions on all platforms.
    if lower.ends_with(".exe")
        || lower.ends_with(".bat")
        || lower.ends_with(".cmd")
        || lower.ends_with(".ps1")
        || lower.ends_with(".com")
        || lower.ends_with(".bin")
    {
        return true;
    }

    // On Windows a file without a recognised extension is not an executable.
    #[cfg(windows)]
    return false;

    // Unix fallback: files without any extension *may* be binaries, but
    // exclude well-known text / data filenames that appear extension-less.
    #[cfg(not(windows))]
    {
        if !lower.contains('.') {
            const SKIP: &[&str] = &[
                "readme",
                "license",
                "unlicense",
                "copying",
                "changelog",
                "changes",
                "notice",
                "authors",
                "contributors",
                "credits",
                "makefile",
                "dockerfile",
                "procfile",
            ];
            return !SKIP.iter().any(|&s| lower.starts_with(s));
        }
        false
    }
}

/// Add execute permission on Unix
fn make_executable(path: &Path) -> Result<(), NetworkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)
            .map_err(|e| NetworkError::OperationFailed(format!("Failed to read metadata: {e}")))?;
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

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    // ── is_executable_name (name heuristic, both platforms) ─────────────────

    #[test]
    fn test_exec_name_windows_extensions() {
        // .exe / .bat / .cmd / .ps1 / .com / .bin are always executable
        assert!(is_executable_name(Path::new("tool.exe")));
        assert!(is_executable_name(Path::new("script.bat")));
        assert!(is_executable_name(Path::new("run.cmd")));
        assert!(is_executable_name(Path::new("deploy.ps1")));
        assert!(is_executable_name(Path::new("prog.com")));
        assert!(is_executable_name(Path::new("helper.bin")));
    }

    #[test]
    fn test_exec_name_non_executables_with_extension() {
        // Files with non-executable extensions are never selected
        assert!(!is_executable_name(Path::new("readme.md")));
        assert!(!is_executable_name(Path::new("config.json")));
        assert!(!is_executable_name(Path::new("rg.1"))); // man page
        assert!(!is_executable_name(Path::new("data.csv")));
        assert!(!is_executable_name(Path::new("lib.so"))); // shared lib
        assert!(!is_executable_name(Path::new("arch.tar.gz"))); // nested ext
    }

    /// On Unix, extension-less well-known text files must be excluded.
    #[cfg(not(windows))]
    #[test]
    fn test_exec_name_unix_known_text_files_skipped() {
        assert!(!is_executable_name(Path::new("LICENSE")));
        assert!(!is_executable_name(Path::new("UNLICENSE")));
        assert!(!is_executable_name(Path::new("README")));
        assert!(!is_executable_name(Path::new("CHANGELOG")));
        assert!(!is_executable_name(Path::new("COPYING")));
        assert!(!is_executable_name(Path::new("NOTICE")));
        assert!(!is_executable_name(Path::new("AUTHORS")));
        assert!(!is_executable_name(Path::new("CONTRIBUTORS")));
        assert!(!is_executable_name(Path::new("Makefile")));
        assert!(!is_executable_name(Path::new("Dockerfile")));
        assert!(!is_executable_name(Path::new("Procfile")));
    }

    /// On Unix, extension-less names that are NOT in the skip list are
    /// considered potential binaries by the name heuristic.
    #[cfg(not(windows))]
    #[test]
    fn test_exec_name_unix_no_extension_is_binary() {
        assert!(is_executable_name(Path::new("rg")));
        assert!(is_executable_name(Path::new("ripgrep")));
        assert!(is_executable_name(Path::new("fd")));
        assert!(is_executable_name(Path::new("bat")));
    }

    /// On Windows, a file without a recognised extension is NOT executable.
    #[cfg(windows)]
    #[test]
    fn test_exec_name_windows_no_extension_not_executable() {
        assert!(!is_executable_name(Path::new("rg")));
        assert!(!is_executable_name(Path::new("UNLICENSE")));
        assert!(!is_executable_name(Path::new("LICENSE")));
    }

    // ── is_executable (permission-bit check on Unix) ─────────────────────────

    /// On Unix, verify that `is_executable` respects the execute bit and
    /// correctly rejects files that have no execute permission, regardless
    /// of whether their name looks like a binary.
    #[cfg(unix)]
    #[test]
    fn test_is_executable_unix_permission_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tmpdir");

        // Create a file that has the execute bit set → should be selected.
        let bin_path = dir.path().join("mytool");
        std::fs::write(&bin_path, b"#!/bin/sh\necho hi").unwrap();
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).unwrap();
        assert!(is_executable(&bin_path), "executable bit set → selected");

        // Create UNLICENSE with no execute bit → must NOT be selected.
        let license_path = dir.path().join("UNLICENSE");
        std::fs::write(&license_path, b"Public domain").unwrap();
        let mut perms = std::fs::metadata(&license_path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&license_path, perms).unwrap();
        assert!(
            !is_executable(&license_path),
            "UNLICENSE has no exec bit → skipped"
        );

        // A shell-completion script (_rg) without the execute bit → skipped.
        let comp_path = dir.path().join("_rg");
        std::fs::write(&comp_path, b"#compdef rg").unwrap();
        let mut perms = std::fs::metadata(&comp_path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&comp_path, perms).unwrap();
        assert!(
            !is_executable(&comp_path),
            "completion script (no exec bit) → skipped"
        );
    }

    // ── path-traversal rejection ─────────────────────────────────────────────

    #[test]
    fn test_path_traversal_rejected() {
        assert!(sanitize_path_components("../evil.txt").is_err());
        assert!(sanitize_path_components("/etc/passwd").is_err());
        assert!(sanitize_path_components("safe/file.txt").is_ok());
    }

    // ── bin_dir creation ──────────────────────────────────────────────────────

    /// `link_binaries` must create bin_dir even when it doesn't exist yet.
    #[test]
    fn test_link_binaries_creates_bin_dir() {
        let root = tempfile::tempdir().expect("tmpdir");
        let install_dir = root.path().join("pkg");
        let bin_dir = root.path().join("bin"); // does not exist yet

        std::fs::create_dir_all(&install_dir).unwrap();
        // extracted/ dir with a single executable
        let extracted = install_dir.join("extracted");
        std::fs::create_dir_all(&extracted).unwrap();

        let bin_path = extracted.join("mytool");
        std::fs::write(&bin_path, b"ELF").unwrap();

        // Give it the execute bit on Unix so is_executable returns true.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&bin_path).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&bin_path, p).unwrap();
        }

        let result = link_binaries(&install_dir, &bin_dir, "mytool.tar.gz");
        assert!(
            result.is_ok(),
            "link_binaries should not fail: {:?}",
            result
        );
        assert!(bin_dir.exists(), "bin_dir must be created by link_binaries");
    }
}
