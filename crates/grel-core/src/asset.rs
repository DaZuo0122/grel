//! Asset tokenization and extraction from filenames.
//!
//! ## Strategy
//!
//! Tokenization works in four steps:
//!
//! 1. **Normalise** – lower-case the basename and rewrite known hyphenated
//!    arch variants (e.g. `x86-64` → `x86_64`) that would otherwise split
//!    into meaningless fragments.
//! 2. **Strip extension** – detect the archive/binary format and remove it,
//!    leaving a clean "stem" (`ripgrep-14.1.1-x86_64-unknown-linux-musl`).
//! 3. **Tokenise** – split the stem on `-` and `_`, producing a list of
//!    lowercase tokens (`["ripgrep", "14.1.1", "x86_64", "unknown", "linux",
//!    "musl"]`).
//! 4. **Match** – compare each token exactly against OS, architecture, and
//!    flag tables.  Version tokens are identified via `semver::Version::parse`
//!    with a two-part (X.Y) fallback.
//!
//! Exact token matching avoids the false positives that substring matching
//! produces (e.g. `"win"` inside `"darwin"`, `"x86"` inside `"x86_64"`).

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::platform::{Arch, Os};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Tokenized representation of an asset filename
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetTokens {
    /// Original filename (case-preserved)
    pub filename: String,

    /// Parsed OS
    pub os: Os,

    /// Parsed architecture
    pub arch: Arch,

    /// File format / extension (`"tar.gz"`, `"zip"`, `"exe"`, …)
    pub format: String,

    /// Version string (from filename tokens or release tag fallback)
    pub version: Option<String>,

    /// Whether it's a musl-libc build
    pub is_musl: bool,

    /// Whether it's a statically linked build
    pub is_static: bool,
}

impl AssetTokens {
    /// Parse an asset filename into tokens.
    pub fn from_filename(filename: &str) -> Self {
        Self::from_filename_with_tag(filename, None)
    }

    /// Parse an asset filename, with an optional release tag for version
    /// fallback when the filename itself does not embed one.
    pub fn from_filename_with_tag(filename: &str, release_tag: Option<&str>) -> Self {
        // Strip any leading path component.
        let basename = filename.rsplit('/').next().unwrap_or(filename);

        // Lower-case + fix known hyphenated arch spellings before splitting.
        let lower = normalize(basename);

        let format = detect_format(&lower);
        let stem = strip_extension(&lower, &format);

        // Split on `-` and `_`; skip empty segments.
        let tokens: Vec<&str> = stem
            .split(|c| c == '-' || c == '_')
            .filter(|t| !t.is_empty())
            .collect();

        let mut os: Option<Os> = None;
        let mut arch: Option<Arch> = None;
        let mut version: Option<String> = None;
        let mut is_musl = false;
        let mut is_static = false;

        for token in &tokens {
            if os.is_none() {
                if let Some(detected) = match_os_token(token) {
                    os = Some(detected);
                }
            }
            if arch.is_none() {
                if let Some(detected) = match_arch_token(token) {
                    arch = Some(detected);
                }
            }
            if version.is_none() {
                if let Some(v) = try_parse_version(token) {
                    version = Some(v);
                }
            }
            // musl: exact token OR token that starts with "musl" (e.g. "musleabihf")
            if token.starts_with("musl") {
                is_musl = true;
            }
            if *token == "static" {
                is_static = true;
            }
        }

        // Version fallback: try the release tag when the filename has none.
        if version.is_none() {
            version = release_tag.and_then(detect_version_from_tag);
        }

        Self {
            filename: filename.to_string(),
            os: os.unwrap_or_else(|| Os::Unknown(lower.clone())),
            arch: arch.unwrap_or_else(|| Arch::Unknown(lower.clone())),
            format,
            version,
            is_musl,
            is_static,
        }
    }

    /// Return the file extension(s) that represent this asset's format.
    pub fn extensions(&self) -> Vec<String> {
        vec![format!(".{}", self.format)]
    }
}

// ---------------------------------------------------------------------------
// Normalisation
// ---------------------------------------------------------------------------

