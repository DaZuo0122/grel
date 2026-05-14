//! Database models for the installed packages table.

use serde::{Deserialize, Serialize};

/// Status of an installed package
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageStatus {
    Active,
    Orphaned,
    Migrated,
}

/// Source of the manifest used during installation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManifestSource {
    /// From the central registry
    Registry,
    /// From `.grel.toml` in the upstream repo
    InRepo,
    /// Heuristic filename resolution (no manifest)
    Heuristic,
}

impl std::fmt::Display for PackageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageStatus::Active => write!(f, "active"),
            PackageStatus::Orphaned => write!(f, "orphaned"),
            PackageStatus::Migrated => write!(f, "migrated"),
        }
    }
}

impl PackageStatus {
    pub fn from_str(s: &str) -> Self {
        match s {
            "active" => PackageStatus::Active,
            "orphaned" => PackageStatus::Orphaned,
            "migrated" => PackageStatus::Migrated,
            _ => PackageStatus::Active,
        }
    }
}

impl std::fmt::Display for ManifestSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestSource::Registry => write!(f, "registry"),
            ManifestSource::InRepo => write!(f, "in_repo"),
            ManifestSource::Heuristic => write!(f, "heuristic"),
        }
    }
}

impl ManifestSource {
    pub fn from_str(s: &str) -> Self {
        match s {
            "registry" => ManifestSource::Registry,
            "in_repo" => ManifestSource::InRepo,
            _ => ManifestSource::Heuristic,
        }
    }
}

/// Represents an installed package in the database.
///
/// ## File layout
/// - **Managed packages**: archive + extracted files live under
///   `<install_root>/<forge>/<owner>/<repo>/`. Binaries are linked or
///   copied into `bin_dir/`.
/// - **Unmanaged packages**: the downloaded file lives in `download_dir/`
///   and `install_path` points to it directly.
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub id: Option<i64>,
    pub forge: String,
    pub owner: String,
    pub repo: String,
    pub version: String,
    pub asset_filename: String,
    pub checksum: Option<String>,
    /// For managed packages: `<install_root>/<forge>/<owner>/<repo>/`
    /// (the directory containing the downloaded archive + extracted tree).
    /// For unmanaged: the full path to the downloaded file.
    pub install_path: String,
    /// Binary filenames that were installed into `bin_dir/`.
    /// Stored as a semicolon-separated list for simplicity.
    pub installed_binaries: String,
    pub is_managed: bool,
    pub status: PackageStatus,
    pub orphaned_at: Option<i64>,
    pub last_checked: Option<i64>,
    pub installed_at: Option<i64>,
    pub manifest_source: ManifestSource,
    pub is_explicit: bool,
}

impl InstalledPackage {
    /// Create a new package reference
    pub fn new(forge: String, owner: String, repo: String) -> Self {
        Self {
            id: None,
            forge,
            owner,
            repo,
            version: String::new(),
            asset_filename: String::new(),
            checksum: None,
            install_path: String::new(),
            installed_binaries: String::new(),
            is_managed: true,
            status: PackageStatus::Active,
            orphaned_at: None,
            last_checked: None,
            installed_at: None,
            manifest_source: ManifestSource::Heuristic,
            is_explicit: true,
        }
    }

    /// Get the full package reference string
    pub fn package_ref(&self) -> String {
        format!("{}/{}/{}", self.forge, self.owner, self.repo)
    }

    /// List of installed binary filenames (split from semicolon-separated string)
    pub fn binary_list(&self) -> Vec<String> {
        if self.installed_binaries.is_empty() {
            return vec![];
        }
        self.installed_binaries
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Set the list of installed binary filenames
    pub fn set_binary_list(&mut self, binaries: Vec<String>) {
        self.installed_binaries = binaries.join(";");
    }
}

/// A dependency relationship between packages
#[derive(Debug, Clone)]
pub struct Dependency {
    pub id: Option<i64>,
    pub package_id: i64,
    /// Target of the dependency.
    /// - Grel packages: "forge/owner/repo" (e.g., "github/BurntSushi/ripgrep")
    /// - System libraries: "system:libname" (e.g., "system:libssl.so.3")
    pub dep_target: String,
    pub dep_type: DependencyType,
}

impl Dependency {
    /// Create a dependency on a grel-installable package.
    pub fn grel(package_id: i64, pkg_ref: &grel_core::PackageRef) -> Self {
        Self {
            id: None,
            package_id,
            dep_target: format!("{}/{}/{}", pkg_ref.forge, pkg_ref.owner, pkg_ref.repo),
            dep_type: DependencyType::Grel,
        }
    }

