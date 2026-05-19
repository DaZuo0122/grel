//! Query commands (-Q operation)

use anyhow::{Context, Result};
use grel_cache::models;
use grel_config::Config;
use grel_core::{Forge, PackageRef};
use owo_colors::OwoColorize;

use crate::commands::CommandContext;

/// List installed packages
pub async fn cmd_list(ctx: &CommandContext<'_>) -> Result<()> {
    let db = ctx.db().await?;
    let packages = db.list_packages().await?;

    let quiet = ctx.cli.quiet;
    let explicit_only = ctx.cli.explicit;
    let deps_only = ctx.cli.deps_filter;

    if quiet {
        for pkg in &packages {
            if explicit_only && !pkg.is_explicit {
                continue;
            }
            if deps_only && pkg.is_explicit {
                continue;
            }
            println!("{}", pkg.package_ref());
        }
        db.close().await;
        return Ok(());
    }

    if packages.is_empty() {
        println!("No packages installed.");
        println!("\nInstall your first package with:");
        println!("  grel -S owner/repo");
        db.close().await;
        return Ok(());
    }

    let active_count = packages
        .iter()
        .filter(|p| matches!(p.status, models::PackageStatus::Active))
        .count();
    let orphaned_count = packages
        .iter()
        .filter(|p| matches!(p.status, models::PackageStatus::Orphaned))
        .count();

    println!(
        "{}",
        format!("Installed packages ({active_count} active, {orphaned_count} orphaned)").bold()
    );
    println!();

    for pkg in &packages {
        if explicit_only && !pkg.is_explicit {
            continue;
        }
        if deps_only && pkg.is_explicit {
            continue;
        }

        let status_icon: String = match pkg.status {
            models::PackageStatus::Active => "●".green().to_string(),
            models::PackageStatus::Orphaned => "●".yellow().to_string(),
            models::PackageStatus::Migrated => "●".white().to_string(),
        };

        let managed_tag = if pkg.is_managed {
            "managed"
        } else {
            "unmanaged"
        };
        let explicit_tag = if pkg.is_explicit { "explicit" } else { "dep" };

        print!(
            "  {}  {:<30} {:<12} {:<10} {:<8} {}",
            status_icon,
            pkg.package_ref(),
            format!("v{}", pkg.version),
            managed_tag,
            explicit_tag,
            pkg.status,
        );

        if pkg.status == models::PackageStatus::Orphaned {
            if let Some(orphaned_at) = pkg.orphaned_at {
                let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(orphaned_at, 0);
                if let Some(dt) = dt {
                    print!(" (since {})", dt.format("%Y-%m-%d"));
                }
            }
        }

        println!();
    }

    db.close().await;
    Ok(())
}

/// List files owned by a package (or all packages)
pub async fn cmd_list_files(ctx: &CommandContext<'_>, pkg_filter: Option<&str>) -> Result<()> {
    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let quiet = ctx.cli.quiet;

    let targets: Vec<_> = if let Some(filter) = pkg_filter {
        packages
            .into_iter()
            .filter(|p| {
                let short = p.package_ref();
                short == filter || short.contains(filter)
            })
            .collect()
    } else {
        packages
    };

    if targets.is_empty() {
        if let Some(f) = pkg_filter {
            println!("No package matches '{}'", f);
        } else {
            println!("No packages installed.");
        }
        db.close().await;
        return Ok(());
    }

    for pkg in &targets {
        let install_path = std::path::Path::new(&pkg.install_path);
        let files = if let Some(id) = pkg.id {
            db.get_package_files(id).await.unwrap_or_default()
        } else {
            Vec::new()
        };

        if files.is_empty() && !install_path.exists() {
            if !quiet {
                eprintln!("  {}: install path not found", pkg.package_ref());
            }
            continue;
        }

        if quiet {
            for file in &files {
                println!("{}", file.file_path);
            }
        } else {
            println!("{} {}", pkg.package_ref().bold(), install_path.display());
            for file in &files {
                println!("  {}", file.file_path);
            }
        }
    }

    db.close().await;
    Ok(())
}

