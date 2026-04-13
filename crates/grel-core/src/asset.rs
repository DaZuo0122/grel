//! Asset tokenization and extraction from filenames.

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::platform::{Arch, Os};

/// Tokenized representation of an asset filename
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetTokens {
    /// Original filename
    pub filename: String,

    /// Parsed OS
    pub os: Os,

    /// Parsed architecture
    pub arch: Arch,

    /// File format/extension
    pub format: String,

    /// Version string (from filename or release tag)
    pub version: Option<String>,

    /// Whether it's a musl build
    pub is_musl: bool,

    /// Whether it's a static build
    pub is_static: bool,
}

impl AssetTokens {
    /// Parse an asset filename into tokens
    pub fn from_filename(filename: &str) -> Self {
        Self::from_filename_with_tag(filename, None)
    }

    /// Parse an asset filename, with an optional release tag for version fallback
    pub fn from_filename_with_tag(filename: &str, release_tag: Option<&str>) -> Self {
        let filename_lower = filename.to_lowercase();
        let basename = filename_lower
            .rsplit('/')
            .next()
            .unwrap_or(&filename_lower)
            .to_string();

        let os = detect_os(&basename);
        let arch = detect_arch(&basename);
        let format = detect_format(&basename);

        // Try filename first, fall back to release tag for version
        let version = detect_version(&basename)
            .or_else(|| release_tag.and_then(detect_version_from_tag));

        let is_musl = basename.contains("musl");
        let is_static = basename.contains("static");

        Self {
            filename: filename.to_string(),
            os,
            arch,
            format,
            version,
            is_musl,
            is_static,
        }
    }

    /// Get file extension(s) for matching
    pub fn extensions(&self) -> Vec<String> {
        // Handle compound extensions like .tar.gz
        if self.format == "tar.gz" || self.format == "tar.xz" || self.format == "tar.bz2" {
            return vec![format!(".{}", self.format)];
        }
        vec![format!(".{}", self.format)]
    }
}

// ---------------------------------------------------------------------------
// OS detection – ordered from most specific to least specific to avoid
// false positives (e.g. "win" must come before "windows" is unnecessary
// since both map to Windows, but "darwin" must come before a hypothetical
// generic "apple" catch-all).
// ---------------------------------------------------------------------------

/// OS keyword groups, checked in order.  The first matching group wins.
/// Order matters: more specific patterns (darwin) must come before patterns
/// that could match as substrings (win inside darwin).
const OS_KEYWORDS: &[(&[&str], Os)] = &[
    // macOS family – checked before Windows because "darwin" contains "win"
    (
        &["macos", "darwin", "apple", "osx", "mac"],
        Os::MacOS,
    ),
    // Linux family
    (
        &["linux", "ubuntu", "debian", "alpine", "fedora", "rhel", "centos", "suse", "arch"],
        Os::Linux,
    ),
    // Windows family – "msvc" and "mingw" are ABI markers that imply Windows
    (
        &["windows", "win64", "win32", "mingw64", "mingw32", "msvc", "win"],
        Os::Windows,
    ),
    // BSD
    (&["freebsd", "openbsd", "netbsd"], Os::FreeBSD),
    // Mobile
    (&["android"], Os::Android),
    (&["ios", "iphone", "ipad"], Os::iOS),
];

/// Detect OS from filename using ordered keyword groups.
fn detect_os(filename: &str) -> Os {
    let lower = filename.to_lowercase();
    for (keywords, os) in OS_KEYWORDS {
        for &kw in *keywords {
            if lower.contains(kw) {
                return os.clone();
            }
        }
    }
    Os::Unknown(filename.to_string())
}

// ---------------------------------------------------------------------------
// Architecture detection
// ---------------------------------------------------------------------------

const ARCH_KEYWORDS: &[(&[&str], Arch)] = &[
    (&["x86_64", "x86-64", "amd64", "x64"], Arch::X86_64),
    (&["aarch64", "arm64"], Arch::Aarch64),
    (&["i686", "i386", "x86", "386"], Arch::I686),
    (&["armv7", "armv7l", "arm32"], Arch::ArmV7),
    (&["armv6", "armhf"], Arch::ArmV6),
    (&["riscv64", "riscv"], Arch::Riscv64),
    (&["s390x"], Arch::S390x),
    (&["ppc64le", "ppc64", "powerpc64"], Arch::PowerPC64),
];

fn detect_arch(filename: &str) -> Arch {
    let lower = filename.to_lowercase();
    for (keywords, arch) in ARCH_KEYWORDS {
        for &kw in *keywords {
            if lower.contains(kw) {
                return arch.clone();
            }
        }
    }
    Arch::Unknown(filename.to_string())
}

// ---------------------------------------------------------------------------
// Format detection
// ---------------------------------------------------------------------------

