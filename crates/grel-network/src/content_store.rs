//! Content-addressed download cache.
//!
//! Archives are stored by SHA-256 hash so that identical assets are
//! deduplicated on disk.  On Unix, hard links are used when possible;
//! on Windows (or when hard-linking fails) a copy is used as fallback.

use std::path::{Path, PathBuf};

/// A content-addressed store for downloaded archives.
pub struct ContentStore {
    root: PathBuf,
}

impl ContentStore {
    /// Create a new content store at the given root path.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Return the filesystem path for a given SHA-256 hash.
    pub fn path_for(&self, sha256: &str) -> PathBuf {
        let prefix = &sha256[..2.min(sha256.len())];
        self.root.join(prefix).join(sha256)
    }

    /// Check whether the given hash is already present in the store.
    pub fn contains(&self, sha256: &str) -> bool {
        self.path_for(sha256).exists()
    }

    /// Link (or copy) cached content to a destination path.
    ///
    /// If `dest` already exists it is removed first.
    pub fn link_to(&self, sha256: &str, dest: &Path) -> std::io::Result<()> {
        let src = self.path_for(sha256);
        if dest.exists() {
            std::fs::remove_file(dest)?;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Try hard link first, fall back to copy.
        std::fs::hard_link(&src, dest).or_else(|_| {
            std::fs::copy(&src, dest).map(|_| ())
        })
    }

    /// Insert a file from an existing path into the store keyed by SHA-256.
    ///
    /// If the hash is already present this is a no-op.
    pub fn insert_from_path(&self, source: &Path, sha256: &str) -> std::io::Result<PathBuf> {
        let dest = self.path_for(sha256);
        if dest.exists() {
            return Ok(dest);
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Try hard link first, fall back to copy.
        std::fs::hard_link(source, &dest).or_else(|_| std::fs::copy(source, &dest).map(|_| ()))?;
        Ok(dest)
    }

    /// Remove entries older than `max_age_days`.
    ///
    /// Returns the number of files removed.
    pub fn clean_old(&self, max_age_days: u64) -> std::io::Result<u64> {
        let mut removed = 0u64;
        let cutoff = std::time::SystemTime::now()
            - std::time::Duration::from_secs(max_age_days * 86400);

        if !self.root.exists() {
            return Ok(0);
        }

        for prefix_entry in std::fs::read_dir(&self.root)? {
            let prefix_entry = prefix_entry?;
            if !prefix_entry.file_type()?.is_dir() {
                continue;
            }
            for file_entry in std::fs::read_dir(prefix_entry.path())? {
                let file_entry = file_entry?;
                if !file_entry.file_type()?.is_file() {
                    continue;
                }
                if let Ok(meta) = file_entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if modified < cutoff {
                            std::fs::remove_file(file_entry.path()).ok();
                            removed += 1;
                        }
                    }
                }
            }
            // Remove empty prefix directories
            if std::fs::read_dir(prefix_entry.path())?.next().is_none() {
                std::fs::remove_dir(prefix_entry.path()).ok();
            }
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_for_uses_two_char_prefix() {
        let store = ContentStore::new(PathBuf::from("/tmp/cache"));
        let path = store.path_for("abcdef123456");
        assert_eq!(path, PathBuf::from("/tmp/cache/ab/abcdef123456"));
    }

    #[test]
    fn insert_and_contains_round_trip() {
        let tmp = std::env::temp_dir().join(format!("grel-cs-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = ContentStore::new(tmp.join("store"));

        let source = tmp.join("hello.txt");
        std::fs::write(&source, b"hello world").unwrap();

        let hash = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let stored = store.insert_from_path(&source, hash).unwrap();
        assert!(stored.exists());
        assert!(store.contains(hash));

        let dest = tmp.join("linked.txt");
        store.link_to(hash, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello world");

        let _ = std::fs::remove_dir_all(&tmp);
    }

}