/// List orphaned packages (repo-unreachable status)
pub async fn cmd_list_orphans(ctx: &CommandContext<'_>) -> Result<()> {
    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let orphans: Vec<_> = packages
        .into_iter()
        .filter(|p| matches!(p.status, models::PackageStatus::Orphaned))
        .collect();

    if orphans.is_empty() {
        println!("No orphaned packages.");
        db.close().await;
        return Ok(());
    }

    if ctx.cli.quiet {
        for pkg in &orphans {
            println!("{}", pkg.package_ref());
        }
    } else {
        println!(
            "{}",
            format!("Orphaned packages ({})" , orphans.len()).bold()
        );
        println!();
        for pkg in &orphans {
            print!("  {:<30} v{}", pkg.package_ref(), pkg.version);
            if let Some(orphaned_at) = pkg.orphaned_at {
                let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(orphaned_at, 0);
                if let Some(dt) = dt {
                    print!(" (since {})", dt.format("%Y-%m-%d"));
                }
            }
            println!();
        }
    }

    db.close().await;
    Ok(())
}

/// List unrequired packages (dependency orphans: no other package depends on them)
pub async fn cmd_list_unrequired(ctx: &CommandContext<'_>) -> Result<()> {
    let db = ctx.db().await?;
    let all_packages = db.list_packages().await?;

    let mut graph = grel_core::DependencyGraph::new();
    let explicit: Vec<_> = all_packages
        .iter()
        .filter(|p| p.is_explicit)
        .filter_map(|p| PackageRef::parse_with_forge(&p.package_ref(), Forge::GitHub).ok())
        .collect();
    let installed: Vec<_> = all_packages
        .iter()
        .filter_map(|p| PackageRef::parse_with_forge(&p.package_ref(), Forge::GitHub).ok())
        .collect();

    for p in &all_packages {
        if let Ok(deps) = db.get_dependencies(p.id.unwrap_or(0)).await {
            if let Ok(from) = PackageRef::parse_with_forge(&p.package_ref(), Forge::GitHub) {
                for dep in deps {
                    if let Some(to) = dep.as_grel_ref() {
                        graph.add_edge(from.clone(), to);
                    }
                }
            }
        }
    }

    let orphans = graph.find_orphans(&installed, &explicit);

    if orphans.is_empty() {
        println!("No unrequired packages.");
        db.close().await;
        return Ok(());
    }

    if ctx.cli.quiet {
        for orphan in &orphans {
            println!("{}", orphan.to_short_ref());
        }
    } else {
        println!(
            "{}",
            format!("Unrequired packages ({})", orphans.len()).bold()
        );
        println!();
        for orphan in &orphans {
            println!("  {}", orphan.to_short_ref());
        }
    }

    db.close().await;
    Ok(())
}

/// Show local package info
pub async fn cmd_info_local(ctx: &CommandContext<'_>, pkg_ref_str: String) -> Result<()> {
    let db = ctx.db().await?;

    let pkg_ref = PackageRef::parse_with_forge(&pkg_ref_str, Forge::GitHub)
        .with_context(|| format!("Invalid package reference: {pkg_ref_str}"))?;

    let pkg = db
        .get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo)
        .await?;

    let Some(pkg) = pkg else {
        eprintln!(
            "{}",
            format!("Package not found: {}", pkg_ref.to_short_ref()).red()
        );
        db.close().await;
        return Ok(());
    };

    println!("{}", format!("Package: {}", pkg.package_ref()).bold());
    println!("  Version:       {}", pkg.version);
    println!("  Status:        {}", pkg.status);
    println!("  Managed:       {}", pkg.is_managed);
    println!("  Explicit:      {}", pkg.is_explicit);
    println!("  Asset:         {}", pkg.asset_filename);
    println!("  Install path:  {}", pkg.install_path);

    if let Some(checksum) = &pkg.checksum {
        println!("  SHA256:        {checksum}");
    }

    // Show dependencies
    if let Ok(deps) = db.get_dependencies(pkg.id.unwrap_or(0)).await {
        if !deps.is_empty() {
            println!("  Dependencies:");
            for dep in &deps {
                println!("    {}", dep.display_target());
            }
        }
    }

    db.close().await;
    Ok(())
}

