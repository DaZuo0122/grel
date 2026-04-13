//! Release provider trait definition.

use async_trait::async_trait;
use grel_core::{RemoteAsset, AssetTokens};

use crate::github::GitHubRelease;

/// A release from a git forge
#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub description: String,
    pub assets: Vec<RemoteAsset>,
    pub prerelease: bool,
    pub provider: ProviderType,
}

/// A search result entry (repository summary)
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub owner: String,
    pub repo: String,
    pub description: String,
    pub stargazers_count: u64,
    pub latest_tag: Option<String>,
}

/// Type of provider
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderType {
    GitHub,
    GitLab,
    Gitea,
    Codeberg,
}

/// Release provider trait
#[async_trait]
pub trait ReleaseProvider: Send + Sync {
    /// Get the latest release for a repository
    async fn latest_release(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Release, ProviderError>;

    /// Get a specific release by tag
    async fn get_release(
        &self,
        owner: &str,
        repo: &str,
        tag: &str,
    ) -> Result<Release, ProviderError>;

    /// Search repositories by keyword
    async fn search_repos(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, ProviderError>;

    /// Get the provider type
    fn provider_type(&self) -> ProviderType;
}

/// Provider errors
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Release not found: {0}")]
    NotFound(String),

    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("API rate limit exceeded")]
    RateLimitExceeded,

    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Parse error: {0}")]
    ParseError(String),
}

impl Release {
    /// Convert a GitHubRelease to a generic Release
    pub fn from_github(release: GitHubRelease, _owner: &str, _repo: &str) -> Self {
        let assets = release
            .assets
            .into_iter()
            .map(|asset| {
                let tokens = AssetTokens::from_filename_with_tag(
                    &asset.name,
                    Some(&release.tag_name),
                );
                RemoteAsset {
                    filename: asset.name.clone(),
                    url: asset.browser_download_url,
                    size_bytes: Some(asset.size),
                    tokens,
                }
            })
            .collect();

        Self {
            tag: release.tag_name,
            name: release.name.clone().unwrap_or_default(),
            description: release.description.unwrap_or_default(),
            assets,
            prerelease: release.prerelease,
            provider: ProviderType::GitHub,
        }
    }
}
