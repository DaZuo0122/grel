//! Main configuration structures and loading logic.

use std::path::PathBuf;

use figment::{
    providers::{Env, Format, Toml, Serialized},
    Figment,
};
use serde::{Deserialize, Serialize};

use crate::paths::{default_bin_dir, default_install_root};

/// Schema version for the configuration
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Default check interval in hours
pub const DEFAULT_CHECK_INTERVAL_HOURS: u64 = 6;

/// Default max concurrent downloads
pub const DEFAULT_MAX_CONCURRENT: usize = 4;

/// Default max parallel checks for -Syu
pub const DEFAULT_MAX_PARALLEL_CHECKS: usize = 10;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub assets: AssetConfig,
    pub paths: PathConfig,
    pub upgrade: UpgradeConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub migrations: std::collections::HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            assets: AssetConfig::default(),
            paths: PathConfig::default(),
            upgrade: UpgradeConfig::default(),
            auth: AuthConfig::default(),
            migrations: std::collections::HashMap::new(),
        }
    }
}

/// General settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Schema version (do not modify)
    #[serde(default = "default_schema_version")]
    pub version: u32,

    /// Parallel downloads (0 = auto CPU/2, min 2)
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,

    /// Proxy setting (empty = auto-detect)
    #[serde(default)]
    pub proxy: String,

    /// Keep downloaded archives after extraction (default: true)
    #[serde(default = "default_keep_archives")]
    pub keep_archives: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            version: default_schema_version(),
            max_concurrent: default_max_concurrent(),
            proxy: String::new(),
            keep_archives: default_keep_archives(),
        }
    }
}

fn default_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

fn default_max_concurrent() -> usize {
    DEFAULT_MAX_CONCURRENT
}

fn default_keep_archives() -> bool {
    true
}

/// Asset resolution settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetConfig {
    /// Default selection policy: "first" | "largest"
    #[serde(default = "default_selection_policy_default")]
    pub default_selection_policy: SelectionPolicy,

    /// Keywords to exclude (hard block)
    #[serde(default = "exclude_keywords_default")]
    pub exclude_keywords: Vec<String>,

    /// Formats to ignore by default
    #[serde(default = "ignore_formats_default")]
    pub ignore_formats: Vec<String>,

    /// Preferred formats
    #[serde(default = "prefer_formats_default")]
    pub prefer_formats: Vec<String>,

    /// Prefers 32-bit assets ONLY when running on 64-bit OS
    #[serde(default)]
    pub prefer_32bit_on_64bit: bool,

    /// Allows 32-bit install if NO 64-bit asset exists
    #[serde(default = "fallback_to_32bit_default")]
    pub fallback_to_32bit: bool,

    /// Linux-only: prefers musl over gnu builds
    #[serde(default)]
    pub prefer_musl: bool,
}

impl Default for AssetConfig {
    fn default() -> Self {
        Self {
            default_selection_policy: default_selection_policy_default(),
            exclude_keywords: exclude_keywords_default(),
            ignore_formats: ignore_formats_default(),
            prefer_formats: prefer_formats_default(),
            prefer_32bit_on_64bit: false,
            fallback_to_32bit: fallback_to_32bit_default(),
            prefer_musl: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionPolicy {
    First,
    Largest,
}

fn default_selection_policy_default() -> SelectionPolicy {
    SelectionPolicy::First
}

fn exclude_keywords_default() -> Vec<String> {
    vec![
        "setup".into(),
        "installer".into(),
        "bundle".into(),
        "nupkg".into(),
        // Standalone checksum / signature files without an extension
        // (e.g. "SHA256SUMS", "checksums", "MD5SUMS")
        "sha256sums".into(),
        "sha512sums".into(),
        "md5sums".into(),
        "checksums".into(),
    ]
}

fn ignore_formats_default() -> Vec<String> {
    vec![
        "*.deb".into(),
        "*.rpm".into(),
        "*.msi".into(),
        "*.dmg".into(),
        "*.pkg".into(),
        "*.AppImage".into(),
        // Checksum files (e.g. "foo.tar.gz.sha256")
        "*.sha256".into(),
        "*.sha512".into(),
        "*.sha384".into(),
        "*.sha1".into(),
        "*.md5".into(),
        "*.b2sum".into(),
        // Detached signature files
        "*.asc".into(),
        "*.sig".into(),
        "*.minisig".into(),
    ]
}

fn prefer_formats_default() -> Vec<String> {
    vec![
        "*.tar.gz".into(),
        "*.tar.xz".into(),
        "*.zip".into(),
        "*.exe".into(),
    ]
}

fn fallback_to_32bit_default() -> bool {
    true
}

/// Path configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConfig {
    /// Managed package storage
    #[serde(default = "install_root_default")]
    pub install_root: PathBuf,

    /// Extracted binaries (added to $PATH)
    #[serde(default = "bin_dir_default")]
    pub bin_dir: PathBuf,

    /// Unmanaged/extra-op packages
    #[serde(default = "download_dir_default")]
    pub download_dir: PathBuf,
}

impl Default for PathConfig {
    fn default() -> Self {
        Self {
            install_root: install_root_default(),
            bin_dir: bin_dir_default(),
            download_dir: download_dir_default(),
        }
    }
}

fn install_root_default() -> PathBuf {
    default_install_root()
}

fn bin_dir_default() -> PathBuf {
    default_bin_dir()
}

fn download_dir_default() -> PathBuf {
    // Fallback to system Downloads directory or temp
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir().join("grel-downloads"))
}

/// Upgrade settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeConfig {
    /// Remote check cooldown per package
    #[serde(default = "check_interval_default")]
    pub check_interval_hours: u64,

    /// Concurrent API requests during -Syu
    #[serde(default = "max_parallel_checks_default")]
    pub max_parallel_checks: usize,
}

impl Default for UpgradeConfig {
    fn default() -> Self {
        Self {
            check_interval_hours: check_interval_default(),
            max_parallel_checks: max_parallel_checks_default(),
        }
    }
}

fn check_interval_default() -> u64 {
    DEFAULT_CHECK_INTERVAL_HOURS
}

fn max_parallel_checks_default() -> usize {
    DEFAULT_MAX_PARALLEL_CHECKS
}

/// Authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    /// Use env $GREL_GITHUB_TOKEN instead
    #[serde(default)]
    pub github_token: String,

    /// Use env $GREL_GITLAB_TOKEN instead
    #[serde(default)]
    pub gitlab_token: String,

    /// Use env $GREL_GITEA_TOKEN instead
    #[serde(default)]
    pub gitea_token: String,
}

/// Load configuration from file, environment, and defaults
pub fn load_config(config_path: Option<&std::path::Path>) -> Result<Config, ConfigError> {
    let mut figment = Figment::from(Serialized::defaults(Config::default()));

    // Load from config file if provided
    if let Some(path) = config_path {
        figment = figment.merge(Toml::file(path));
    }

    // Load from environment variables (GREL_ prefix)
    figment = figment.merge(Env::prefixed("GREL_").global());

    let config: Config = figment.extract()?;

    // Validate schema version
    if config.general.version != CONFIG_SCHEMA_VERSION {
        return Err(ConfigError::InvalidSchemaVersion {
            found: config.general.version,
            expected: CONFIG_SCHEMA_VERSION,
        });
    }

    Ok(config)
}

/// Configuration errors
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Invalid configuration schema version: found {found}, expected {expected}")]
    InvalidSchemaVersion { found: u32, expected: u32 },

    #[error("Configuration loading failed: {0}")]
    LoadError(#[from] figment::Error),

    #[error("Invalid configuration value: {0}")]
    InvalidValue(String),
}
