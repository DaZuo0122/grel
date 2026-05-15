//! Deterministic asset resolution pipeline.
//!
//! Strict filtering → priority sorting → policy fallback.
//! No scoring, no fuzzy logic.

use crate::asset::AssetTokens;
use crate::platform::{Arch, Os};

/// Selection policy for tie-breaking
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionPolicy {
    First,
    Largest,
}

impl Default for SelectionPolicy {
    fn default() -> Self {
        Self::First
    }
}

/// Configuration for asset resolution
#[derive(Debug, Clone)]
pub struct ResolverConfig {
    pub default_selection_policy: SelectionPolicy,
    pub exclude_keywords: Vec<String>,
    pub ignore_formats: Vec<String>,
    pub prefer_formats: Vec<String>,
    pub prefer_32bit_on_64bit: bool,
    pub fallback_to_32bit: bool,
    pub prefer_musl: bool,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            default_selection_policy: SelectionPolicy::First,
            exclude_keywords: vec![
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
            ],
            ignore_formats: vec![
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
            ],
            prefer_formats: vec![
                "*.tar.gz".into(),
                "*.tar.xz".into(),
                "*.zip".into(),
                "*.exe".into(),
            ],
            prefer_32bit_on_64bit: false,
            fallback_to_32bit: true,
            prefer_musl: false,
        }
    }
}

/// A remote asset from a release
#[derive(Debug, Clone)]
pub struct RemoteAsset {
    pub filename: String,
    pub url: String,
    pub size_bytes: Option<u64>,
    pub tokens: AssetTokens,
}

/// Resolved asset selection
#[derive(Debug, Clone)]
pub enum SelectionResult {
    /// No compatible assets found
    NoCompatibleAssets,

    /// Single asset selected
    SingleAsset(RemoteAsset),

    /// Multiple assets tied, need user choice
    MultipleAssets(Vec<RemoteAsset>),
}

/// Full resolution result including the default selection and all
/// compatible alternatives (for display/selection).
#[derive(Debug, Clone)]
pub struct AssetSelection {
    /// The top-ranked asset (the one that would be auto-selected)
    pub default: RemoteAsset,
    /// All other compatible assets, sorted by priority
    pub alternatives: Vec<RemoteAsset>,
    /// Whether the default asset is unmanaged (matches exclude_keywords or ignore_formats)
    pub default_is_managed: bool,
}

/// Resolve assets to a selection
pub fn resolve_assets(
    assets: &[RemoteAsset],
    target_os: &Os,
    target_arch: &Arch,
    config: &ResolverConfig,
    allow_keyword: bool,
) -> SelectionResult {
    // Step 1: Strict filtering
    let filtered: Vec<RemoteAsset> = assets
        .iter()
        .filter(|a| {
            // OS match or Unknown
            a.tokens.os == *target_os || matches!(a.tokens.os, Os::Unknown(_))
        })
        .filter(|a| {
            // Architecture match with fallback logic
            arch_matches(
                &a.tokens.arch,
                target_arch,
                config.fallback_to_32bit,
                config.prefer_32bit_on_64bit,
            )
        })
        .filter(|a| {
            // Keyword exclusion (unless overridden)
            allow_keyword || !is_keyword_excluded(&a.tokens.filename, &config.exclude_keywords)
        })
        .filter(|a| {
            // Format exclusion
            !is_format_ignored(&a.tokens.format, &config.ignore_formats)
        })
        .cloned()
        .collect();

    if filtered.is_empty() {
        return SelectionResult::NoCompatibleAssets;
    }

    // Step 2: Deterministic sorting
    let mut sorted = filtered;
    sorted.sort_by(|a, b| {
        // 1. arch_priority index
        let arch_a = arch_priority_index(&a.tokens.arch, target_arch);
        let arch_b = arch_priority_index(&b.tokens.arch, target_arch);
        arch_a
            .cmp(&arch_b)
            // 2. prefer_formats index
            .then(
                format_priority_index(&a.tokens.format, &config.prefer_formats).cmp(
                    &format_priority_index(&b.tokens.format, &config.prefer_formats),
                ),
            )
            // 3. Lexicographic filename
            .then(a.filename.cmp(&b.filename))
            // 4. Size descending
            .then(b.size_bytes.unwrap_or(0).cmp(&a.size_bytes.unwrap_or(0)))
    });

    // Step 3: Policy fallback
    match sorted.len() {
        0 => SelectionResult::NoCompatibleAssets,
        1 => SelectionResult::SingleAsset(sorted.remove(0)),
        _ => {
            // Check if there's a clear winner after sorting
            match &config.default_selection_policy {
                SelectionPolicy::First => {
                    SelectionResult::SingleAsset(sorted.remove(0))
                }
                SelectionPolicy::Largest => {
                    // Pick by size_bytes descending
                    sorted.sort_by_key(|a| std::cmp::Reverse(a.size_bytes.unwrap_or(0)));
                    SelectionResult::SingleAsset(sorted.remove(0))
                }
            }
        }
    }
}

