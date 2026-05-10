//! Distro detection via /etc/os-release with configurable family mapping.

use std::collections::HashMap;
use std::sync::OnceLock;

use phf::phf_map;

/// Known distro families for package manager selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroFamily {
    Debian,
    RedHat,
    Arch,
    Alpine,
    Suse,
    Unknown,
}

impl std::str::FromStr for DistroFamily {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "debian" => Ok(DistroFamily::Debian),
            "redhat" | "rhel" | "fedora" => Ok(DistroFamily::RedHat),
            "arch" => Ok(DistroFamily::Arch),
            "alpine" => Ok(DistroFamily::Alpine),
            "suse" | "opensuse" => Ok(DistroFamily::Suse),
            "unknown" => Ok(DistroFamily::Unknown),
            _ => Err(format!("Unknown distro family: {s}")),
        }
    }
}

impl std::fmt::Display for DistroFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DistroFamily::Debian => write!(f, "debian"),
            DistroFamily::RedHat => write!(f, "redhat"),
            DistroFamily::Arch => write!(f, "arch"),
            DistroFamily::Alpine => write!(f, "alpine"),
            DistroFamily::Suse => write!(f, "suse"),
            DistroFamily::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DistroInfo {
    /// The `ID` field from /etc/os-release (e.g. "ubuntu", "fedora").
    pub id: String,
    pub family: DistroFamily,
}

impl DistroInfo {
    pub fn unknown() -> Self {
        Self {
            id: "unknown".to_string(),
            family: DistroFamily::Unknown,
        }
    }
}

// ---------------------------------------------------------------------------
// Tier 2: Built-in defaults
// ---------------------------------------------------------------------------

static DEFAULT_FAMILY_MAP: phf::Map<&'static str, DistroFamily> = phf_map! {
    "debian" => DistroFamily::Debian,
    "ubuntu" => DistroFamily::Debian,
    "linuxmint" => DistroFamily::Debian,
    "pop" => DistroFamily::Debian,
    "elementary" => DistroFamily::Debian,
    "kali" => DistroFamily::Debian,
    "raspbian" => DistroFamily::Debian,
    "fedora" => DistroFamily::RedHat,
    "rhel" => DistroFamily::RedHat,
    "centos" => DistroFamily::RedHat,
    "almalinux" => DistroFamily::RedHat,
    "rocky" => DistroFamily::RedHat,
    "ol" => DistroFamily::RedHat,
    "amzn" => DistroFamily::RedHat,
    "arch" => DistroFamily::Arch,
    "manjaro" => DistroFamily::Arch,
    "endeavouros" => DistroFamily::Arch,
    "garuda" => DistroFamily::Arch,
    "alpine" => DistroFamily::Alpine,
    "opensuse" => DistroFamily::Suse,
    "opensuse-leap" => DistroFamily::Suse,
    "opensuse-tumbleweed" => DistroFamily::Suse,
    "sles" => DistroFamily::Suse,
};

// ---------------------------------------------------------------------------
// Tier 1: Standalone user config file
// ---------------------------------------------------------------------------

/// Path to the standalone distro-family mapping file.
///
/// Env var `GREL_DISTRO_FAMILY_MAP` overrides the default path.
fn user_map_path() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("GREL_DISTRO_FAMILY_MAP") {
        return std::path::PathBuf::from(path);
    }

    dirs::config_dir()
        .map(|p| p.join("grel").join("distro-family-map.toml"))
        .unwrap_or_else(|| {
            std::path::PathBuf::from(".").join("distro-family-map.toml")
        })
}

fn load_user_family_map() -> HashMap<String, DistroFamily> {
    let path = user_map_path();
    if !path.exists() {
        return HashMap::new();
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to read distro-family-map at {}: {}",
                path.display(),
                e
            );
            return HashMap::new();
        }
    };

    let raw: HashMap<String, String> = match toml::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                "Failed to parse distro-family-map at {}: {}",
                path.display(),
                e
            );
            return HashMap::new();
        }
    };

    let mut map = HashMap::new();
    for (distro_id, family_str) in raw {
        match family_str.parse::<DistroFamily>() {
            Ok(family) => {
                map.insert(distro_id, family);
            }
            Err(e) => {
                tracing::warn!(
                    "Ignoring invalid family '{}' for distro '{}' in {}: {}",
                    family_str,
                    distro_id,
                    path.display(),
                    e
                );
            }
        }
    }

    map
}

fn user_family_map() -> &'static HashMap<String, DistroFamily> {
    static MAP: OnceLock<HashMap<String, DistroFamily>> = OnceLock::new();
    MAP.get_or_init(load_user_family_map)
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect the current Linux distro from /etc/os-release.
pub fn detect(override_id: Option<&str>) -> DistroInfo {
    let id = if let Some(ov) = override_id {
        ov.to_string()
    } else {
        read_os_release_id().unwrap_or_default()
    };

    let family = classify_family(&id);
    DistroInfo { id, family }
}

fn classify_family(id: &str) -> DistroFamily {
    // Tier 1: standalone user config file
    if let Some(f) = user_family_map().get(id) {
        return *f;
    }

    // Tier 2: built-in defaults
    if let Some(f) = DEFAULT_FAMILY_MAP.get(id) {
        return *f;
    }

    DistroFamily::Unknown
}

fn read_os_release_id() -> Option<String> {
    let content = std::fs::read_to_string("/etc/os-release").ok()?;
    parse_os_release_field(&content, "ID")
}

