//! Codeberg API implementation.
//! Codeberg uses Forgejo (a Gitea fork), so the API is Gitea-compatible.

use async_trait::async_trait;
use reqwest_middleware::ClientWithMiddleware;
use serde::Deserialize;

use crate::trait_def::{ProviderError, ProviderType, Release, ReleaseProvider, SearchResult};
use grel_core::AssetTokens;

/// Codeberg API base URL
const CODEBERG_API: &str = "https://codeberg.org/api/v1";

/// Codeberg provider
pub struct CodebergProvider {
    client: ClientWithMiddleware,
    token: Option<String>,
}

impl CodebergProvider {
    pub fn new(client: ClientWithMiddleware, token: Option<String>) -> Self {
        Self { client, token }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{CODEBERG_API}{path}")
    }

    fn add_auth_header(&self, builder: reqwest_middleware::RequestBuilder) -> reqwest_middleware::RequestBuilder {
        if let Some(token) = &self.token {
            builder.header("Authorization", format!("token {token}"))
        } else {
            builder
        }
    }
}

#[async_trait]
impl ReleaseProvider for CodebergProvider {
    async fn latest_release(&self, owner: &str, repo: &str) -> Result<Release, ProviderError> {
        let url = self.api_url(&format!("/repos/{owner}/{repo}/releases/latest"));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;

        if response.status() == 404 {
            return Err(ProviderError::NotFound(format!("{owner}/{repo} not found")));
        }

        let release: ForgejoRelease = response.json().await?;
        Ok(from_forgejo_release(release, owner, repo))
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

        let release: ForgejoRelease = response.json().await?;
        Ok(from_forgejo_release(release, owner, repo))
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
        let search_result: ForgejoSearchResponse = response.json().await?;

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
        ProviderType::Codeberg
    }

    async fn fetch_raw_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        ref_name: &str,
    ) -> Result<String, ProviderError> {
        let url = format!("https://codeberg.org/{owner}/{repo}/raw/branch/{ref_name}/{path}");
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

fn from_forgejo_release(release: ForgejoRelease, _owner: &str, _repo: &str) -> Release {
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
        provider: ProviderType::Codeberg,
    }
}

#[derive(Debug, Deserialize)]
struct ForgejoRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    prerelease: bool,
    assets: Vec<ForgejoAsset>,
}

#[derive(Debug, Deserialize)]
struct ForgejoAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct ForgejoSearchResponse {
    data: Vec<ForgejoRepo>,
}

#[derive(Debug, Deserialize)]
struct ForgejoRepo {
    name: String,
    full_name: String,
    description: Option<String>,
    stars_count: u64,
}
