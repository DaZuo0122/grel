//! Configuration management for grel.
//!
//! Handles TOML configuration loading, environment variable overrides,
//! and configuration migrations.

mod config;
mod paths;

pub use config::*;
pub use paths::*;
