//! Distro detection via /etc/os-release.

/// Known distro families for package manager selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistroFamily {
    Debian,
    RedHat,
    Arch,
    Alpine,
    Suse,
    Unknown,
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

fn classify_family(id: &str) -> DistroFamily {
    match id {
        "debian" | "ubuntu" | "linuxmint" | "pop" | "elementary" | "kali" | "raspbian" => {
            DistroFamily::Debian
        }
        "fedora" | "rhel" | "centos" | "almalinux" | "rocky" | "ol" | "amzn" => {
            DistroFamily::RedHat
        }
        "arch" | "manjaro" | "endeavouros" | "garuda" => DistroFamily::Arch,
        "alpine" => DistroFamily::Alpine,
        "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" => DistroFamily::Suse,
        _ => DistroFamily::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn classify_unknown() {
        assert_eq!(classify_family("gentoo"), DistroFamily::Unknown);
    }

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
    fn parse_field_returns_none_for_missing_field() {
        let content = "ID=ubuntu\nVERSION_ID=\"22.04\"\n";
        assert_eq!(parse_os_release_field(content, "NAME"), None);
    }

    #[test]
    fn parse_field_lowercases_value() {
        // /etc/os-release sometimes has mixed-case IDs; we normalize to lowercase
        let content = "ID=Ubuntu\n";
        assert_eq!(parse_os_release_field(content, "ID"), Some("ubuntu".into()));
    }

    #[test]
    fn distro_info_unknown_constructor() {
        let info = DistroInfo::unknown();
        assert_eq!(info.id, "unknown");
        assert_eq!(info.family, DistroFamily::Unknown);
    }
}
