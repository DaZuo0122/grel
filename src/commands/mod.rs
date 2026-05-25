//! Command handlers for grel operations.
//!
//! Each submodule corresponds to a pacman-style operation:
//!   sync     → -S
//!   query    → -Q
//!   remove   → -R
//!   database → -D
//!   upgrade  → -U
//!   files    → -F

use std::path::PathBuf;

use anyhow::{Context, Result};
use grel_cache::Database;
use grel_cli::Cli;
use grel_config::Config;
use grel_core::RemoteAsset;

pub mod database;
pub mod files;
pub mod journal;
pub mod query;
pub mod remove;
pub mod sync;
pub mod transaction;
pub mod upgrade;

/// Shared context for command handlers.
pub struct CommandContext<'a> {
    pub config: &'a Config,
    pub cli: &'a Cli,
}

impl<'a> CommandContext<'a> {
    /// Path to the SQLite state database.
    pub fn db_path(&self) -> PathBuf {
        self.config.paths.install_root.join("state.sqlite")
    }

    /// Initialize the state database.
    pub async fn db(&self) -> Result<Database> {
        let path = self.db_path();
        Database::init(&path)
            .await
            .with_context(|| format!("Failed to initialize database at {}", path.display()))
    }

    /// Default forge from CLI.
    pub fn default_forge(&self) -> grel_core::Forge {
        self.cli.default_forge.0
    }

    /// Whether to skip interactive confirmations.
    pub fn _noconfirm(&self) -> bool {
        self.cli.noconfirm
    }
}

/// Check if an asset should be managed (auto-installed).
pub fn is_asset_managed(asset: &RemoteAsset, asset_config: &grel_config::AssetConfig) -> bool {
    if asset_config.ignore_formats.iter().any(|p| {
        if let Some(ext) = p.strip_prefix("*.") {
            asset.tokens.format == *ext
        } else {
            asset.tokens.format == *p
        }
    }) {
        return false;
    }

    let lower = asset.filename.to_lowercase();
    if asset_config
        .exclude_keywords
        .iter()
        .any(|k| lower.contains(&k.to_lowercase()))
    {
        return false;
    }

    true
}

/// Format byte size for display.
pub fn format_size(bytes: u64) -> String {
    grel_cli::format_size(bytes)
}
