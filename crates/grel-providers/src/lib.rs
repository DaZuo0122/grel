//! Git forge provider adapters for grel.

mod trait_def;
mod github;
mod gitlab;
mod gitea;
mod codeberg;
mod registry;

pub use trait_def::*;
pub use github::*;
pub use gitlab::*;
pub use gitea::*;
pub use codeberg::*;
pub use registry::*;
