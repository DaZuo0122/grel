//! File commands (-F operation)

use anyhow::{Context, Result};
use grel_core::{Forge, PackageRef};
use owo_colors::OwoColorize;

use crate::commands::CommandContext;

/// Search installed packages for a filename pattern
pub async fn cmd_file_search(ctx: &CommandContext<'_>, pattern: String) -> Result<()> {
    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let lower = pattern.to_lowercase();

    for pkg in &packages {
        let install_path = std::path::Path::new(&pkg.install_path);
        if !install_path.exists() {
            continue;
        }

        let mut matched = false;
        for entry in walkdir(install_path)? {
            let file_name = entry
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if file_name.to_lowercase().contains(&lower) {
                matched = true;
                if ctx.cli.quiet {
                    println!("{}", entry.display());
                } else {
                    println!("  {}  {}", pkg.package_ref(), entry.display());
                }
            }
        }

        if !matched {
            if pkg.asset_filename.to_lowercase().contains(&lower)
                || pkg.install_path.to_lowercase().contains(&lower)
            {
                if ctx.cli.quiet {
                    println!("{}", pkg.install_path);
                } else {
                    println!("  {}  {}", pkg.package_ref(), pkg.asset_filename);
                }
            }
        }
    }

    db.close().await;
    Ok(())
}

/// List all files from a package
pub async fn cmd_file_list(ctx: &CommandContext<'_>, pkg_ref_str: String) -> Result<()> {
    let db = ctx.db().await?;

    let pkg_ref = PackageRef::parse_with_forge(&pkg_ref_str, Forge::GitHub)
        .with_context(|| format!("Invalid package reference: {pkg_ref_str}"))?;

    let pkg = db
        .get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo)
        .await?;

    match pkg {
        Some(p) => {
            let install_path = std::path::Path::new(&p.install_path);
            if install_path.exists() {
                for entry in walkdir(install_path)? {
                    if ctx.cli.quiet {
                        println!("{}", entry.display());
                    } else {
                        println!("{} {}", p.package_ref(), entry.display());
                    }
                }
            } else {
                eprintln!("Install path not found: {}", p.install_path);
            }
        }
        None => {
            eprintln!("Package not found: {}", pkg_ref.to_short_ref());
        }
    }

    db.close().await;
    Ok(())
}

/// Rebuild file index by walking all managed packages
pub async fn cmd_reindex(ctx: &CommandContext<'_>) -> Result<()> {
    println!("{}", "Rebuilding file index...".bold());

    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let mut total_files = 0;

    for pkg in &packages {
        let install_path = std::path::Path::new(&pkg.install_path);
        if !install_path.exists() {
            continue;
        }

        let files = walkdir(install_path)?;
        total_files += files.len();

        if !ctx.cli.quiet {
            println!("  {}: {} file(s)", pkg.package_ref(), files.len());
        }
    }

    println!();
    println!(
        "Indexed {} file(s) across {} package(s)",
        total_files,
        packages.len()
    );

    db.close().await;
    Ok(())
}

/// Walk a directory recursively, returning all file paths
fn walkdir(path: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let mut result = Vec::new();
    if path.is_file() {
        result.push(path.to_path_buf());
        return Ok(result);
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            result.extend(walkdir(&path)?);
        } else {
            result.push(path);
        }
    }
    Ok(result)
}
