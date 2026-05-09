//! Git forge provider adapters for grel.

mod codeberg;
mod gitea;
mod github;
mod gitlab;
mod registry;
mod trait_def;

pub use codeberg::*;
pub use gitea::*;
pub use github::*;
pub use gitlab::*;
pub use registry::*;
pub use trait_def::*;
