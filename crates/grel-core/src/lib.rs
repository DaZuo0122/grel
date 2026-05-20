//! Core resolution logic and asset tokenization for grel.
//!
//! This crate provides:
//! - Platform enums with alias mapping
//! - Asset tokenization from filenames
//! - Deterministic filter/sort pipeline

mod asset;
mod checksum;
mod dependency;
mod manifest;
mod package_ref;
mod platform;
mod registry;
mod resolver;

pub use asset::*;
pub use checksum::*;
pub use dependency::*;
pub use manifest::*;
pub use package_ref::*;
pub use platform::*;
pub use registry::*;
pub use resolver::*;

// Re-export for external use
pub use asset::detect_version_from_tag;
