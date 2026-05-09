//! ELF system dependency auto-resolution for grel.
//!
//! On Linux: parses DT_NEEDED from installed binaries, cross-references against
//! the live system, and optionally installs missing system libraries via the
//! host distro's package manager.
//!
//! On non-Linux: all public functions are no-ops that return empty reports.

pub mod distro;
pub mod parser;
pub mod pkgmgr;
pub mod resolver;

pub use resolver::{ResolutionReport, ResolvedPackage};

use std::path::PathBuf;

use grel_cache::Database;
use grel_config::ElfDepConfig;

/// Run ELF dep resolution for the given installed binary paths.
///
/// On non-Linux platforms this is a no-op returning an empty report.
#[cfg(target_os = "linux")]
pub async fn resolve_elf_deps(
    bin_paths: &[PathBuf],
    config: &ElfDepConfig,
    db: Option<&Database>,
) -> ResolutionReport {
    if !config.auto_resolve_system_deps || bin_paths.is_empty() {
        return ResolutionReport::default();
    }

    let distro = distro::detect(config.distro_override.as_deref());
    let pkg_mgr = pkgmgr::for_family(&distro.family);

    let r = resolver::SystemDepResolver::new(config, db, distro, pkg_mgr);
    r.resolve(bin_paths).await
}

#[cfg(not(target_os = "linux"))]
pub async fn resolve_elf_deps(
    _bin_paths: &[PathBuf],
    _config: &ElfDepConfig,
    _db: Option<&Database>,
) -> ResolutionReport {
    ResolutionReport::default()
}
