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

    // -----------------------------------------------------------------------
    // Part 2.1: InstallRollback Exhaustive Coverage
    // -----------------------------------------------------------------------

    #[test]
    fn test_rollback_deletes_nested_dirs() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-rollback-nested-{}",
            std::process::id()
        ));
        let install_dir = tmp.join("install");
        let bin_dir = tmp.join("bin");

        std::fs::create_dir_all(install_dir.join("a/b/c")).unwrap();
        std::fs::write(install_dir.join("a/b/c/deep.txt"), b"deep").unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("rg"), b"bin").unwrap();

        let mut rb = InstallRollback::new(install_dir.clone(), bin_dir.clone(), tmp.join("archive.tar.gz"), true);
        rb.record_binaries(&["rg".into()]);
        rb.rollback();

        assert!(!install_dir.exists(), "nested install_dir must be removed");
        assert!(!bin_dir.join("rg").exists(), "tracked binary must be removed");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_rollback_idempotent() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-rollback-idempotent-{}",
            std::process::id()
        ));
        let install_dir = tmp.join("install");
        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::write(install_dir.join("f.txt"), b"x").unwrap();

        let rb = InstallRollback::new(install_dir.clone(), tmp.join("bin"), tmp.join("archive.tar.gz"), true);
        rb.rollback();
        assert!(!install_dir.exists());
        // Second rollback must not panic
        rb.rollback();
        assert!(!install_dir.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_rollback_preserves_unrelated_bins() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-rollback-unrelated-{}",
            std::process::id()
        ));
        let bin_dir = tmp.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("tracked"), b"a").unwrap();
        std::fs::write(bin_dir.join("unrelated"), b"b").unwrap();

        let mut rb = InstallRollback::new(tmp.join("install"), bin_dir.clone(), tmp.join("archive.tar.gz"), true);
        rb.record_binaries(&["tracked".into()]);
        rb.rollback();

        assert!(!bin_dir.join("tracked").exists());
        assert!(bin_dir.join("unrelated").exists(), "unrelated binary must be preserved");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_record_binaries_overwrite_behavior() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-rollback-overwrite-{}",
            std::process::id()
        ));
        let bin_dir = tmp.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("first"), b"a").unwrap();
        std::fs::write(bin_dir.join("second"), b"b").unwrap();

        let mut rb = InstallRollback::new(tmp.join("install"), bin_dir.clone(), tmp.join("archive.tar.gz"), true);
        rb.record_binaries(&["first".into()]);
        rb.record_binaries(&["second".into()]); // overwrite
        rb.rollback();

        assert!(bin_dir.join("first").exists(), "first was overwritten, should remain");
        assert!(!bin_dir.join("second").exists(), "second was the final tracked bin, should be removed");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // Part 2.2: Backup / Restore Collision & Safety
    // -----------------------------------------------------------------------

    #[test]
    fn test_backup_when_backup_already_exists() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-backup-collision-{}",
            std::process::id()
        ));
        let original = tmp.join("extracted");
        let backup_path = original.with_extension("grel-backup");

        // Pre-existing stale backup
        std::fs::create_dir_all(&backup_path).unwrap();
        std::fs::write(backup_path.join("stale.txt"), b"stale").unwrap();

        // Original dir
        std::fs::create_dir_all(&original).unwrap();
        std::fs::write(original.join("fresh.txt"), b"fresh").unwrap();

        let backup = backup_directory(&original);
        assert!(backup.is_some());
        assert_eq!(backup.as_ref().unwrap(), &backup_path);
        assert!(!backup_path.join("stale.txt").exists(), "stale backup must be replaced");
        assert!(backup_path.join("fresh.txt").exists(), "new backup must contain fresh data");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_restore_when_original_still_exists() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-restore-exists-{}",
            std::process::id()
        ));
        let original = tmp.join("extracted");
        let backup = tmp.join("extracted.grel-backup");

        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("backup.txt"), b"backup").unwrap();

        // Original was somehow recreated
        std::fs::create_dir_all(&original).unwrap();
        std::fs::write(original.join("new.txt"), b"new").unwrap();

        restore_backup(&backup, &original);
        assert!(original.exists());
        assert!(original.join("backup.txt").exists(), "restored file must be present");
        assert!(!original.join("new.txt").exists(), "recreated original must be replaced");
        assert!(!backup.exists(), "backup must be consumed");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_restore_when_backup_missing() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-restore-missing-{}",
            std::process::id()
        ));
        let original = tmp.join("extracted");
        let missing_backup = tmp.join("missing.grel-backup");

        // Should not panic
        restore_backup(&missing_backup, &original);
        assert!(!original.exists());
    }

    #[test]
    fn test_discard_backup_idempotent() {
        let tmp = std::env::temp_dir().join(format!(
            "grel-discard-idempotent-{}",
            std::process::id()
        ));
        let backup = tmp.join("extracted.grel-backup");
        std::fs::create_dir_all(&backup).unwrap();

        discard_backup(&backup);
        assert!(!backup.exists());
        // Second discard must not panic
        discard_backup(&backup);
        assert!(!backup.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
