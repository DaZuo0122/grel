//! GitHub API implementation.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::trait_def::{ProviderError, Release, ReleaseProvider, ProviderType, SearchResult};

/// GitHub provider
pub struct GitHubProvider {
    client: Client,
    token: Option<String>,
}

impl GitHubProvider {
    pub fn new(client: Client, token: Option<String>) -> Self {
        Self { client, token }
    }

    fn api_url(&self, path: &str) -> String {
        format!("https://api.github.com{path}")
    }

    fn add_auth_header(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.token {
            builder.header("Authorization", format!("Bearer {token}"))
        } else {
            builder
        }
    }
}

#[async_trait]
impl ReleaseProvider for GitHubProvider {
    async fn latest_release(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Release, ProviderError> {
        let url = self.api_url(&format!("/repos/{owner}/{repo}/releases/latest"));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;

        let status = response.status();
        if status == 404 {
            return Err(ProviderError::NotFound(format!(
                "{owner}/{repo} not found"
            )));
        }

        if status == 403 {
            // Check for rate limiting
            if let Some(limit) = response.headers().get("X-RateLimit-Remaining") {
                if limit.to_str().map_or(0, |s| s.parse::<u64>().unwrap_or(0)) == 0 {
                    return Err(ProviderError::RateLimitExceeded);
                }
            }
        }

        if !status.is_success() {
            // Try to read error body for better diagnostics
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ApiError(format!(
                "GitHub API returned HTTP {status}: {body}"
            )));
        }

        let release: GitHubRelease = response.json().await.map_err(|e| {
            ProviderError::ParseError(format!("Failed to decode release JSON: {e}"))
        })?;
        Ok(Release::from_github(release, owner, repo))
    }

    async fn get_release(
        &self,
        owner: &str,
        repo: &str,
        tag: &str,
    ) -> Result<Release, ProviderError> {
        let url = self.api_url(&format!("/repos/{owner}/{repo}/releases/tags/{tag}"));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;

        if response.status() == 404 {
            return Err(ProviderError::NotFound(format!(
                "Tag {tag} not found in {owner}/{repo}"
            )));
        }

        let release: GitHubRelease = response.json().await?;
        Ok(Release::from_github(release, owner, repo))
    }

    async fn search_repos(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        let url = self.api_url(&format!(
            "/search/repositories?q={}&sort=stars&order=desc&per_page={}",
            urlencoding::encode(query),
            max_results.min(100) // GitHub API max per page is 100
        ));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;

        if response.status() == 403 {
            if let Some(limit) = response.headers().get("X-RateLimit-Remaining") {
                if limit.to_str().map_or(0, |s| s.parse::<u64>().unwrap_or(0)) == 0 {
                    return Err(ProviderError::RateLimitExceeded);
                }
            }
        }

        let search_result: GitHubSearchResponse = response.json().await.map_err(|e| {
            ProviderError::ParseError(format!("Failed to decode search response: {e}"))
        })?;
        let results: Vec<SearchResult> = search_result
            .items
            .into_iter()
            .filter_map(|item| {
                // Skip entries with no owner
                let owner = item.owner?.login;
                Some(SearchResult {
                    owner,
                    repo: item.name,
                    description: item.description.unwrap_or_default(),
                    stargazers_count: item.stargazers_count,
                    latest_tag: None,
                })
            })
            .collect();

        Ok(results)
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::GitHub
    }
}

/// GitHub release asset
#[derive(Debug, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

/// GitHub release
#[derive(Debug, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub prerelease: bool,
    pub assets: Vec<GitHubAsset>,
}

/// GitHub search response
#[derive(Debug, Deserialize)]
pub struct GitHubSearchResponse {
    pub total_count: u64,
    pub incomplete_results: bool,
    pub items: Vec<GitHubRepo>,
}

/// GitHub repository (search result item)
#[derive(Debug, Deserialize)]
pub struct GitHubRepo {
    pub name: String,
    pub full_name: String, // "owner/repo"
    pub description: Option<String>,
    pub stargazers_count: u64,
    pub owner: Option<GitHubOwner>,
}

/// GitHub owner (from search result)
#[derive(Debug, Deserialize)]
pub struct GitHubOwner {
    pub login: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_asset_deserialization() {
        let json = r#"{
            "name": "tool-v1.0.0-linux-x86_64.tar.gz",
            "browser_download_url": "https://example.com/tool.tar.gz",
            "size": 12345
        }"#;

        let asset: GitHubAsset = serde_json::from_str(json).unwrap();
        assert_eq!(asset.name, "tool-v1.0.0-linux-x86_64.tar.gz");
        assert_eq!(asset.size, 12345);
    }
}
