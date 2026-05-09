//! Integration tests for the grel-elf public API.

use grel_config::ElfDepConfig;
use grel_elf::resolve_elf_deps;

/// When `auto_resolve_system_deps` is disabled the function must return an
/// empty report without touching the file system or running any subprocess.
#[tokio::test]
async fn resolve_elf_deps_disabled_returns_empty() {
    let mut config = ElfDepConfig::default();
    config.auto_resolve_system_deps = false;
    // Use a real path so the "disabled" short-circuit is the only reason for
    // the empty result (not "path doesn't exist").
    let paths = vec![std::path::PathBuf::from("/bin/ls")];
    let report = resolve_elf_deps(&paths, &config, None).await;
    assert!(report.missing_libs.is_empty());
    assert!(report.resolved_pkgs.is_empty());
    assert!(report.install_cmd.is_none());
}

/// An empty path list must short-circuit before any parsing work is done.
#[tokio::test]
async fn resolve_elf_deps_empty_paths_returns_empty() {
    let config = ElfDepConfig::default();
    let report = resolve_elf_deps(&[], &config, None).await;
    assert!(report.missing_libs.is_empty());
    assert!(report.resolved_pkgs.is_empty());
    assert!(report.install_cmd.is_none());
}

/// Plain text files are not valid ELF binaries.  The parser returns no
/// DT_NEEDED entries, so the resolver has nothing to look up.
#[tokio::test]
async fn resolve_elf_deps_non_elf_files_returns_empty_missing_libs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not_elf.txt");
    std::fs::write(&path, b"just text, not an ELF binary").unwrap();

    let config = ElfDepConfig::default();
    let report = resolve_elf_deps(&[path], &config, None).await;
    // No DT_NEEDED entries → nothing can be "missing"
    assert!(report.missing_libs.is_empty());
}

/// A file containing only the ELF magic bytes (truncated) must not panic.
#[tokio::test]
async fn resolve_elf_deps_truncated_elf_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("truncated.elf");
    std::fs::write(&path, b"\x7fELF\x02\x01\x01\x00").unwrap();

    let config = ElfDepConfig::default();
    let report = resolve_elf_deps(&[path], &config, None).await;
    assert!(report.missing_libs.is_empty());
}

/// A mix of valid text files and a missing path must not panic; any parse
/// failures are silently skipped.
#[tokio::test]
async fn resolve_elf_deps_mixed_valid_and_missing_paths() {
    let dir = tempfile::tempdir().unwrap();
    let text_path = dir.path().join("text.txt");
    std::fs::write(&text_path, b"not an elf").unwrap();

    let config = ElfDepConfig::default();
    let paths = vec![
        text_path,
        std::path::PathBuf::from("/nonexistent/__grel_test__"),
    ];
    let report = resolve_elf_deps(&paths, &config, None).await;
    assert!(report.missing_libs.is_empty());
}
