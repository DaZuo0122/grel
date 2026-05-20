//! Integration tests for root-level grel behavior.
//!
//! Live GitHub API coverage is marked `#[ignore]` so the default local test
//! suite stays deterministic and offline-friendly.
//! Run with: `cargo test --test integration -- --ignored` to include them.

#![allow(clippy::unwrap_used)]

use grel_config::GeneralConfig;
use grel_core::{
    Arch, Forge, Os, PackageRef, ResolverConfig, SelectionPolicy, SelectionResult, resolve_assets,
};
use grel_network::{build_http_client, download};
use grel_providers::ProviderRegistry;

fn make_client() -> grel_network::Client {
    build_http_client(&GeneralConfig::default()).expect("failed to build HTTP client")
}

fn make_registry() -> ProviderRegistry {
    let client = make_client();
    let token = std::env::var("GREL_GITHUB_TOKEN").ok();
    ProviderRegistry::new(client, token)
}

// ---------------------------------------------------------------------------
// Provider: search_repos
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires live GitHub API access"]
async fn search_repos_returns_results() {
    let registry = make_registry();
    let provider = registry
        .get_provider(&Forge::GitHub)
        .expect("GitHub provider");

    let results = provider
        .search_repos("ripgrep", 5)
        .await
        .expect("search should succeed");
    assert!(
        !results.is_empty(),
        "expected at least one result for 'ripgrep'"
    );

    let found = results
        .iter()
        .any(|r| r.owner == "BurntSushi" && r.repo == "ripgrep");
    assert!(
        found,
        "expected BurntSushi/ripgrep in search results for 'ripgrep'"
    );
}

#[tokio::test]
#[ignore = "requires live GitHub API access"]
async fn search_repos_respects_max_results() {
    let registry = make_registry();
    let provider = registry
        .get_provider(&Forge::GitHub)
        .expect("GitHub provider");

    let results = provider
        .search_repos("rust", 3)
        .await
        .expect("search should succeed");
    assert!(
        results.len() <= 3,
        "expected at most 3 results, got {}",
        results.len()
    );
}

#[tokio::test]
#[ignore = "requires live GitHub API access"]
async fn search_repopulates_description() {
    let registry = make_registry();
    let provider = registry
        .get_provider(&Forge::GitHub)
        .expect("GitHub provider");

    let results = provider
        .search_repos("ripgrep", 1)
        .await
        .expect("search should succeed");
    assert!(!results.is_empty());
    assert!(!results[0].owner.is_empty());
    assert!(!results[0].repo.is_empty());
}

// ---------------------------------------------------------------------------
// Provider: latest_release
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires live GitHub API access"]
async fn latest_release_has_assets() {
    let registry = make_registry();
    let provider = registry
        .get_provider(&Forge::GitHub)
        .expect("GitHub provider");

    let release = provider
        .latest_release("BurntSushi", "ripgrep")
        .await
        .expect("should find latest release");

    assert!(!release.tag.is_empty());
    assert!(
        !release.assets.is_empty(),
        "ripgrep release should have assets"
    );
    assert!(release.assets.iter().any(|a| !a.url.is_empty()));
}

