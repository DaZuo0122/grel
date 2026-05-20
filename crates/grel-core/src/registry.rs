//! Central registry management.
//!
//! The registry is a git repository of `owner/repo.toml` manifest files,
//! cached locally under `<install_root>/registry/`.
//!
//! If `git` is not available on the system, the registry operates in
//! offline mode using whatever manifests are already cached.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, ManifestError};
use crate::package_ref::Forge;

/// Local registry cache manager.
#[derive(Debug, Clone)]
pub struct Registry {
    /// Path to the local registry clone
    pub path: PathBuf,
}

impl Registry {
    /// Open a registry at the given local path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Fetch or update the registry index from a remote URL.
    ///
    /// If the local path does not exist, attempts `git clone`.
    /// If it exists, attempts `git pull`.
    /// Returns an error if `git` is not available or the operation fails.
    pub fn fetch_index(&self, registry_url: &str) -> Result<(), RegistryError> {
        if !self.path.exists() {
            std::fs::create_dir_all(self.path.parent().unwrap_or(Path::new("")))?;
            run_git(&[
                "clone",
                "--depth",
                "1",
                registry_url,
                &self.path.to_string_lossy(),
            ])?;
        } else {
            run_git(&["-C", &self.path.to_string_lossy(), "pull", "--ff-only"])?;
        }
        Ok(())
    }

    /// Check if the registry has any manifests cached locally.
    pub fn is_available(&self) -> bool {
        self.path.exists() && self.path.is_dir()
    }

    /// Look up a manifest in the registry.
    ///
    /// Searches for `<registry>/<forge>/<owner>/<repo>.toml`.
    pub fn get_manifest(
        &self,
        forge: &Forge,
        owner: &str,
        repo: &str,
    ) -> Result<Option<Manifest>, RegistryError> {
        let path = self
            .path
            .join(forge.to_string())
            .join(owner)
            .join(format!("{repo}.toml"));

        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)?;
        let manifest = Manifest::load_from_str(&content)?;
        Ok(Some(manifest))
    }

    /// Search the registry for manifests matching a pattern.
    ///
    /// Searches manifest `name` and `description` fields case-insensitively.
    /// Returns a map of `forge/owner/repo` -> Manifest.
    pub fn search(&self, pattern: &str) -> Result<HashMap<String, Manifest>, RegistryError> {
        let mut results = HashMap::new();
        let pattern_lower = pattern.to_lowercase();

        if !self.path.exists() {
            return Ok(results);
        }

        self.walk_registry(&self.path.clone(), &pattern_lower, &mut results)?;
        Ok(results)
    }

    fn walk_registry(
        &self,
        dir: &Path,
        pattern: &str,
        results: &mut HashMap<String, Manifest>,
    ) -> Result<(), RegistryError> {
        if !dir.is_dir() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                self.walk_registry(&path, pattern, results)?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(manifest) = Manifest::load_from_str(&content) {
                        let name_match = manifest.name.to_lowercase().contains(pattern);
                        let desc_match = manifest.description.to_lowercase().contains(pattern);
                        if name_match || desc_match {
                            let key = format!(
                                "{}/{}/{}",
                                manifest.source.forge, manifest.source.owner, manifest.source.repo
                            );
                            results.insert(key, manifest);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Run a git command, returning an error if git is not installed or fails.
fn run_git(args: &[&str]) -> Result<(), RegistryError> {
    let output = std::process::Command::new("git")
        .args(args)
        .output()
        .map_err(|_| RegistryError::GitNotAvailable)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RegistryError::GitError(stderr.to_string()));
    }

    Ok(())
}

/// Registry errors
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Git is not available on this system")]
    GitNotAvailable,

    #[error("Git operation failed: {0}")]
    GitError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Manifest parse error: {0}")]
    ManifestError(#[from] ManifestError),
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_get_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Registry::new(tmp.path());

        // Create a fake manifest
        let github_dir = tmp.path().join("github").join("testuser");
        std::fs::create_dir_all(&github_dir).unwrap();
        std::fs::write(
            github_dir.join("testrepo.toml"),
            r#"
name = "testrepo"
[source]
forge = "github"
owner = "testuser"
repo = "testrepo"
"#,
        )
        .unwrap();

        let manifest = registry
            .get_manifest(&Forge::GitHub, "testuser", "testrepo")
            .unwrap();
        assert!(manifest.is_some());
        assert_eq!(manifest.unwrap().name, "testrepo");
    }

    #[test]
    fn test_registry_search() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Registry::new(tmp.path());

        let github_dir = tmp.path().join("github").join("testuser");
        std::fs::create_dir_all(&github_dir).unwrap();
        std::fs::write(
            github_dir.join("ripgrep.toml"),
            r#"
name = "ripgrep"
description = "A fast line-oriented search tool"
[source]
forge = "github"
owner = "testuser"
repo = "ripgrep"
"#,
        )
        .unwrap();

        let results = registry.search("fast").unwrap();
        assert_eq!(results.len(), 1);
    }
}
