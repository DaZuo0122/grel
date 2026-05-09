//! Gitea API implementation.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::trait_def::{ProviderError, ProviderType, Release, ReleaseProvider, SearchResult};
use grel_core::AssetTokens;

/// Gitea API base URL (default: gitea.com)
const GITEA_API: &str = "https://gitea.com/api/v1";

/// Gitea provider
pub struct GiteaProvider {
    client: Client,
    token: Option<String>,
}

impl GiteaProvider {
    pub fn new(client: Client, token: Option<String>) -> Self {
        Self { client, token }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{GITEA_API}{path}")
    }

    fn add_auth_header(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.token {
            builder.header("Authorization", format!("token {token}"))
        } else {
            builder
        }
    }
}

#[async_trait]
impl ReleaseProvider for GiteaProvider {
    async fn latest_release(&self, owner: &str, repo: &str) -> Result<Release, ProviderError> {
        let url = self.api_url(&format!("/repos/{owner}/{repo}/releases/latest"));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;

        if response.status() == 404 {
            return Err(ProviderError::NotFound(format!("{owner}/{repo} not found")));
        }

        let release: GiteaRelease = response.json().await?;
        Ok(from_gitea_release(release, owner, repo))
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

        let release: GiteaRelease = response.json().await?;
        Ok(from_gitea_release(release, owner, repo))
    }

    async fn search_repos(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        let url = self.api_url(&format!(
            "/repos/search?q={}&limit={}&sort=stars&order=desc",
            urlencoding::encode(query),
            max_results.min(50)
        ));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;
        let search_result: GiteaSearchResponse = response.json().await?;

        Ok(search_result
            .data
            .into_iter()
            .map(|r| {
                let parts: Vec<&str> = r.full_name.split('/').collect();
                let owner = parts.first().copied().unwrap_or("unknown").to_string();
                let repo = parts.last().copied().unwrap_or(&r.name).to_string();
                SearchResult {
                    owner,
                    repo,
                    description: r.description.unwrap_or_default(),
                    stargazers_count: r.stars_count,
                    latest_tag: None,
                }
            })
            .collect())
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Gitea
    }

    async fn fetch_raw_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        ref_name: &str,
    ) -> Result<String, ProviderError> {
        let url = format!("https://gitea.com/{owner}/{repo}/raw/branch/{ref_name}/{path}");
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;

        if response.status() == 404 {
            return Err(ProviderError::NotFound(format!(
                "File {path} not found in {owner}/{repo}"
            )));
        }

        let text = response
            .text()
            .await
            .map_err(|e| ProviderError::ApiError(format!("Failed to read raw file: {e}")))?;
        Ok(text)
    }
}

fn from_gitea_release(release: GiteaRelease, _owner: &str, _repo: &str) -> Release {
    let assets = release
        .assets
        .into_iter()
        .map(|asset| {
            let tokens = AssetTokens::from_filename_with_tag(&asset.name, Some(&release.tag_name));
            grel_core::RemoteAsset {
                filename: asset.name.clone(),
                url: asset.browser_download_url,
                size_bytes: Some(asset.size),
                tokens,
            }
        })
        .collect();

    Release {
        tag: release.tag_name,
        name: release.name.unwrap_or_default(),
        description: release.body.unwrap_or_default(),
        assets,
        prerelease: release.prerelease,
        provider: ProviderType::Gitea,
    }
}

#[derive(Debug, Deserialize)]
struct GiteaRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    prerelease: bool,
    assets: Vec<GiteaAsset>,
}

#[derive(Debug, Deserialize)]
struct GiteaAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct GiteaSearchResponse {
    data: Vec<GiteaRepo>,
}

#[derive(Debug, Deserialize)]
struct GiteaRepo {
    name: String,
    full_name: String,
    description: Option<String>,
    stars_count: u64,
}
