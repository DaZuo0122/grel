//! Package manifest format (`grel.toml`).
//!
//! Manifests provide structured metadata and exact asset patterns,
//! replacing or augmenting heuristic filename resolution.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::package_ref::{Forge, PackageRef};
use crate::platform::{Arch, Os};

/// A `grel.toml` manifest file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Human-readable package name
    #[serde(default)]
    pub name: String,

    /// Short description
    #[serde(default)]
    pub description: String,

    /// SPDX license identifier
    #[serde(default)]
    pub license: String,

    /// Source repository information
    #[serde(default)]
    pub source: SourceSpec,

    /// Per-platform asset mappings
    #[serde(default)]
    pub assets: Vec<AssetMapping>,

    /// Optional checksum filename pattern
    #[serde(default)]
    pub checksum_filename: Option<String>,

    /// Optional detached signature pattern
    #[serde(default)]
    pub signature_filename: Option<String>,

    /// Signature kind (e.g. "minisign", "gpg", "sha256")
    #[serde(default)]
    pub signature_kind: Option<String>,

    /// Dependency specification
    #[serde(default)]
    pub dependencies: DependencySpec,

    /// Install hooks
    #[serde(default)]
    pub hooks: HookSpec,
}

impl Manifest {
    /// Parse a manifest from a TOML string.
    pub fn load_from_str(s: &str) -> Result<Self, ManifestError> {
        toml::from_str(s).map_err(ManifestError::from)
    }

    /// Resolve the best asset for the given platform and version.
    ///
    /// Substitutes `{version}` in the filename pattern with the provided
    /// version string, then returns the first matching asset mapping.
    pub fn resolve_asset(
        &self,
        target_os: &Os,
        target_arch: &Arch,
        _version: &str,
    ) -> Option<&AssetMapping> {
        self.assets.iter().find(|asset| {
            let os_matches = asset.os_matches(target_os);
            let arch_matches = asset.arch_matches(target_arch);
            os_matches && arch_matches
        })
    }

    /// Substitute `{version}` in a pattern string.
    pub fn subst_version(pattern: &str, version: &str) -> String {
        pattern.replace("{version}", version)
    }

    /// Parse grel dependencies into structured `PackageRef`s.
    pub fn parse_grel_deps(&self) -> Vec<PackageRef> {
        let forge = self.source.forge_enum().unwrap_or(Forge::GitHub);
        self.dependencies
            .grel
            .iter()
            .filter_map(|s| PackageRef::parse_with_forge(s, forge).ok())
            .collect()
    }

    /// Parse optional grel dependencies.
    pub fn parse_grel_opt_deps(&self) -> Vec<(PackageRef, String)> {
        let forge = self.source.forge_enum().unwrap_or(Forge::GitHub);
        self.dependencies
            .grel_opt
            .iter()
            .filter_map(|(s, reason)| {
                PackageRef::parse_with_forge(s, forge)
                    .ok()
                    .map(|p| (p, reason.clone()))
            })
            .collect()
    }
}

/// Source repository specification.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceSpec {
    #[serde(default)]
    pub forge: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub repo: String,
}

impl SourceSpec {
    pub fn forge_enum(&self) -> Option<Forge> {
        if self.forge.is_empty() {
            return None;
        }
        self.forge.parse().ok()
    }
}

/// A per-platform asset mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetMapping {
    /// Target OS (e.g. "linux", "windows", "macos")
    pub os: String,

    /// Target architecture (e.g. "x86_64", "aarch64")
    pub arch: String,

    /// Filename pattern with `{version}` placeholder.
    /// Example: `ripgrep-{version}-x86_64-unknown-linux-musl.tar.gz`
    pub filename: String,

    /// Archive format (e.g. "tar.gz", "zip", "exe")
    #[serde(default)]
    pub format: String,

    /// Whether grel should extract and manage this asset.
    #[serde(default = "default_true")]
    pub managed: bool,
}

