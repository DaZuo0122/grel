//! Database models for the installed packages table.

use serde::{Deserialize, Serialize};

/// Status of an installed package
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageStatus {
    Active,
    Orphaned,
    Migrated,
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

/// Represents an installed package in the database
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub id: Option<i64>,
    pub forge: String,
    pub owner: String,
    pub repo: String,
    pub version: String,
    pub asset_filename: String,
    pub checksum: Option<String>,
    pub install_path: String,
    pub is_managed: bool,
    pub status: PackageStatus,
    pub orphaned_at: Option<i64>,
    pub last_checked: Option<i64>,
    pub installed_at: Option<i64>,
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
            is_managed: true,
            status: PackageStatus::Active,
            orphaned_at: None,
            last_checked: None,
            installed_at: None,
        }
    }

    /// Get the full package reference string
    pub fn package_ref(&self) -> String {
        format!("{}/{}/{}", self.forge, self.owner, self.repo)
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