/// Show remote package info
pub async fn cmd_info_remote(ctx: &CommandContext<'_>, pkg_ref_str: String) -> Result<()> {
    let pkg_ref = PackageRef::parse_with_forge(&pkg_ref_str, ctx.default_forge())
        .with_context(|| format!("Invalid package reference: {pkg_ref_str}"))?;

    let client = grel_network::build_http_client(&Config::default().general)?;
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let registry = grel_providers::ProviderRegistry::new(client, github_token);

    let provider = registry
        .get_provider(&pkg_ref.forge)
        .with_context(|| format!("Provider for {} not available", pkg_ref.forge))?;

    let release = provider
        .latest_release(&pkg_ref.owner, &pkg_ref.repo)
        .await?;

    println!(
        "{}",
        format!("Package: {}/{}", pkg_ref.owner, pkg_ref.repo).bold()
    );
    println!("  Latest:        {}", release.tag);
    println!("  Name:          {}", &release.name);
    if !release.description.is_empty() {
        println!("  Description:   {}", release.description);
    }
    if release.prerelease {
        println!("  Pre-release:   yes");
    }
    println!("  Assets:        {}", release.assets.len());

    Ok(())
}

/// Find which package owns a file
pub async fn cmd_owns(ctx: &CommandContext<'_>, path: String) -> Result<()> {
    let db = ctx.db().await?;

    let query = std::path::Path::new(&path);
    let query_name = query.file_name();
    let query_canon = query.canonicalize().ok();

    #[cfg(debug_assertions)]
    eprintln!("[owns] query='{}' name={:?} canon={:?}", path, query_name, query_canon);

    let mut found = Vec::new();

    // Candidate 1: exact path match via package_files index
    if let Ok(pkgs) = db.find_package_by_file_path(&path).await {
        for pkg in pkgs {
            if !found.iter().any(|p: &grel_cache::models::InstalledPackage| p.id == pkg.id) {
                found.push(pkg);
            }
        }
    }

    // Candidate 2: canonicalized path match via package_files index
    if let Some(ref canon) = query_canon {
        let canon_str = canon.to_string_lossy().to_string();
        if canon_str != path {
            if let Ok(pkgs) = db.find_package_by_file_path(&canon_str).await {
                for pkg in pkgs {
                    if !found.iter().any(|p: &grel_cache::models::InstalledPackage| p.id == pkg.id) {
                        found.push(pkg);
                    }
                }
            }
        }
    }

    // Candidate 3: filename match via package_files index
    if let Some(q_name) = query_name {
        let name_str = q_name.to_string_lossy().to_string();
        if let Ok(pkgs) = db.find_package_by_filename(&name_str).await {
            for pkg in pkgs {
                if !found.iter().any(|p: &grel_cache::models::InstalledPackage| p.id == pkg.id) {
                    found.push(pkg);
                }
            }
        }
    }

    // Candidate 4: stem match (e.g. "rg" matches "rg.exe")
    if let Some(q_name) = query_name {
        let name_str = q_name.to_string_lossy().to_string();
        let stem = std::path::Path::new(&name_str)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string());
        if let Some(stem_str) = stem {
            if stem_str != name_str {
                if let Ok(pkgs) = db.find_package_by_filename(&stem_str).await {
                    for pkg in pkgs {
                        if !found.iter().any(|p: &grel_cache::models::InstalledPackage| p.id == pkg.id) {
                            found.push(pkg);
                        }
                    }
                }
            }
        }
    }

    // Fallback: filesystem walk for packages not yet indexed
    if found.is_empty() {
        let packages = db.list_packages().await?;
        for pkg in &packages {
            let install_path = std::path::Path::new(&pkg.install_path);
            let mut matched = false;

            // Linked binaries in bin_dir
            for bin_name in pkg.binary_list() {
                let bin_path = ctx.config.paths.bin_dir.join(&bin_name);
                if path_matches(&bin_path, query, query_canon.as_deref()) {
                    matched = true;
                    break;
                }
            }

            // Files inside install directory
            if !matched && install_path.exists() {
                let files = walkdir_for_owns(install_path);
                for file in files {
                    if path_matches(&file, query, query_canon.as_deref()) {
                        matched = true;
                        break;
                    }
                }
            }

            // Install directory itself
            if !matched && path_matches(install_path, query, query_canon.as_deref()) {
                matched = true;
            }

            // Archive file
            if !matched {
                let archive_path = if install_path.ends_with(&pkg.asset_filename) {
                    install_path.to_path_buf()
                } else {
                    install_path.join(&pkg.asset_filename)
                };
                if path_matches(&archive_path, query, query_canon.as_deref()) {
                    matched = true;
                }
            }

            if matched {
                found.push(pkg.clone());
            }
        }
    }

    if found.is_empty() {
        println!("No package owns '{}'", path);
    } else {
        for pkg in found {
            println!("{}", pkg.package_ref());
        }
    }

    db.close().await;
    Ok(())
}

