//! GitLab API implementation.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::trait_def::{ProviderError, ProviderType, Release, ReleaseProvider, SearchResult};
use grel_core::AssetTokens;

/// GitLab API base URL
const GITLAB_API: &str = "https://gitlab.com/api/v4";

/// GitLab provider
pub struct GitLabProvider {
    client: Client,
    token: Option<String>,
}

impl GitLabProvider {
    pub fn new(client: Client, token: Option<String>) -> Self {
        Self { client, token }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{GITLAB_API}{path}")
    }

    fn add_auth_header(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.token {
            builder.header("PRIVATE-TOKEN", token)
        } else {
            builder
        }
    }

    fn project_path(owner: &str, repo: &str) -> String {
        urlencoding::encode(&format!("{owner}/{repo}")).into_owned()
    }
}

#[async_trait]
impl ReleaseProvider for GitLabProvider {
    async fn latest_release(&self, owner: &str, repo: &str) -> Result<Release, ProviderError> {
        // GitLab doesn't have a "latest release" endpoint like GitHub.
        // We fetch all releases and pick the first (sorted by created_at desc).
        let project = Self::project_path(owner, repo);
        let url = self.api_url(&format!("/projects/{project}/releases?per_page=1"));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;

        if response.status() == 404 {
            return Err(ProviderError::NotFound(format!("{owner}/{repo} not found")));
        }

        let releases: Vec<GitLabRelease> = response.json().await?;
        if releases.is_empty() {
            return Err(ProviderError::NotFound(format!(
                "No releases found for {owner}/{repo}"
            )));
        }

        let release = from_gitlab_release(releases.into_iter().next().unwrap(), owner, repo);
        Ok(release)
    }

    async fn get_release(
        &self,
        owner: &str,
        repo: &str,
        tag: &str,
    ) -> Result<Release, ProviderError> {
        let project = Self::project_path(owner, repo);
        let tag_enc = urlencoding::encode(tag);
        let url = self.api_url(&format!("/projects/{project}/releases/{tag_enc}"));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;

        if response.status() == 404 {
            return Err(ProviderError::NotFound(format!(
                "Tag {tag} not found in {owner}/{repo}"
            )));
        }

        let release: GitLabRelease = response.json().await?;
        Ok(from_gitlab_release(release, owner, repo))
    }

    async fn search_repos(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        let url = self.api_url(&format!(
            "/projects?search={}&per_page={}&order_by=stars&sort=desc",
            urlencoding::encode(query),
            max_results.min(100)
        ));
        let builder = self.client.get(&url);
        let builder = self.add_auth_header(builder);

        let response = builder.send().await?;
        let projects: Vec<GitLabProject> = response.json().await?;

        Ok(projects
            .into_iter()
            .map(|p| {
                let parts: Vec<&str> = p.path_with_namespace.split('/').collect();
                let owner = parts.first().copied().unwrap_or("unknown").to_string();
                let repo = parts.last().copied().unwrap_or(&p.name).to_string();
                SearchResult {
                    owner,
                    repo,
                    description: p.description.unwrap_or_default(),
                    stargazers_count: p.star_count,
                    latest_tag: None,
                }
            })
            .collect())
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::GitLab
    }

    async fn fetch_raw_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        ref_name: &str,
    ) -> Result<String, ProviderError> {
        let url = format!("https://gitlab.com/{owner}/{repo}/-/raw/{ref_name}/{path}");
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

fn from_gitlab_release(release: GitLabRelease, _owner: &str, _repo: &str) -> Release {
    let assets = release
        .assets
        .links
        .into_iter()
        .map(|link| {
            let tokens = AssetTokens::from_filename_with_tag(&link.name, Some(&release.tag_name));
            grel_core::RemoteAsset {
                filename: link.name.clone(),
                url: link.direct_asset_url,
                size_bytes: None,
                tokens,
            }
        })
        .collect();

    Release {
        tag: release.tag_name,
        name: release.name.unwrap_or_default(),
        description: release.description.unwrap_or_default(),
        assets,
        prerelease: false, // GitLab doesn't distinguish pre-releases
        provider: ProviderType::GitLab,
    }
}

#[derive(Debug, Deserialize)]
struct GitLabRelease {
    tag_name: String,
    name: Option<String>,
    description: Option<String>,
    assets: GitLabReleaseAssets,
}

#[derive(Debug, Deserialize)]
struct GitLabReleaseAssets {
    links: Vec<GitLabAssetLink>,
}

#[derive(Debug, Deserialize)]
struct GitLabAssetLink {
    name: String,
    direct_asset_url: String,
}

#[derive(Debug, Deserialize)]
struct GitLabProject {
    name: String,
    path_with_namespace: String,
    description: Option<String>,
    star_count: u64,
}
