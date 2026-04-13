//! Core resolution logic and asset tokenization for grel.
//!
//! This crate provides:
//! - Platform enums with alias mapping
//! - Asset tokenization from filenames
//! - Deterministic filter/sort pipeline

mod platform;
mod asset;
mod resolver;
mod package_ref;

pub use platform::*;
pub use asset::*;
pub use resolver::*;
pub use package_ref::*;

// Re-export for external use
pub use asset::detect_version_from_tag;
