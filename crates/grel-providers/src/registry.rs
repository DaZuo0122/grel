//! Provider registry and factory.

use grel_core::Forge;
use reqwest::Client;

use crate::codeberg::CodebergProvider;
use crate::gitea::GiteaProvider;
use crate::github::GitHubProvider;
use crate::gitlab::GitLabProvider;
use crate::trait_def::{ProviderError, ProviderType, ReleaseProvider};

/// Registry of release providers
pub struct ProviderRegistry {
    github: Option<GitHubProvider>,
    gitlab: Option<GitLabProvider>,
    gitea: Option<GiteaProvider>,
    codeberg: Option<CodebergProvider>,
}

impl ProviderRegistry {
    pub fn new(client: Client, github_token: Option<String>) -> Self {
        // For now, only GitHub token is used. Other forges could have their own env vars.
        let gitlab_token = std::env::var("GREL_GITLAB_TOKEN").ok();
        let gitea_token = std::env::var("GREL_GITEA_TOKEN").ok();
        let codeberg_token = std::env::var("GREL_CODEBERG_TOKEN").ok();

        Self {
            github: Some(GitHubProvider::new(client.clone(), github_token)),
            gitlab: Some(GitLabProvider::new(client.clone(), gitlab_token)),
            gitea: Some(GiteaProvider::new(client.clone(), gitea_token)),
            codeberg: Some(CodebergProvider::new(client, codeberg_token)),
        }
    }

    /// Get a provider by forge type
    pub fn get_provider(&self, forge: &Forge) -> Result<&dyn ReleaseProvider, ProviderError> {
        match forge {
            Forge::GitHub => self
                .github
                .as_ref()
                .map(|p| p as &dyn ReleaseProvider)
                .ok_or_else(|| ProviderError::ApiError("GitHub provider not initialized".into())),
            Forge::GitLab => self
                .gitlab
                .as_ref()
                .map(|p| p as &dyn ReleaseProvider)
                .ok_or_else(|| ProviderError::ApiError("GitLab provider not initialized".into())),
            Forge::Gitea => self
                .gitea
                .as_ref()
                .map(|p| p as &dyn ReleaseProvider)
                .ok_or_else(|| ProviderError::ApiError("Gitea provider not initialized".into())),
            Forge::Codeberg => self
                .codeberg
                .as_ref()
                .map(|p| p as &dyn ReleaseProvider)
                .ok_or_else(|| ProviderError::ApiError("Codeberg provider not initialized".into())),
        }
    }

    /// Get provider type for a forge
    pub fn provider_type_for_forge(forge: &Forge) -> ProviderType {
        match forge {
            Forge::GitHub => ProviderType::GitHub,
            Forge::GitLab => ProviderType::GitLab,
            Forge::Gitea => ProviderType::Gitea,
            Forge::Codeberg => ProviderType::Codeberg,
        }
    }
}