/// Lower-case `s` and rewrite arch spellings that would be incorrectly split
/// by the `-`/`_` tokeniser.
///
/// Both `x86-64` (hyphen) and `x86_64` (underscore) are rewritten to `amd64`
/// before splitting, so the underscore in `x86_64` never breaks it into
/// `["x86", "64"]`, which would be misidentified as i686.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .replace("x86-64", "amd64")
        .replace("x86_64", "amd64")
}

// ---------------------------------------------------------------------------
// Extension stripping
// ---------------------------------------------------------------------------

/// Remove the detected format extension from a lowercased basename,
/// returning the stem.
fn strip_extension(basename: &str, format: &str) -> String {
    match format {
        "tar.gz" => basename
            .strip_suffix(".tar.gz")
            .or_else(|| basename.strip_suffix(".tgz"))
            .unwrap_or(basename)
            .to_string(),
        "tar.xz" => basename.strip_suffix(".tar.xz").unwrap_or(basename).to_string(),
        "tar.bz2" => basename.strip_suffix(".tar.bz2").unwrap_or(basename).to_string(),
        "tar.zst" => basename.strip_suffix(".tar.zst").unwrap_or(basename).to_string(),
        other if other != "unknown" => {
            let ext = format!(".{other}");
            basename.strip_suffix(&ext).unwrap_or(basename).to_string()
        }
        _ => basename.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Format detection
// ---------------------------------------------------------------------------

fn detect_format(filename: &str) -> String {
    // Compound extensions must be checked before simple ones.
    const COMPOUND: &[(&str, &str)] = &[
        (".tar.gz", "tar.gz"),
        (".tgz", "tar.gz"),
        (".tar.xz", "tar.xz"),
        (".tar.bz2", "tar.bz2"),
        (".tar.zst", "tar.zst"),
    ];
    for (ext, fmt) in COMPOUND {
        if filename.ends_with(ext) {
            return (*fmt).to_string();
        }
    }

    const SIMPLE: &[&str] = &[
        "exe", "zip", "deb", "rpm", "msi", "dmg", "pkg", "appimage", "gz", "xz", "bz2", "zst",
        "tar", "7z",
    ];
    for ext in SIMPLE {
        if filename.ends_with(&format!(".{ext}")) {
            return (*ext).to_string();
        }
    }

    "unknown".into()
}

// ---------------------------------------------------------------------------
// OS token matching  (exact, not substring)
// ---------------------------------------------------------------------------

/// Match a single lowercase token to an OS variant.
/// Returns `None` for tokens that are not OS identifiers (vendor tags like
/// `"unknown"` or `"pc"`, ABI tags like `"gnu"`, tool names, etc.).
fn match_os_token(token: &str) -> Option<Os> {
    match token {
        // Linux distributions
        "linux" | "ubuntu" | "debian" | "alpine" | "fedora" | "rhel" | "centos" | "suse"
        | "arch" => Some(Os::Linux),
        // macOS / Darwin
        "darwin" | "macos" | "osx" | "mac" | "apple" => Some(Os::MacOS),
        // Windows and Windows ABI markers
        "windows" | "win64" | "win32" | "win" | "msvc" | "mingw64" | "mingw32" | "mingw" => {
            Some(Os::Windows)
        }
        // BSDs
        "freebsd" | "openbsd" | "netbsd" => Some(Os::FreeBSD),
        // Mobile
        "android" => Some(Os::Android),
        "ios" | "iphone" | "ipad" => Some(Os::iOS),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Architecture token matching  (exact, not substring)
// ---------------------------------------------------------------------------

/// Match a single lowercase token to an Arch variant.
/// Vendor/ABI tokens (`"unknown"`, `"pc"`, `"none"`, `"gnu"`, `"gnueabihf"`,
/// `"elf"`, …) return `None`.
fn match_arch_token(token: &str) -> Option<Arch> {
    match token {
        "x86_64" | "amd64" | "x64" => Some(Arch::X86_64),
        "aarch64" | "arm64" => Some(Arch::Aarch64),
        "i686" | "i386" | "x86" | "386" => Some(Arch::I686),
        // Bare "arm" in release filenames almost always means ARMv7-compatible.
        "arm" | "armv7" | "armv7l" | "arm32" => Some(Arch::ArmV7),
        "armv6" | "armhf" => Some(Arch::ArmV6),
        "riscv64" | "riscv" => Some(Arch::Riscv64),
        "s390x" => Some(Arch::S390x),
        "ppc64le" | "ppc64" | "powerpc64" => Some(Arch::PowerPC64),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Version parsing
// ---------------------------------------------------------------------------

/// Try to interpret a single token as a version string.
///
/// Accepts:
/// - Full semver `1.2.3`, with optional pre-release `1.2.3-beta.1`.
/// - Two-part `X.Y` (e.g. `1.0`).
/// - Any of the above with a leading `v`/`V` prefix.
///
/// Returns the numeric part only (no leading `v`).
fn try_parse_version(token: &str) -> Option<String> {
    let s = token.strip_prefix('v').unwrap_or(token);

    // Full semver (handles pre-release and build metadata).
    if Version::parse(s).is_ok() {
        return Some(s.to_string());
    }

    // Two-part X.Y (not valid semver, but common in release filenames).
    let parts: Vec<&str> = s.splitn(3, '.').collect();
    if parts.len() == 2
        && parts[0].parse::<u64>().is_ok()
        && parts[1].parse::<u64>().is_ok()
    {
        return Some(s.to_string());
    }

    None
}

/// Detect version from a release tag (e.g. `"v1.2.3"`, `"release-1.2.3"`).
///
/// Falls back to splitting the tag on `-`/`_` and trying each segment, which
/// handles unusual tag patterns like `"cli-v1.2.3"` or `"release-14.1.1"`.
pub fn detect_version_from_tag(tag: &str) -> Option<String> {
    let s = tag.strip_prefix(['v', 'V']).unwrap_or(tag);

    // Try the whole stripped tag first.
    if let Some(v) = try_parse_version(s) {
        return Some(v);
    }

    // Tags like "release-1.2.3" or "cli-v2.0.0": try each segment.
    for part in s.split(|c| c == '-' || c == '_') {
        if let Some(v) = try_parse_version(part) {
            return Some(v);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- existing tests (must continue to pass) ----------------------------

    #[test]
    fn test_asset_tokenization() {
        let t = AssetTokens::from_filename("mytool-v1.2.3-linux-x86_64.tar.gz");
        assert_eq!(t.os, Os::Linux);
        assert_eq!(t.arch, Arch::X86_64);
        assert_eq!(t.format, "tar.gz");
        assert_eq!(t.version, Some("1.2.3".into()));
        assert!(!t.is_musl);
    }

    #[test]
    fn test_version_from_tag() {
        let t = AssetTokens::from_filename_with_tag("tool-linux-x86_64.tar.gz", Some("v2.0.0"));
        assert_eq!(t.version, Some("2.0.0".into()));
    }

    #[test]
    fn test_version_from_tag_with_prerelease() {
        let t =
            AssetTokens::from_filename_with_tag("tool-linux-x86_64.tar.gz", Some("v1.0.0-beta.1"));
        assert_eq!(t.version, Some("1.0.0-beta.1".into()));
    }

    #[test]
    fn test_filename_version_takes_priority() {
        let t = AssetTokens::from_filename_with_tag(
            "tool-v3.0.0-linux-x86_64.tar.gz",
            Some("v2.0.0"),
        );
        assert_eq!(t.version, Some("3.0.0".into()));
    }

    #[test]
    fn test_musl_detection() {
        let t = AssetTokens::from_filename("tool-v1.0-x86_64-linux-musl.tar.gz");
        assert!(t.is_musl);
    }

    #[test]
    fn test_windows_detection() {
        let t = AssetTokens::from_filename("tool-1.0-windows-amd64.exe");
        assert_eq!(t.os, Os::Windows);
        assert_eq!(t.arch, Arch::X86_64);
        assert_eq!(t.format, "exe");
    }

    #[test]
    fn test_os_detection_mingw() {
        let t = AssetTokens::from_filename("tool-v1.0-x86_64-mingw64.zip");
        assert_eq!(t.os, Os::Windows);
    }

    #[test]
    fn test_os_detection_darwin() {
        let t = AssetTokens::from_filename("tool-v1.0-darwin-arm64.tar.gz");
        assert_eq!(t.os, Os::MacOS);
        assert_eq!(t.arch, Arch::Aarch64);
    }

    // ---- new: real-world ripgrep release assets ----------------------------

    #[test]
    fn test_ripgrep_linux_musl() {
        // ripgrep 14.x Rust cross-compilation triple style
        let t = AssetTokens::from_filename("ripgrep-14.1.1-x86_64-unknown-linux-musl.tar.gz");
        assert_eq!(t.os, Os::Linux);
        assert_eq!(t.arch, Arch::X86_64);
        assert_eq!(t.format, "tar.gz");
        assert_eq!(t.version, Some("14.1.1".into()));
        assert!(t.is_musl);
        assert!(!t.is_static);
    }

    #[test]
    fn test_ripgrep_linux_gnu() {
        let t = AssetTokens::from_filename("ripgrep-14.1.1-x86_64-unknown-linux-gnu.tar.gz");
        assert_eq!(t.os, Os::Linux);
        assert_eq!(t.arch, Arch::X86_64);
        assert!(!t.is_musl);
    }

    #[test]
    fn test_ripgrep_windows_msvc() {
        let t = AssetTokens::from_filename("ripgrep-14.1.1-x86_64-pc-windows-msvc.zip");
        assert_eq!(t.os, Os::Windows);
        assert_eq!(t.arch, Arch::X86_64);
        assert_eq!(t.format, "zip");
        assert_eq!(t.version, Some("14.1.1".into()));
    }

    #[test]
    fn test_ripgrep_macos_apple_darwin() {
        let t = AssetTokens::from_filename("ripgrep-14.1.1-x86_64-apple-darwin.tar.gz");
        assert_eq!(t.os, Os::MacOS);
        assert_eq!(t.arch, Arch::X86_64);
    }

    #[test]
    fn test_ripgrep_aarch64_linux() {
        let t = AssetTokens::from_filename("ripgrep-14.1.1-aarch64-unknown-linux-gnu.tar.gz");
        assert_eq!(t.arch, Arch::Aarch64);
        assert_eq!(t.os, Os::Linux);
    }

    #[test]
    fn test_ripgrep_arm_linux() {
        // Plain "arm" target (ARMv7 compatible)
        let t = AssetTokens::from_filename("ripgrep-14.1.1-arm-unknown-linux-gnueabihf.tar.gz");
        assert_eq!(t.arch, Arch::ArmV7);
        assert_eq!(t.os, Os::Linux);
        assert!(!t.is_musl); // "gnueabihf" does not start with "musl"
    }

    // ---- new: edge cases ---------------------------------------------------

    #[test]
    fn test_musl_variant_musleabihf() {
        // musleabihf is the musl hard-float ARM ABI token
        let t =
            AssetTokens::from_filename("tool-14.0.0-arm-unknown-linux-musleabihf.tar.gz");
        assert!(t.is_musl, "musleabihf should set is_musl");
        assert_eq!(t.arch, Arch::ArmV7);
    }

    #[test]
    fn test_hyphenated_x86_64_normalised() {
        // "x86-64" (with hyphen) must not be misclassified as i686
        let t = AssetTokens::from_filename("tool-1.0-linux-x86-64.tar.gz");
        assert_eq!(t.arch, Arch::X86_64);
    }

    #[test]
    fn test_tgz_extension() {
        let t = AssetTokens::from_filename("tool-1.2.3-linux-amd64.tgz");
        assert_eq!(t.format, "tar.gz");
        assert_eq!(t.arch, Arch::X86_64);
    }

    #[test]
    fn test_two_part_version() {
        let t = AssetTokens::from_filename("tool-1.0-linux-x86_64.tar.gz");
        assert_eq!(t.version, Some("1.0".into()));
    }

    #[test]
    fn test_tag_with_prefix() {
        // Tag "release-14.1.1" should still yield version "14.1.1"
        let t = AssetTokens::from_filename_with_tag(
            "tool-linux-x86_64.tar.gz",
            Some("release-14.1.1"),
        );
        assert_eq!(t.version, Some("14.1.1".into()));
    }

    #[test]
    fn test_static_flag() {
        let t = AssetTokens::from_filename("tool-1.0.0-x86_64-linux-static.tar.gz");
        assert!(t.is_static);
    }

    #[test]
    fn test_apple_before_darwin() {
        // "apple" should resolve OS before "darwin" is even checked
        let t = AssetTokens::from_filename("tool-aarch64-apple-darwin.tar.gz");
        assert_eq!(t.os, Os::MacOS);
        assert_eq!(t.arch, Arch::Aarch64);
    }
}
