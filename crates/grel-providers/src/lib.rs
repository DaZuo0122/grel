//! Git forge provider adapters for grel.

mod trait_def;
mod github;
mod registry;

pub use trait_def::*;
pub use github::*;
pub use registry::*;
