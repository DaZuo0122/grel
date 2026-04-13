//! Provider registry and factory.

use grel_core::Forge;
use reqwest::Client;

use crate::trait_def::{ProviderError, ProviderType, ReleaseProvider};
use crate::github::GitHubProvider;

/// Registry of release providers
pub struct ProviderRegistry {
    github: Option<GitHubProvider>,
}

impl ProviderRegistry {
    pub fn new(client: Client, github_token: Option<String>) -> Self {
        Self {
            github: Some(GitHubProvider::new(client, github_token)),
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
            Forge::GitLab => Err(ProviderError::ApiError(
                "GitLab provider not yet implemented".into(),
            )),
            Forge::Gitea => Err(ProviderError::ApiError(
                "Gitea provider not yet implemented".into(),
            )),
            Forge::Codeberg => Err(ProviderError::ApiError(
                "Codeberg provider not yet implemented".into(),
            )),
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
