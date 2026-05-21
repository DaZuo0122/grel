//! Main configuration structures and loading logic.

use std::collections::HashMap;
use std::path::PathBuf;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
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

/// Default registry URL
pub const DEFAULT_REGISTRY_URL: &str = "https://github.com/grel-registry/packages";

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub assets: AssetConfig,
    pub paths: PathConfig,
    pub upgrade: UpgradeConfig,
    pub auth: AuthConfig,
    pub security: SecurityConfig,
    pub registry: RegistryConfig,
    #[serde(default)]
    pub elf_deps: ElfDepConfig,
    #[serde(default)]
    pub migrations: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            assets: AssetConfig::default(),
            paths: PathConfig::default(),
            upgrade: UpgradeConfig::default(),
            auth: AuthConfig::default(),
            security: SecurityConfig::default(),
            registry: RegistryConfig::default(),
            elf_deps: ElfDepConfig::default(),
            migrations: HashMap::new(),
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

    /// Max retries for transient download failures (default: 3)
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Base delay between retries in milliseconds (default: 1000)
    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: u64,

    /// Overall request timeout in seconds (default: 300)
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    /// TCP connect timeout in seconds (default: 30)
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,

    /// Max idle connections per host in the HTTP pool (default: 10)
    #[serde(default = "default_pool_max_idle")]
    pub pool_max_idle: usize,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            version: default_schema_version(),
            max_concurrent: default_max_concurrent(),
            proxy: String::new(),
            keep_archives: default_keep_archives(),
            max_retries: default_max_retries(),
            retry_delay_ms: default_retry_delay_ms(),
            timeout_secs: default_timeout_secs(),
            connect_timeout_secs: default_connect_timeout_secs(),
            pool_max_idle: default_pool_max_idle(),
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

fn default_max_retries() -> u32 {
    3
}

fn default_retry_delay_ms() -> u64 {
    1000
}

fn default_timeout_secs() -> u64 {
    300
}

fn default_connect_timeout_secs() -> u64 {
    30
}

fn default_pool_max_idle() -> usize {
    10
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

/// Security settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Verify cryptographic signatures on downloaded assets.
    /// When true, grel will refuse to install assets that lack
    /// a valid detached signature or checksum file.
    #[serde(default = "default_verify_signatures")]
    pub verify_signatures: bool,

    /// Verify upstream checksums on downloaded assets.
    /// When true, grel will download and compare the published
    /// checksum (sha256/sha512/md5) against the computed hash.
    #[serde(default = "default_verify_checksums")]
    pub verify_checksums: bool,

    /// Enable execution of manifest hooks (post_install / pre_remove).
    /// Disabled by default for security.
    #[serde(default = "default_enable_hooks")]
    pub enable_hooks: bool,

    /// Trusted PGP public keys (ASCII-armored or binary, one per element).
    /// Used to verify GPG detached signatures when verify_signatures is true.
    #[serde(default)]
    pub trusted_pgp_keys: Vec<String>,

    /// Minisign public key (base64 encoded, e.g. "RWQf6LRCGA9i53ml...").
    /// Used to verify minisign signatures when verify_signatures is true.
    #[serde(default)]
    pub minisign_public_key: Option<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            verify_signatures: default_verify_signatures(),
            verify_checksums: default_verify_checksums(),
            enable_hooks: default_enable_hooks(),
            trusted_pgp_keys: Vec::new(),
            minisign_public_key: None,
        }
    }
}

fn default_verify_signatures() -> bool {
    false
}

fn default_verify_checksums() -> bool {
    false
}

fn default_enable_hooks() -> bool {
    false
}

/// Registry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// URL to the git repository serving as the central manifest registry.
    #[serde(default = "default_registry_url")]
    pub url: String,

    /// Automatically update the registry index on sync operations.
    #[serde(default = "default_registry_auto_update")]
    pub auto_update: bool,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            url: default_registry_url(),
            auto_update: default_registry_auto_update(),
        }
    }
}

fn default_registry_url() -> String {
    DEFAULT_REGISTRY_URL.to_string()
}

fn default_registry_auto_update() -> bool {
    true
}

/// ELF system dependency auto-resolution settings (Linux-only feature).
///
/// Env var equivalents use the `GREL_ELF_DEPS__` prefix, e.g.:
///   GREL_ELF_DEPS__AUTO_RESOLVE_SYSTEM_DEPS=false
///   GREL_ELF_DEPS__DISTRO_OVERRIDE=debian
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElfDepConfig {
    /// Master switch: parse DT_NEEDED and offer to install missing system libs.
    #[serde(default = "default_true")]
    pub auto_resolve_system_deps: bool,

    /// Print the list of discovered DT_NEEDED libraries before resolving.
    #[serde(default = "default_true")]
    pub show_parsed_deps: bool,

    /// Override distro detection (e.g. "debian", "fedora", "arch").
    #[serde(default)]
    pub distro_override: Option<String>,

    /// Override the entire install command template.
    /// Use `{packages}` as the placeholder, e.g. "sudo apt-get install -y {packages}".
    #[serde(default)]
    pub install_cmd_template: Option<String>,

    /// Global library → package name overrides (applies on all distros).
    #[serde(default)]
    pub library_map: HashMap<String, String>,

    /// Per-distro library → package name overrides.
    /// Keys are distro IDs (e.g. "fedora"). Takes precedence over `library_map`.
    #[serde(default)]
    pub distro_library_map: HashMap<String, HashMap<String, String>>,
}

impl Default for ElfDepConfig {
    fn default() -> Self {
        Self {
            auto_resolve_system_deps: true,
            show_parsed_deps: true,
            distro_override: None,
            install_cmd_template: None,
            library_map: HashMap::new(),
            distro_library_map: HashMap::new(),
        }
    }
}

fn default_true() -> bool {
    true
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
