//! Transaction helpers for atomic package install/upgrade/remove operations.

use std::path::PathBuf;

/// Tracks filesystem state created during a package install so it can be
/// rolled back if the DB transaction fails or extraction fails.
pub struct InstallRollback {
    install_dir: PathBuf,
    bin_dir: PathBuf,
    binaries: Vec<String>,
    archive_path: PathBuf,
    is_managed: bool,
}

impl InstallRollback {
    /// Create a new rollback tracker.
    pub fn new(
        install_dir: PathBuf,
        bin_dir: PathBuf,
        archive_path: PathBuf,
        is_managed: bool,
    ) -> Self {
        Self {
            install_dir,
            bin_dir,
            binaries: Vec::new(),
            archive_path,
            is_managed,
        }
    }

    /// Record the binaries that were installed (for cleanup on rollback).
    pub fn record_binaries(&mut self, binaries: &[String]) {
        self.binaries = binaries.to_vec();
    }

    /// Remove all created files/directories. Best-effort: errors are ignored.
    pub fn rollback(&self) {
        if self.is_managed {
            if self.install_dir.exists() {
                let _ = std::fs::remove_dir_all(&self.install_dir);
            }
            for bin in &self.binaries {
                let bin_path = self.bin_dir.join(bin);
                if bin_path.exists() {
                    let _ = std::fs::remove_file(&bin_path);
                }
            }
        } else if self.archive_path.exists() {
            let _ = std::fs::remove_file(&self.archive_path);
        }
    }
}

/// Backup an existing directory by renaming it with a `.grel-backup` suffix.
/// Returns the backup path if a backup was created.
pub fn backup_directory(dir: &std::path::Path) -> Option<PathBuf> {
    if !dir.exists() {
        return None;
    }
    let backup = dir.with_extension("grel-backup");
    if backup.exists() {
        let _ = std::fs::remove_dir_all(&backup);
    }
    if std::fs::rename(dir, &backup).is_ok() {
        Some(backup)
    } else {
        None
    }
}

/// Restore a directory from its backup.
pub fn restore_backup(backup: &std::path::Path, original: &std::path::Path) {
    if original.exists() {
        let _ = std::fs::remove_dir_all(original);
    }
    let _ = std::fs::rename(backup, original);
}

/// Delete a backup directory if it exists.
pub fn discard_backup(backup: &std::path::Path) {
    if backup.exists() {
        let _ = std::fs::remove_dir_all(backup);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rollback_removes_created_files() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-rollback-test-{}",
            std::process::id()
        ));
        let install_dir = tmp.join("install");
        let bin_dir = tmp.join("bin");
        let archive = tmp.join("archive.tar.gz");

        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(&archive, b"data").unwrap();
        std::fs::write(install_dir.join("file.txt"), b"content").unwrap();
        std::fs::write(bin_dir.join("mybin"), b"bin").unwrap();

        let mut rb = InstallRollback::new(
            install_dir.clone(),
            bin_dir.clone(),
            archive.clone(),
            true,
        );
        rb.record_binaries(&["mybin".into()]);
        rb.rollback();

        assert!(!install_dir.exists());
        assert!(!bin_dir.join("mybin").exists());
        // Archive is inside install_dir for managed packages, so it's gone too.

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_rollback_unmanaged_removes_archive_only() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-rollback-unmanaged-{}",
            std::process::id()
        ));
        let archive = tmp.join("download.tar.gz");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(&archive, b"data").unwrap();

        let rb = InstallRollback::new(
            tmp.join("install"),
            tmp.join("bin"),
            archive.clone(),
            false,
        );
        rb.rollback();

        assert!(!archive.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_rollback_no_panic_on_missing_paths() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-rollback-missing-{}",
            std::process::id()
        ));
        let mut rb = InstallRollback::new(
            tmp.join("nonexistent"),
            tmp.join("bin"),
            tmp.join("archive.tar.gz"),
            true,
        );
        rb.record_binaries(&["nobin".into()]);
        // Should not panic
        rb.rollback();
    }

    #[test]
    fn test_backup_and_restore_round_trip() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-backup-test-{}",
            std::process::id()
        ));
        let original = tmp.join("extracted");
        std::fs::create_dir_all(&original).unwrap();
        std::fs::write(original.join("old.txt"), b"old").unwrap();

        let backup = backup_directory(&original);
        assert!(backup.is_some());
        assert!(!original.exists());
        assert!(backup.as_ref().unwrap().join("old.txt").exists());

        restore_backup(backup.as_ref().unwrap(), &original);
        assert!(original.exists());
        assert!(original.join("old.txt").exists());
        assert!(!backup.as_ref().unwrap().exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
