//! Package reference parsing (forge/owner/repo or owner/repo with default forge).

use std::fmt;
use std::str::FromStr;

/// A fully-qualified package reference
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageRef {
    pub forge: Forge,
    pub owner: String,
    pub repo: String,
}

impl PackageRef {
    pub fn new(forge: Forge, owner: String, repo: String) -> Self {
        Self { forge, owner, repo }
    }

    /// Parse a reference string with an explicit default forge.
    ///
    /// Accepts:
    /// - `owner/repo` → uses `default_forge`
    /// - `forge/owner/repo` → parses forge from first segment
    pub fn parse_with_forge(s: &str, default_forge: Forge) -> Result<Self, PackageRefError> {
        let parts: Vec<&str> = s.split('/').collect();

        match parts.len() {
            2 => Ok(Self {
                forge: default_forge,
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
            }),
            3 => {
                let forge = parts[0].parse()?;
                Ok(Self {
                    forge,
                    owner: parts[1].to_string(),
                    repo: parts[2].to_string(),
                })
            }
            _ => Err(PackageRefError::InvalidFormat(s.to_string())),
        }
    }

    /// Parse a reference string, defaulting to GitHub.
    pub fn parse(s: &str) -> Result<Self, PackageRefError> {
        Self::parse_with_forge(s, Forge::GitHub)
    }

    /// Full reference string
    pub fn to_string_ref(&self) -> String {
        format!("{}/{}/{}", self.forge, self.owner, self.repo)
    }

    /// Short reference (owner/repo)
    pub fn to_short_ref(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

impl fmt::Display for PackageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.forge, self.owner, self.repo)
    }
}

impl FromStr for PackageRef {
    type Err = PackageRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Supported forges
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Forge {
    GitHub,
    GitLab,
    Gitea,
    Codeberg,
}

impl fmt::Display for Forge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Forge::GitHub => write!(f, "github"),
            Forge::GitLab => write!(f, "gitlab"),
            Forge::Gitea => write!(f, "gitea"),
            Forge::Codeberg => write!(f, "codeberg"),
        }
    }
}

impl FromStr for Forge {
    type Err = PackageRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "github" => Ok(Forge::GitHub),
            "gitlab" => Ok(Forge::GitLab),
            "gitea" => Ok(Forge::Gitea),
            "codeberg" => Ok(Forge::Codeberg),
            _ => Err(PackageRefError::UnknownForge(s.to_string())),
        }
    }
}

/// Package reference errors
#[derive(Debug, thiserror::Error)]
pub enum PackageRefError {
    #[error("Invalid package reference format: '{0}' (expected: forge/owner/repo or owner/repo)")]
    InvalidFormat(String),

    #[error("Unknown forge: '{0}' (supported: github, gitlab, gitea, codeberg)")]
    UnknownForge(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_two_parts() {
        let pkg = PackageRef::parse("cli/cli").unwrap();
        assert_eq!(pkg.forge, Forge::GitHub);
        assert_eq!(pkg.owner, "cli");
        assert_eq!(pkg.repo, "cli");
    }

    #[test]
    fn test_parse_three_parts() {
        let pkg = PackageRef::parse("gitlab/owner/repo").unwrap();
        assert_eq!(pkg.forge, Forge::GitLab);
        assert_eq!(pkg.owner, "owner");
        assert_eq!(pkg.repo, "repo");
    }

    #[test]
    fn test_parse_invalid() {
        assert!(PackageRef::parse("foo").is_err());
        assert!(PackageRef::parse("a/b/c/d").is_err());
    }

    #[test]
    fn test_display() {
        let pkg = PackageRef::parse("github/owner/repo").unwrap();
        assert_eq!(pkg.to_string(), "github/owner/repo");
        assert_eq!(pkg.to_string_ref(), "github/owner/repo");
        assert_eq!(pkg.to_short_ref(), "owner/repo");
    }
}