/// Walk a directory recursively, returning all file paths.
/// Lightweight version for cmd_owns (does not use Result).
fn walkdir_for_owns(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    if path.is_file() {
        result.push(path.to_path_buf());
        return result;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return result;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            result.extend(walkdir_for_owns(&path));
        } else {
            result.push(path);
        }
    }
    result
}

/// Check if a candidate path matches the query path.
fn path_matches(
    candidate: &std::path::Path,
    query: &std::path::Path,
    query_canon: Option<&std::path::Path>,
) -> bool {
    if candidate == query {
        return true;
    }
    if let Ok(canon) = candidate.canonicalize() {
        if canon == query {
            return true;
        }
        if let Some(qc) = query_canon {
            if canon == qc {
                return true;
            }
        }
    }
    // Fallback: check filenames (including stem match for .exe etc.)
    if filename_matches(candidate.file_name(), query.file_name()) {
        return true;
    }
    false
}

/// Check whether two filename OsStrs match.
/// Matches exact names or stems (e.g. "rg" matches "rg.exe" on Windows).
fn filename_matches(a: Option<&std::ffi::OsStr>, b: Option<&std::ffi::OsStr>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    if a == b {
        return true;
    }
    // Compare stems: "rg" should match "rg.exe"
    let a_str = a.to_string_lossy();
    let b_str = b.to_string_lossy();
    let a_stem = std::path::Path::new(&*a_str)
        .file_stem()
        .map(|s| s.to_string_lossy());
    let b_stem = std::path::Path::new(&*b_str)
        .file_stem()
        .map(|s| s.to_string_lossy());
    a_stem == b_stem
}

