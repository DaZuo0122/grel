//! Shared test helpers for grel integration tests.
//!
//! Import with `mod common;` and use `common::TempDb`, `common::fake_package`, etc.

#![allow(clippy::unwrap_used)]

use std::io::Write;
use std::path::{Path, PathBuf};

use grel_cache::{
    Database,
    models::{InstalledPackage, PackageStatus},
};

// ---------------------------------------------------------------------------
// Counter for collision-safe temp directory names
// ---------------------------------------------------------------------------

static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Create a unique temporary directory under the system temp folder.
/// Format: `<temp>/grel-test-{label}-{pid}-{counter}/`
pub fn temp_dir(label: &str) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp = std::env::temp_dir().join(format!(
        "grel-test-{label}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    tmp
}

/// Recursively remove a temp directory.  Call this explicitly if you want to
/// clean up early; otherwise `TempDb::drop` and similar guards handle it.
pub fn cleanup(tmp: &Path) {
    let _ = std::fs::remove_dir_all(tmp);
}

// ---------------------------------------------------------------------------
// RAII database guard
// ---------------------------------------------------------------------------

/// A temporary SQLite database that cleans itself up on drop.
pub struct TempDb {
    db: Option<Database>,
    pub path: PathBuf,
    tmp: PathBuf,
}

impl TempDb {
    /// Open a fresh `Database` in a new temp directory.
    pub async fn open(label: &str) -> Self {
        let tmp = temp_dir(label);
        let db_path = tmp.join("state.sqlite");
        let db = Database::init(&db_path).await.expect("Database::init");
        Self {
            db: Some(db),
            path: db_path,
            tmp,
        }
    }

    /// Access the underlying database handle.
    pub fn db(&self) -> &Database {
        self.db.as_ref().expect("database already closed")
    }

    /// Explicitly close the database and clean up the temp directory.
    pub async fn close(mut self) {
        if let Some(db) = self.db.take() {
            db.close().await;
        }
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        // Best-effort cleanup.  We can't `.await` in Drop, so we just
        // delete the directory tree.  sqlx may keep file handles open;
        // tests that need strict ordering should call `TempDb::close().await`
        // before the guard goes out of scope.
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

// ---------------------------------------------------------------------------
// Package builders
// ---------------------------------------------------------------------------

/// Create a bare `InstalledPackage` with only forge/owner/repo set.
pub fn fake_package(forge: &str, owner: &str, repo: &str) -> InstalledPackage {
    InstalledPackage::new(forge.into(), owner.into(), repo.into())
}

/// Create a fully-configured managed package ready for upsert.
pub fn fake_managed_pkg(
    forge: &str,
    owner: &str,
    repo: &str,
    version: &str,
    install_root: &Path,
    binaries: &[&str],
) -> InstalledPackage {
    let mut pkg = fake_package(forge, owner, repo);
    pkg.version = version.into();
    pkg.asset_filename = format!("{repo}-{version}.tar.gz");
    pkg.install_path = install_root.to_string_lossy().to_string();
    pkg.set_binary_list(binaries.iter().map(|s| s.to_string()).collect());
    pkg.is_managed = true;
    pkg.status = PackageStatus::Active;
    pkg
}

// ---------------------------------------------------------------------------
// Archive helpers
// ---------------------------------------------------------------------------

/// Build a valid `.tar.gz` archive containing the given entries.
/// `entries` is a list of `(archive_path, bytes)` pairs.
pub fn fake_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).expect("create archive");
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);

    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, *name, *data)
            .expect("append tar entry");
    }

    tar.finish().expect("finish tar");
}

/// Build a valid `.zip` archive containing the given entries.
pub fn fake_zip_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).expect("create zip archive");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    for (name, data) in entries {
        zip.start_file(*name, options).expect("start zip entry");
        zip.write_all(data).expect("write zip entry");
    }

    zip.finish().expect("finish zip");
}

/// Write garbage bytes that will fail gzip/tar validation.
pub fn corrupt_archive(path: &Path) {
    std::fs::write(path, b"this is not a valid archive\x00\x01\x02").expect("write corrupt");
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Make a file or directory read-only.
/// On Windows this uses `attrib +R`; on Unix it removes user-write.
pub fn make_readonly(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("attrib")
            .arg("+R")
            .arg(path)
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "attrib +R failed",
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)?;
        let mut perm = meta.permissions();
        let mode = perm.mode();
        perm.set_mode(mode & !0o200);
        std::fs::set_permissions(path, perm)?;
    }
    Ok(())
}

/// Restore write permissions (inverse of `make_readonly`).
pub fn make_writable(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("attrib")
            .arg("-R")
            .arg(path)
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "attrib -R failed",
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)?;
        let mut perm = meta.permissions();
        let mode = perm.mode();
        perm.set_mode(mode | 0o200);
        std::fs::set_permissions(path, perm)?;
    }
    Ok(())
}

/// Count files (not directories) recursively under `dir`.
pub fn count_files(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_files(&path);
            } else {
                count += 1;
            }
        }
    }
    count
}

/// Count rows in a database table.
pub async fn count_rows(db: &Database, table: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(db.pool())
        .await
        .unwrap_or((0,));
    row.0
}