fn detect_format(filename: &str) -> String {
    // Compound extensions first
    let compound = [
        (".tar.gz", "tar.gz"),
        (".tgz", "tar.gz"),
        (".tar.xz", "tar.xz"),
        (".tar.bz2", "tar.bz2"),
        (".tar.zst", "tar.zst"),
    ];
    for (ext, fmt) in compound {
        if filename.ends_with(ext) {
            return fmt.into();
        }
    }

    // Simple extensions
    let extensions = [
        "exe", "zip", "deb", "rpm", "msi", "dmg", "pkg", "appimage",
        "gz", "xz", "bz2", "zst", "tar", "7z",
    ];
    for ext in extensions {
        if filename.ends_with(&format!(".{ext}")) {
            return ext.to_string();
        }
    }

    "unknown".into()
}

// ---------------------------------------------------------------------------
// Version detection
// ---------------------------------------------------------------------------

/// Patterns for extracting version from filenames.
fn detect_version(filename: &str) -> Option<String> {
    // Pre-release identifiers: alpha, beta, rc, pre, dev, snapshot, etc.
    // We restrict the suffix to known pre-release markers to avoid capturing
    // platform names like "linux", "windows", etc.
    let patterns = [
        // v1.2.3 or v1.2.3-beta.1 etc.
        r"[vV](\d+\.\d+\.\d+(?:-(?:alpha|beta|rc|pre|dev|snapshot|nightly|edge)\.?\d*(?:\.\d+)*)?)(?:[-_./]|$)",
        // 1.2.3 or 1.2.3-rc.1 etc.
        r"[-_](\d+\.\d+\.\d+(?:-(?:alpha|beta|rc|pre|dev|snapshot|nightly|edge)\.?\d*(?:\.\d+)*)?)(?:[-_./]|$)",
    ];

    for pattern in &patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(caps) = re.captures(filename) {
                if let Some(m) = caps.get(1) {
                    return Some(m.as_str().to_string());
                }
            }
        }
    }

    None
}

/// Detect version from a release tag (e.g. "v1.2.3", "1.2.3-beta.1").
/// This is the primary source for version when filenames don't embed it.
pub fn detect_version_from_tag(tag: &str) -> Option<String> {
    // Strip leading 'v' or 'V' if present
    let stripped = tag.strip_prefix(['v', 'V']).unwrap_or(tag);

    // Match semver-like patterns: 1.2.3, 1.2.3-beta.1, 1.2.3+build
    let pattern = r"^(\d+\.\d+\.\d+(?:[-+][\w.]+)?)";
    if let Ok(re) = Regex::new(pattern) {
        if let Some(caps) = re.captures(stripped) {
            if let Some(m) = caps.get(1) {
                return Some(m.as_str().to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_tokenization() {
        let tokens = AssetTokens::from_filename("mytool-v1.2.3-linux-x86_64.tar.gz");
        assert_eq!(tokens.os, Os::Linux);
        assert_eq!(tokens.arch, Arch::X86_64);
        assert_eq!(tokens.format, "tar.gz");
        assert_eq!(tokens.version, Some("1.2.3".into()));
        assert!(!tokens.is_musl);
    }

    #[test]
    fn test_version_from_tag() {
        // Filename has no version, tag provides it
        let tokens = AssetTokens::from_filename_with_tag(
            "tool-linux-x86_64.tar.gz",
            Some("v2.0.0"),
        );
        assert_eq!(tokens.version, Some("2.0.0".into()));
    }

    #[test]
    fn test_version_from_tag_with_prerelease() {
        let tokens = AssetTokens::from_filename_with_tag(
            "tool-linux-x86_64.tar.gz",
            Some("v1.0.0-beta.1"),
        );
        assert_eq!(tokens.version, Some("1.0.0-beta.1".into()));
    }

    #[test]
    fn test_filename_version_takes_priority() {
        // Filename has v3.0.0, tag has v2.0.0 → filename wins
        let tokens = AssetTokens::from_filename_with_tag(
            "tool-v3.0.0-linux-x86_64.tar.gz",
            Some("v2.0.0"),
        );
        assert_eq!(tokens.version, Some("3.0.0".into()));
    }

    #[test]
    fn test_musl_detection() {
        let tokens = AssetTokens::from_filename("tool-v1.0-x86_64-linux-musl.tar.gz");
        assert!(tokens.is_musl);
    }

    #[test]
    fn test_windows_detection() {
        let tokens = AssetTokens::from_filename("tool-1.0-windows-amd64.exe");
        assert_eq!(tokens.os, Os::Windows);
        assert_eq!(tokens.arch, Arch::X86_64);
        assert_eq!(tokens.format, "exe");
    }

    #[test]
    fn test_os_detection_mingw() {
        let tokens = AssetTokens::from_filename("tool-v1.0-x86_64-mingw64.zip");
        assert_eq!(tokens.os, Os::Windows);
    }

    #[test]
    fn test_os_detection_darwin() {
        let tokens = AssetTokens::from_filename("tool-v1.0-darwin-arm64.tar.gz");
        assert_eq!(tokens.os, Os::MacOS);
    }
}