#[tokio::test]
#[ignore = "requires live GitHub API access"]
async fn latest_release_nonexistent_returns_not_found() {
    let registry = make_registry();
    let provider = registry
        .get_provider(&Forge::GitHub)
        .expect("GitHub provider");

    let err = provider
        .latest_release("this-repo-does-not-exist-12345", "foo")
        .await
        .expect_err("should fail for nonexistent repo");

    assert!(
        matches!(err, grel_providers::ProviderError::NotFound(_)),
        "expected NotFound, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Resolver: end-to-end asset selection
// ---------------------------------------------------------------------------

fn default_resolver_config() -> ResolverConfig {
    ResolverConfig {
        default_selection_policy: SelectionPolicy::First,
        exclude_keywords: vec![
            "setup".into(),
            "installer".into(),
            "bundle".into(),
            "nupkg".into(),
        ],
        ignore_formats: vec![
            "*.deb".into(),
            "*.rpm".into(),
            "*.msi".into(),
            "*.dmg".into(),
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

#[tokio::test]
#[ignore = "requires live GitHub API access"]
async fn resolve_selects_compatible_asset() {
    let registry = make_registry();
    let provider = registry
        .get_provider(&Forge::GitHub)
        .expect("GitHub provider");

    let release = provider
        .latest_release("BurntSushi", "ripgrep")
        .await
        .expect("should find latest release");

    let config = default_resolver_config();
    let host_os = Os::host();
    let host_arch = Arch::host();

    let selection = resolve_assets(&release.assets, &host_os, &host_arch, &config, false);

    match selection {
        SelectionResult::SingleAsset(asset) => {
            assert!(!asset.url.is_empty());
        }
        SelectionResult::NoCompatibleAssets => {
            panic!("expected at least one compatible asset for ripgrep on {host_os}/{host_arch}");
        }
        SelectionResult::MultipleAssets(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Download: basic connectivity check
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires network and downloads a file"]
async fn download_small_file_succeeds() {
    let client = make_client();
    let tmp_dir = std::env::temp_dir().join("grel-test-download");
    let dest = tmp_dir.join("test.txt");

    let result = download::download_file(&client, "https://httpbin.org/get", &dest, None, false, None).await;
    assert!(
        result.is_ok(),
        "download should succeed: {:?}",
        result.err()
    );
    let _checksum = result.unwrap().0;
    assert!(dest.exists(), "destination file should exist");

    let _ = std::fs::remove_file(&dest);
    let _ = std::fs::remove_dir(&tmp_dir);
}

// ---------------------------------------------------------------------------
// PackageRef: default forge behavior
// ---------------------------------------------------------------------------

#[test]
fn parse_two_parts_uses_default_forge() {
    let pkg = PackageRef::parse_with_forge("cli/cli", Forge::GitHub).unwrap();
    assert_eq!(pkg.forge, Forge::GitHub);
    assert_eq!(pkg.owner, "cli");
    assert_eq!(pkg.repo, "cli");
    assert_eq!(pkg.version, None);
}

#[test]
fn parse_two_parts_with_gitlab_forge() {
    let pkg = PackageRef::parse_with_forge("gnome/evince", Forge::GitLab).unwrap();
    assert_eq!(pkg.forge, Forge::GitLab);
    assert_eq!(pkg.owner, "gnome");
    assert_eq!(pkg.repo, "evince");
    assert_eq!(pkg.version, None);
}

#[test]
fn parse_three_parts_overrides_default_forge() {
    let pkg = PackageRef::parse_with_forge("github/cli/cli", Forge::GitLab).unwrap();
    assert_eq!(pkg.forge, Forge::GitHub);
    assert_eq!(pkg.owner, "cli");
    assert_eq!(pkg.repo, "cli");
    assert_eq!(pkg.version, None);
}

#[test]
fn parse_owner_repo_with_version() {
    let pkg = PackageRef::parse_with_forge("BurntSushi/ripgrep@v14.1.1", Forge::GitHub).unwrap();
    assert_eq!(pkg.forge, Forge::GitHub);
    assert_eq!(pkg.owner, "BurntSushi");
    assert_eq!(pkg.repo, "ripgrep");
    assert_eq!(pkg.version, Some("v14.1.1".into()));
}

#[test]
fn parse_forge_owner_repo_with_version() {
    let pkg = PackageRef::parse_with_forge("gitlab/foo/bar@1.0.0-beta.1", Forge::GitHub).unwrap();
    assert_eq!(pkg.forge, Forge::GitLab);
    assert_eq!(pkg.owner, "foo");
    assert_eq!(pkg.repo, "bar");
    assert_eq!(pkg.version, Some("1.0.0-beta.1".into()));
}
