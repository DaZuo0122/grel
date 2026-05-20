//! Checksum file detection and parsing.

use crate::manifest::Manifest;
use crate::resolver::RemoteAsset;

/// Supported checksum algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Sha256,
    Sha512,
    Md5,
}

/// Errors from checksum operations.
#[derive(Debug, thiserror::Error)]
pub enum ChecksumError {
    #[error("No checksum file found for {0}")]
    NotFound(String),

    #[error("Checksum mismatch: expected {expected}, got {computed}")]
    Mismatch { expected: String, computed: String },

    #[error("Parse error: {0}")]
    ParseError(String),
}

/// Locate and parse checksum files for release assets.
pub struct ChecksumVerifier;

impl ChecksumVerifier {
    /// Find the best checksum file for `target_asset` among `assets`.
    ///
    /// Preference order:
    /// 1. `{filename}.sha256` → Sha256
    /// 2. `{filename}.sha512` → Sha512
    /// 3. `{filename}.md5`    → Md5
    /// 4. `SHA256SUMS` / `sha256sums.txt` → Sha256
    /// 5. `SHA512SUMS` / `sha512sums.txt` → Sha512
    /// 6. `MD5SUMS` / `md5sums.txt` → Md5
    ///
    /// If `manifest` specifies `checksum_filename`, that exact pattern is tried first.
    pub fn find_checksum_asset(
        assets: &[RemoteAsset],
        target_asset: &RemoteAsset,
        manifest: Option<&Manifest>,
    ) -> Option<(RemoteAsset, ChecksumAlgorithm)> {
        // If the manifest specifies an exact checksum file, try that first.
        if let Some(manifest) = manifest {
            if let Some(pattern) = &manifest.checksum_filename {
                let candidate = Self::find_by_name(assets, pattern);
                if let Some(asset) = candidate {
                    if let Some(algo) = Self::detect_algorithm(&asset.filename) {
                        return Some((asset, algo));
                    }
                }
            }
        }

        // Per-asset sidecar files.
        let candidates = [
            (format!("{}.sha256", target_asset.filename), ChecksumAlgorithm::Sha256),
            (format!("{}.sha512", target_asset.filename), ChecksumAlgorithm::Sha512),
            (format!("{}.md5", target_asset.filename), ChecksumAlgorithm::Md5),
        ];
        for (name, algo) in &candidates {
            if let Some(asset) = Self::find_by_name(assets, name) {
                return Some((asset, *algo));
            }
        }

        // Global sums files.
        let global_candidates = [
            ("SHA256SUMS", ChecksumAlgorithm::Sha256),
            ("sha256sums.txt", ChecksumAlgorithm::Sha256),
            ("SHA512SUMS", ChecksumAlgorithm::Sha512),
            ("sha512sums.txt", ChecksumAlgorithm::Sha512),
            ("MD5SUMS", ChecksumAlgorithm::Md5),
            ("md5sums.txt", ChecksumAlgorithm::Md5),
        ];
        for (name, algo) in &global_candidates {
            if let Some(asset) = Self::find_by_name(assets, name) {
                return Some((asset, *algo));
            }
        }

        None
    }

    /// Parse a checksum file and extract the hash for `target_filename`.
    ///
    /// Supports three formats:
    /// - **Single-hash**: a file containing exactly one hex string.
    /// - **GNU-style**: `hash  filename` (two spaces, no prefix).
    /// - **Binary-style**: `hash *filename` (space-asterisk, from `sha256sum -b`).
    pub fn parse_checksum(content: &str, target_filename: &str) -> Result<String, ChecksumError> {
        let trimmed = content.trim();

        // If it's a single line with just a hex string, return it directly.
        if !trimmed.contains('\n') && !trimmed.contains(' ') {
            let hex = trimmed.to_lowercase();
            if hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(hex);
            }
        }

        // Multi-line: search for a line matching the target filename.
        let basename = std::path::Path::new(target_filename)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(target_filename);

        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Try binary-style: "<hash> *<filename>"
            if let Some((hash, name)) = line.split_once(" *") {
                let name = name.trim();
                let hash = hash.trim().to_lowercase();
                if name == basename || name == target_filename {
                    return Ok(hash);
                }
            }

            // Try GNU-style: "<hash>  <filename>" (two spaces)
            if let Some((hash, name)) = line.split_once("  ") {
                let name = name.trim();
                let hash = hash.trim().to_lowercase();
                if name == basename || name == target_filename {
                    return Ok(hash);
                }
            }