fn parse_os_release_field(content: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}=");
    for line in content.lines() {
        if line.starts_with(&prefix) {
            let value = line[prefix.len()..].trim().trim_matches('"');
            return Some(value.to_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // DistroFamily FromStr / Display round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn family_from_str_debian() {
        assert_eq!("debian".parse::<DistroFamily>(), Ok(DistroFamily::Debian));
    }

    #[test]
    fn family_from_str_redhat_aliases() {
        assert_eq!("redhat".parse::<DistroFamily>(), Ok(DistroFamily::RedHat));
        assert_eq!("rhel".parse::<DistroFamily>(), Ok(DistroFamily::RedHat));
        assert_eq!("fedora".parse::<DistroFamily>(), Ok(DistroFamily::RedHat));
    }

    #[test]
    fn family_from_str_unknown() {
        assert_eq!(
            "unknown".parse::<DistroFamily>(),
            Ok(DistroFamily::Unknown)
        );
    }

    #[test]
    fn family_from_str_invalid() {
        assert!("not_a_family".parse::<DistroFamily>().is_err());
    }

    #[test]
    fn family_display_roundtrip() {
        for family in [
            DistroFamily::Debian,
            DistroFamily::RedHat,
            DistroFamily::Arch,
            DistroFamily::Alpine,
            DistroFamily::Suse,
            DistroFamily::Unknown,
        ] {
            let s = family.to_string();
            assert_eq!(s.parse::<DistroFamily>(), Ok(family));
        }
    }

    // -----------------------------------------------------------------------
    // classify_family: built-in defaults (Tier 2)
    // -----------------------------------------------------------------------

    #[test]
    fn classify_debian() {
        assert_eq!(classify_family("ubuntu"), DistroFamily::Debian);
        assert_eq!(classify_family("debian"), DistroFamily::Debian);
    }

    #[test]
    fn classify_arch() {
        assert_eq!(classify_family("arch"), DistroFamily::Arch);
    }

    #[test]
    fn classify_unknown_builtin() {
        assert_eq!(classify_family("gentoo"), DistroFamily::Unknown);
    }

    #[test]
    fn classify_redhat_family() {
        assert_eq!(classify_family("fedora"), DistroFamily::RedHat);
        assert_eq!(classify_family("centos"), DistroFamily::RedHat);
        assert_eq!(classify_family("almalinux"), DistroFamily::RedHat);
        assert_eq!(classify_family("rocky"), DistroFamily::RedHat);
        assert_eq!(classify_family("rhel"), DistroFamily::RedHat);
    }

    #[test]
    fn classify_alpine() {
        assert_eq!(classify_family("alpine"), DistroFamily::Alpine);
    }

    #[test]
    fn classify_suse_family() {
        assert_eq!(classify_family("opensuse"), DistroFamily::Suse);
        assert_eq!(classify_family("opensuse-leap"), DistroFamily::Suse);
        assert_eq!(classify_family("opensuse-tumbleweed"), DistroFamily::Suse);
        assert_eq!(classify_family("sles"), DistroFamily::Suse);
    }

    #[test]
    fn classify_debian_variants() {
        assert_eq!(classify_family("linuxmint"), DistroFamily::Debian);
        assert_eq!(classify_family("kali"), DistroFamily::Debian);
        assert_eq!(classify_family("pop"), DistroFamily::Debian);
        assert_eq!(classify_family("raspbian"), DistroFamily::Debian);
    }

    #[test]
    fn classify_arch_variants() {
        assert_eq!(classify_family("manjaro"), DistroFamily::Arch);
        assert_eq!(classify_family("endeavouros"), DistroFamily::Arch);
        assert_eq!(classify_family("garuda"), DistroFamily::Arch);
    }

    // -----------------------------------------------------------------------
    // parse_os_release_field
    // -----------------------------------------------------------------------

    #[test]
    fn parse_field() {
        let content = "ID=ubuntu\nVERSION_ID=\"22.04\"\n";
        assert_eq!(parse_os_release_field(content, "ID"), Some("ubuntu".into()));
        assert_eq!(
            parse_os_release_field(content, "VERSION_ID"),
            Some("22.04".into())
        );
    }

    #[test]
    fn parse_field_returns_none_for_missing_field() {
        let content = "ID=ubuntu\nVERSION_ID=\"22.04\"\n";
        assert_eq!(parse_os_release_field(content, "NAME"), None);
    }

    #[test]
    fn parse_field_lowercases_value() {
        let content = "ID=Ubuntu\n";
        assert_eq!(parse_os_release_field(content, "ID"), Some("ubuntu".into()));
    }

    // -----------------------------------------------------------------------
    // detect() with override
    // -----------------------------------------------------------------------

    #[test]
    fn detect_with_known_override() {
        let info = detect(Some("fedora"));
        assert_eq!(info.id, "fedora");
        assert_eq!(info.family, DistroFamily::RedHat);
    }

    #[test]
    fn detect_with_unknown_override() {
        let info = detect(Some("gentoo"));
        assert_eq!(info.id, "gentoo");
        assert_eq!(info.family, DistroFamily::Unknown);
    }

    #[test]
    fn distro_info_unknown_constructor() {
        let info = DistroInfo::unknown();
        assert_eq!(info.id, "unknown");
        assert_eq!(info.family, DistroFamily::Unknown);
    }
}