impl AssetMapping {
    /// Check if this mapping matches the target OS.
    pub fn os_matches(&self, target: &Os) -> bool {
        let target_str = target.to_string().to_lowercase();
        let asset_os = self.os.to_lowercase();
        asset_os == target_str || asset_os == "any" || asset_os == "unknown"
    }

    /// Check if this mapping matches the target architecture.
    pub fn arch_matches(&self, target: &Arch) -> bool {
        let target_str = target.to_string().to_lowercase();
        let asset_arch = self.arch.to_lowercase();
        asset_arch == target_str || asset_arch == "any" || asset_arch == "unknown"
    }

    /// Get the concrete filename for a given version.
    pub fn concrete_filename(&self, version: &str) -> String {
        Manifest::subst_version(&self.filename, version)
    }
}

fn default_true() -> bool {
    true
}

/// Dependency specification.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DependencySpec {
    /// Required grel-installable packages.
    #[serde(default)]
    pub grel: Vec<String>,

    /// Optional grel-installable packages: name -> reason.
    #[serde(default)]
    pub grel_opt: HashMap<String, String>,

    /// Required system libraries (Linux .so names, etc.).
    #[serde(default)]
    pub system: Vec<String>,

    /// Optional system libraries: name -> reason.
    #[serde(default)]
    pub system_opt: HashMap<String, String>,
}

/// Install hooks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookSpec {
    /// Script to run after extraction (relative to extracted archive root)
    #[serde(default)]
    pub post_install: Option<String>,

    /// Script to run before removal
    #[serde(default)]
    pub pre_remove: Option<String>,
}

/// Manifest errors
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("Failed to parse manifest TOML: {0}")]
    ParseError(#[from] toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_manifest() {
        let toml = r#"
name = "ripgrep"
description = "A fast line-oriented search tool"
license = "MIT"

[source]
forge = "github"
owner = "BurntSushi"
repo = "ripgrep"

[[assets]]
os = "linux"
arch = "x86_64"
filename = "ripgrep-{version}-x86_64-unknown-linux-musl.tar.gz"
format = "tar.gz"
managed = true

[dependencies]
grel = []
system = ["libc6"]
"#;

        let manifest = Manifest::load_from_str(toml).unwrap();
        assert_eq!(manifest.name, "ripgrep");
        assert_eq!(manifest.source.owner, "BurntSushi");
        assert_eq!(manifest.assets.len(), 1);
        assert_eq!(
            manifest.assets[0].filename,
            "ripgrep-{version}-x86_64-unknown-linux-musl.tar.gz"
        );
    }

    #[test]
    fn test_resolve_asset() {
        let toml = r#"
[[assets]]
os = "linux"
arch = "x86_64"
filename = "foo-{version}-linux-x64.tar.gz"

[[assets]]
os = "windows"
arch = "x86_64"
filename = "foo-{version}-win-x64.zip"
"#;

        let manifest = Manifest::load_from_str(toml).unwrap();
        let linux = Os::Linux;
        let x86_64 = Arch::X86_64;
        let asset = manifest.resolve_asset(&linux, &x86_64, "v1.0.0").unwrap();
        assert_eq!(
            asset.concrete_filename("v1.0.0"),
            "foo-v1.0.0-linux-x64.tar.gz"
        );
    }

    #[test]
    fn test_subst_version() {
        assert_eq!(
            Manifest::subst_version("foo-{version}-bar.tar.gz", "1.2.3"),
            "foo-1.2.3-bar.tar.gz"
        );
    }

    #[test]
    fn test_os_arch_matches() {
        let mapping = AssetMapping {
            os: "linux".into(),
            arch: "x86_64".into(),
            filename: "test.tar.gz".into(),
            format: "tar.gz".into(),
            managed: true,
        };
        assert!(mapping.os_matches(&Os::Linux));
        assert!(!mapping.os_matches(&Os::Windows));
        assert!(mapping.arch_matches(&Arch::X86_64));
        assert!(!mapping.arch_matches(&Arch::Aarch64));
    }
}