/// Verify checksums of installed files
pub async fn cmd_verify_checksums(ctx: &CommandContext<'_>) -> Result<()> {
    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let active: Vec<_> = packages
        .into_iter()
        .filter(|p| matches!(p.status, models::PackageStatus::Active) && p.checksum.is_some())
        .collect();

    if active.is_empty() {
        println!("No packages with checksums to verify.");
        db.close().await;
        return Ok(());
    }

    println!("{}", "Verifying checksums...".bold());

    let mut ok = 0;
    let mut failed = 0;
    let mut missing = 0;

    for pkg in &active {
        let install_path = std::path::Path::new(&pkg.install_path);
        let archive_path = if install_path.ends_with(&pkg.asset_filename) {
            install_path.to_path_buf()
        } else {
            install_path.join(&pkg.asset_filename)
        };

        print!("  {:<35} ", pkg.package_ref());

        if !archive_path.exists() {
            println!("{}", "MISSING (archive deleted)".red());
            missing += 1;
            continue;
        }

        let computed = {
            use sha2::Digest;
            use std::io::Read;
            let mut hasher = sha2::Sha256::new();
            let mut file = std::fs::File::open(&archive_path)?;
            let mut buf = [0u8; 65536];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            format!("{:x}", hasher.finalize())
        };

        if let Some(ref expected) = pkg.checksum {
            if computed == *expected {
                println!("{}", "OK".green());
                ok += 1;
            } else {
                println!(
                    "{}",
                    format!("FAILED (expected: {})", &expected[..16]).red()
                );
                failed += 1;
            }
        }
    }

    println!();
    println!("  Verified: {ok}");
    if failed > 0 {
        println!("  {}", format!("Failed: {failed}").red());
    }
    if missing > 0 {
        println!("  {}", format!("Missing: {missing}").yellow());
    }

    db.close().await;
    Ok(())
}

/// Search locally installed packages
pub async fn cmd_local_search(ctx: &CommandContext<'_>, pattern: String) -> Result<()> {
    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let lower = pattern.to_lowercase();

    let matches: Vec<_> = packages
        .into_iter()
        .filter(|p| {
            p.package_ref().to_lowercase().contains(&lower)
                || p.owner.to_lowercase().contains(&lower)
                || p.repo.to_lowercase().contains(&lower)
        })
        .collect();

    if matches.is_empty() {
        println!("No packages match '{}'", pattern);
        db.close().await;
        return Ok(());
    }

    if ctx.cli.quiet {
        for pkg in &matches {
            println!("{}", pkg.package_ref());
        }
    } else {
        println!(
            "{}",
            format!("Packages matching '{}' ({} found)", pattern, matches.len()).bold()
        );
        println!();
        for pkg in &matches {
            println!("  {:<30} v{}", pkg.package_ref(), pkg.version);
        }
    }

    db.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_matches_exact_path() {
        let candidate = std::path::Path::new("/usr/local/bin/rg");
        let query = std::path::Path::new("/usr/local/bin/rg");
        assert!(path_matches(candidate, query, None));
    }

    #[test]
    fn path_matches_canonicalized() {
        // This test may not work on all platforms due to canonicalize requiring the file to exist,
        // so we test the filename fallback instead.
        let candidate = std::path::Path::new("/some/path/rg.exe");
        let query = std::path::Path::new("/other/path/rg");
        assert!(path_matches(candidate, query, None));
    }

    #[test]
    fn path_matches_different_files() {
        let candidate = std::path::Path::new("/usr/local/bin/fd");
        let query = std::path::Path::new("/usr/local/bin/rg");
        assert!(!path_matches(candidate, query, None));
    }

    #[test]
    fn filename_matches_exact() {
        assert!(filename_matches(
            Some(std::ffi::OsStr::new("rg")),
            Some(std::ffi::OsStr::new("rg"))
        ));
    }

    #[test]
    fn filename_matches_stem() {
        assert!(filename_matches(
            Some(std::ffi::OsStr::new("rg.exe")),
            Some(std::ffi::OsStr::new("rg"))
        ));
    }

    #[test]
    fn filename_matches_different() {
        assert!(!filename_matches(
            Some(std::ffi::OsStr::new("fd")),
            Some(std::ffi::OsStr::new("rg"))
        ));
    }

    #[test]
    fn filename_matches_none() {
        assert!(!filename_matches(None, Some(std::ffi::OsStr::new("rg"))));
        assert!(!filename_matches(Some(std::ffi::OsStr::new("rg")), None));
    }
}