/// Check if architecture matches target with fallback logic
fn arch_matches(
    asset_arch: &Arch,
    target_arch: &Arch,
    fallback_to_32bit: bool,
    _prefer_32bit_on_64bit: bool,
) -> bool {
    // Exact match
    if asset_arch == target_arch {
        return true;
    }

    // Unknown matches everything
    if matches!(asset_arch, Arch::Unknown(_)) {
        return true;
    }

    // Fallback to 32-bit on 64-bit systems
    if fallback_to_32bit && target_arch.is_64bit() {
        if target_arch == &Arch::X86_64 && asset_arch == &Arch::I686 {
            return true;
        }
        if target_arch == &Arch::Aarch64 && asset_arch == &Arch::ArmV7 {
            return true;
        }
    }

    false
}

/// Get architecture priority index (lower is better)
fn arch_priority_index(arch: &Arch, _target: &Arch) -> usize {
    match arch {
        // Exact matches would have been filtered already
        // Prefer 64-bit over 32-bit
        Arch::X86_64 | Arch::Aarch64 => 0,
        Arch::I686 | Arch::ArmV7 => 1,
        Arch::Unknown(_) => 2,
        _ => 3,
    }
}

/// Get format priority index (lower is better)
fn format_priority_index(format: &str, prefer_formats: &[String]) -> usize {
    prefer_formats
        .iter()
        .position(|p| format_matches(format, p))
        .unwrap_or(usize::MAX)
}

/// Check if format matches a pattern (supports *.ext)
fn format_matches(format: &str, pattern: &str) -> bool {
    if let Some(ext) = pattern.strip_prefix("*.") {
        return format == ext;
    }
    format == pattern
}

/// Check if filename matches any excluded keyword
fn is_keyword_excluded(filename: &str, keywords: &[String]) -> bool {
    let lower = filename.to_lowercase();
    keywords.iter().any(|k| lower.contains(&k.to_lowercase()))
}

/// Filter and sort assets, returning the full selection info for
/// interactive display. Returns `None` if no compatible assets exist.
pub fn resolve_assets_detailed(
    assets: &[RemoteAsset],
    target_os: &Os,
    target_arch: &Arch,
    config: &ResolverConfig,
    allow_keyword: bool,
) -> Option<AssetSelection> {
    let filtered: Vec<RemoteAsset> = assets
        .iter()
        .filter(|a| a.tokens.os == *target_os || matches!(a.tokens.os, Os::Unknown(_)))
        .filter(|a| {
            arch_matches(
                &a.tokens.arch,
                target_arch,
                config.fallback_to_32bit,
                config.prefer_32bit_on_64bit,
            )
        })
        .filter(|a| {
            allow_keyword || !is_keyword_excluded(&a.tokens.filename, &config.exclude_keywords)
        })
        .filter(|a| !is_format_ignored(&a.tokens.format, &config.ignore_formats))
        .cloned()
        .collect();

    if filtered.is_empty() {
        return None;
    }

    let mut sorted = filtered;
    sorted.sort_by(|a, b| {
        let arch_a = arch_priority_index(&a.tokens.arch, target_arch);
        let arch_b = arch_priority_index(&b.tokens.arch, target_arch);
        arch_a
            .cmp(&arch_b)
            .then(
                format_priority_index(&a.tokens.format, &config.prefer_formats).cmp(
                    &format_priority_index(&b.tokens.format, &config.prefer_formats),
                ),
            )
            .then(a.filename.cmp(&b.filename))
            .then(b.size_bytes.unwrap_or(0).cmp(&a.size_bytes.unwrap_or(0)))
    });

    if sorted.len() == 1 {
        return Some(AssetSelection {
            default: sorted.remove(0),
            alternatives: vec![],
            default_is_managed: false, // will be determined by caller
        });
    }

    // Apply selection policy
    let (default, alternatives) = match &config.default_selection_policy {
        SelectionPolicy::First => {
            let default = sorted.remove(0);
            (default, sorted)
        }
        SelectionPolicy::Largest => {
            sorted.sort_by_key(|a| std::cmp::Reverse(a.size_bytes.unwrap_or(0)));
            let default = sorted.remove(0);
            (default, sorted)
        }
    };

    Some(AssetSelection {
        default,
        alternatives,
        default_is_managed: false,
    })
}

/// Check if format is in ignore list
fn is_format_ignored(format: &str, ignore_formats: &[String]) -> bool {
    ignore_formats.iter().any(|p| format_matches(format, p))
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_exclusion() {
        assert!(is_keyword_excluded("tool-setup-1.0.exe", &["setup".into()]));
        assert!(!is_keyword_excluded(
            "tool-1.0-linux.tar.gz",
            &["setup".into()]
        ));
    }

    #[test]
    fn test_format_matching() {
        assert!(format_matches("tar.gz", "*.tar.gz"));
        assert!(format_matches("exe", "*.exe"));
        assert!(!format_matches("deb", "*.tar.gz"));
    }

    #[test]
    fn test_format_priority() {
        let prefers = vec!["*.tar.gz".into(), "*.zip".into(), "*.exe".into()];
        assert_eq!(format_priority_index("tar.gz", &prefers), 0);
        assert_eq!(format_priority_index("zip", &prefers), 1);
        assert_eq!(format_priority_index("exe", &prefers), 2);
        assert_eq!(format_priority_index("deb", &prefers), usize::MAX);
    }

    #[test]
    fn test_arch_fallback() {
        // x86_64 target can fallback to i686
        assert!(arch_matches(&Arch::I686, &Arch::X86_64, true, false));

        // Without fallback, should not match
        assert!(!arch_matches(&Arch::I686, &Arch::X86_64, false, false));
    }

    fn test_config() -> ResolverConfig {
        ResolverConfig::default()
    }
}