    /// Create a dependency on a system shared library.
    pub fn system(package_id: i64, lib_name: &str) -> Self {
        Self {
            id: None,
            package_id,
            dep_target: format!("system:{lib_name}"),
            dep_type: DependencyType::System,
        }
    }

    /// If this is a grel dependency, parse `dep_target` into a `PackageRef`.
    pub fn as_grel_ref(&self) -> Option<grel_core::PackageRef> {
        if self.dep_type != DependencyType::Grel && self.dep_type != DependencyType::GrelOpt {
            return None;
        }
        grel_core::PackageRef::parse(&self.dep_target).ok()
    }

    /// If this is a system dependency, return the library name (without the "system:" prefix).
    pub fn as_system_lib(&self) -> Option<&str> {
        if self.dep_type != DependencyType::System {
            return None;
        }
        self.dep_target.strip_prefix("system:")
    }

    /// Human-readable display of the dependency target.
    pub fn display_target(&self) -> String {
        match self.dep_type {
            DependencyType::System => {
                self.as_system_lib()
                    .map_or_else(|| self.dep_target.clone(), |lib| format!("{lib} [system]"))
            }
            _ => self.dep_target.clone(),
        }
    }
}

/// Type of dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyType {
    Grel,
    GrelOpt,
    System,
}

impl std::fmt::Display for DependencyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyType::Grel => write!(f, "grel"),
            DependencyType::GrelOpt => write!(f, "grel_opt"),
            DependencyType::System => write!(f, "system"),
        }
    }
}

impl DependencyType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "grel_opt" => DependencyType::GrelOpt,
            "system" => DependencyType::System,
            _ => DependencyType::Grel,
        }
    }
}

/// ETag cache entry
#[derive(Debug, Clone)]
pub struct ETagEntry {
    pub url: String,
    pub etag: String,
    pub last_modified: i64,
}

/// DNS/IP cache entry
#[derive(Debug, Clone)]
pub struct DNSCacheEntry {
    pub hostname: String,
    pub ip_address: String,
    pub rtt_ms: u64,
    pub expires_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use grel_core::{Forge, PackageRef};

    #[test]
    fn test_dependency_grel_constructor() {
        let pkg_ref = PackageRef::new(Forge::GitHub, "BurntSushi".into(), "ripgrep".into(), None);
        let dep = Dependency::grel(42, &pkg_ref);

        assert_eq!(dep.package_id, 42);
        assert_eq!(dep.dep_target, "github/BurntSushi/ripgrep");
        assert_eq!(dep.dep_type, DependencyType::Grel);
    }

    #[test]
    fn test_dependency_system_constructor() {
        let dep = Dependency::system(7, "libssl.so.3");

        assert_eq!(dep.package_id, 7);
        assert_eq!(dep.dep_target, "system:libssl.so.3");
        assert_eq!(dep.dep_type, DependencyType::System);
    }

    #[test]
    fn test_as_grel_ref_parses_correctly() {
        let pkg_ref = PackageRef::new(Forge::GitLab, "owner".into(), "repo".into(), None);
        let dep = Dependency::grel(1, &pkg_ref);

        let parsed = dep.as_grel_ref().expect("should parse");
        assert_eq!(parsed.forge, Forge::GitLab);
        assert_eq!(parsed.owner, "owner");
        assert_eq!(parsed.repo, "repo");
    }

    #[test]
    fn test_as_grel_ref_returns_none_for_system() {
        let dep = Dependency::system(1, "libcurl.so.4");
        assert!(dep.as_grel_ref().is_none());
    }

    #[test]
    fn test_as_system_lib_extracts_name() {
        let dep = Dependency::system(1, "libz.so.1");
        assert_eq!(dep.as_system_lib(), Some("libz.so.1"));
    }

    #[test]
    fn test_as_system_lib_returns_none_for_grel() {
        let pkg_ref = PackageRef::new(Forge::GitHub, "a".into(), "b".into(), None);
        let dep = Dependency::grel(1, &pkg_ref);
        assert!(dep.as_system_lib().is_none());
    }

    #[test]
    fn test_display_target_grel() {
        let pkg_ref = PackageRef::new(Forge::GitHub, "a".into(), "b".into(), None);
        let dep = Dependency::grel(1, &pkg_ref);
        assert_eq!(dep.display_target(), "github/a/b");
    }

    #[test]
    fn test_display_target_system() {
        let dep = Dependency::system(1, "libssl.so.3");
        assert_eq!(dep.display_target(), "libssl.so.3 [system]");
    }
}
