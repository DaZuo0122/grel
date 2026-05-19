//! File commands (-F operation)

use anyhow::{Context, Result};
use grel_core::{Forge, PackageRef};
use owo_colors::OwoColorize;

use crate::commands::CommandContext;

/// Search installed packages for a filename pattern
pub async fn cmd_file_search(ctx: &CommandContext<'_>, pattern: String) -> Result<()> {
    let db = ctx.db().await?;
    let lower = pattern.to_lowercase();

    let results = db.search_package_files(&lower).await?;

    if results.is_empty() {
        // Fallback: search package metadata
        let packages = db.list_packages().await?;
        let mut any = false;
        for pkg in &packages {
            if pkg.asset_filename.to_lowercase().contains(&lower)
                || pkg.install_path.to_lowercase().contains(&lower)
            {
                any = true;
                if ctx.cli.quiet {
                    println!("{}", pkg.install_path);
                } else {
                    println!("  {}  {}", pkg.package_ref(), pkg.asset_filename);
                }
            }
        }
        if !any {
            println!("No files match '{}'", pattern);
        }
        db.close().await;
        return Ok(());
    }

    let mut last_pkg_ref = String::new();
    for (pkg, file) in &results {
        if ctx.cli.quiet {
            println!("{}", file.file_path);
        } else {
            let pkg_ref = pkg.package_ref();
            if pkg_ref != last_pkg_ref {
                if !last_pkg_ref.is_empty() {
                    println!();
                }
                println!("{} {}", pkg_ref.bold(), pkg.install_path);
                last_pkg_ref = pkg_ref;
            }
            println!("  {}", file.file_path);
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
            let files = if let Some(id) = p.id {
                db.get_package_files(id).await.unwrap_or_default()
            } else {
                Vec::new()
            };

            if files.is_empty() {
                // Fallback: walk filesystem
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
            } else {
                for file in &files {
                    if ctx.cli.quiet {
                        println!("{}", file.file_path);
                    } else {
                        println!("{} {}", p.package_ref(), file.file_path);
                    }
                }
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

        if let Some(id) = pkg.id {
            let file_models: Vec<grel_cache::models::PackageFile> = files
                .iter()
                .filter_map(|path| {
                    let abs_str = path.to_string_lossy().to_string();
                    let ftype = classify_file_type(path, &ctx.config.paths.bin_dir);
                    Some(grel_cache::models::PackageFile::new(id, abs_str, ftype))
                })
                .collect();

            if let Err(e) = db.set_package_files(id, &file_models).await {
                eprintln!(
                    "  {}: failed to update file index: {}",
                    pkg.package_ref(),
                    e
                );
            }
        }

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

/// Classify a file into a type hint for the index.
fn classify_file_type(path: &std::path::Path, bin_dir: &std::path::Path) -> String {
    if path.starts_with(bin_dir) {
        return "binary".into();
    }

    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return "data".into();
    };
    let lower = name.to_lowercase();

    if lower.ends_with(".toml")
        || lower.ends_with(".conf")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".json")
        || lower.ends_with(".ini")
        || lower.ends_with(".cfg")
    {
        return "config".into();
    }

    const DOC_NAMES: &[&str] = &[
        "readme", "license", "unlicense", "copying", "changelog", "changes", "notice",
        "authors", "contributors", "credits",
    ];
    let stem = std::path::Path::new(&lower)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if DOC_NAMES.contains(&stem.as_str()) || lower.ends_with(".md") || lower.ends_with(".txt") {
        return "doc".into();
    }

    if lower.ends_with(".exe")
        || lower.ends_with(".bat")
        || lower.ends_with(".cmd")
        || lower.ends_with(".ps1")
        || lower.ends_with(".com")
        || lower.ends_with(".bin")
    {
        return "binary".into();
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.permissions().mode() & 0o111 != 0 {
                return "binary".into();
            }
        }
    }

    "data".into()
}