            // Try single-space fallback: "<hash> <filename>"
            if let Some((hash, name)) = line.split_once(' ') {
                let name = name.trim();
                let hash = hash.trim().to_lowercase();
                if name == basename || name == target_filename {
                    return Ok(hash);
                }
            }
        }

        Err(ChecksumError::ParseError(format!(
            "Could not find checksum for {target_filename}"
        )))
    }

    /// Compare `computed` against `expected`, normalising case.
    pub fn verify(computed: &str, expected: &str) -> Result<(), ChecksumError> {
        if computed.eq_ignore_ascii_case(expected) {
            Ok(())
        } else {
            Err(ChecksumError::Mismatch {
                expected: expected.to_lowercase(),
                computed: computed.to_lowercase(),
            })
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn find_by_name(assets: &[RemoteAsset], name: &str) -> Option<RemoteAsset> {
        assets.iter().find(|a| a.filename == name).cloned()
    }

    fn detect_algorithm(filename: &str) -> Option<ChecksumAlgorithm> {
        let lower = filename.to_lowercase();
        if lower.contains("sha256") {
            Some(ChecksumAlgorithm::Sha256)
        } else if lower.contains("sha512") {
            Some(ChecksumAlgorithm::Sha512)
        } else if lower.contains("md5") {
            Some(ChecksumAlgorithm::Md5)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_asset(filename: &str) -> RemoteAsset {
        RemoteAsset {
            filename: filename.to_string(),
            url: format!("https://example.com/{}", filename),
            size_bytes: None,
            tokens: crate::AssetTokens::from_filename(filename),
        }
    }

    #[test]
    fn find_sidecar_sha256() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![
            target.clone(),
            make_asset("foo.tar.gz.sha256"),
        ];
        let result = ChecksumVerifier::find_checksum_asset(&assets, &target, None);
        assert!(result.is_some());
        let (asset, algo) = result.unwrap();
        assert_eq!(asset.filename, "foo.tar.gz.sha256");
        assert_eq!(algo, ChecksumAlgorithm::Sha256);
    }

    #[test]
    fn find_sidecar_sha512() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![
            target.clone(),
            make_asset("foo.tar.gz.sha512"),
        ];
        let result = ChecksumVerifier::find_checksum_asset(&assets, &target, None);
        assert!(result.is_some());
        let (_, algo) = result.unwrap();
        assert_eq!(algo, ChecksumAlgorithm::Sha512);
    }

    #[test]
    fn find_global_sha256sums() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![
            target.clone(),
            make_asset("SHA256SUMS"),
        ];
        let result = ChecksumVerifier::find_checksum_asset(&assets, &target, None);
        assert!(result.is_some());
        let (_, algo) = result.unwrap();
        assert_eq!(algo, ChecksumAlgorithm::Sha256);
    }

    #[test]
    fn find_prefers_sha256_over_md5() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![
            target.clone(),
            make_asset("foo.tar.gz.md5"),
            make_asset("foo.tar.gz.sha256"),
        ];
        let result = ChecksumVerifier::find_checksum_asset(&assets, &target, None);
        assert_eq!(result.unwrap().0.filename, "foo.tar.gz.sha256");
    }

    #[test]
    fn find_none_when_missing() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![target.clone()];
        let result = ChecksumVerifier::find_checksum_asset(&assets, &target, None);
        assert!(result.is_none());
    }

    #[test]
    fn parse_single_hash() {
        let hash = "aabbccdd1122";
        let result = ChecksumVerifier::parse_checksum(hash, "foo.tar.gz");
        assert_eq!(result.unwrap(), "aabbccdd1122");
    }

    #[test]
    fn parse_gnu_style() {
        let content = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  foo.tar.gz\n";
        let result = ChecksumVerifier::parse_checksum(content, "foo.tar.gz");
        assert_eq!(result.unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn parse_binary_style() {
        let content = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 *foo.tar.gz\n";
        let result = ChecksumVerifier::parse_checksum(content, "foo.tar.gz");
        assert_eq!(result.unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn parse_multi_line_finds_correct_file() {
        let content = r#"
# Comment
abc123  bar.tar.gz
def456  foo.tar.gz
ghi789  baz.tar.gz
"#;
        let result = ChecksumVerifier::parse_checksum(content, "foo.tar.gz");
        assert_eq!(result.unwrap(), "def456");
    }

    #[test]
    fn parse_missing_file_errors() {
        let content = "abc123  bar.tar.gz\n";
        let result = ChecksumVerifier::parse_checksum(content, "foo.tar.gz");
        assert!(matches!(result, Err(ChecksumError::ParseError(_))));
    }

    #[test]
    fn verify_match() {
        assert!(ChecksumVerifier::verify("abc", "abc").is_ok());
        assert!(ChecksumVerifier::verify("ABC", "abc").is_ok());
    }

    #[test]
    fn verify_mismatch() {
        let err = ChecksumVerifier::verify("abc", "def").unwrap_err();
        assert!(matches!(err, ChecksumError::Mismatch { .. }));
    }
}
