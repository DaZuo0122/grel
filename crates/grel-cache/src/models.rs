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
    pub dep_forge: String,
    pub dep_owner: String,
    pub dep_repo: String,
    pub dep_type: DependencyType,
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
