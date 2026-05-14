//! Remove commands (-R operation)

use anyhow::{Context, Result};
use grel_cache::models;
use grel_core::{Forge, PackageRef};
use owo_colors::OwoColorize;

use crate::commands::CommandContext;

/// Remove packages
pub async fn cmd_remove(ctx: &CommandContext<'_>, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        eprintln!("No packages specified. Usage: grel -R owner/repo");
        return Ok(());
    }

    let db = ctx.db().await?;

    for pkg_str in packages {
        let pkg_ref = PackageRef::parse_with_forge(pkg_str, Forge::GitHub)
            .with_context(|| format!("Invalid package reference: {pkg_str}"))?;

        let pkg = db
            .get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo)
            .await?;

        let Some(pkg) = pkg else {
            eprintln!("{}", format!("Package not found: {pkg_str}").red());
            continue;
        };

        let id = pkg.id.expect("Package must have an ID");

        let dependents = db
            .get_dependents(&pkg_ref.to_string_ref())
            .await?;
        if !dependents.is_empty() && !ctx.cli.recursive {
            eprintln!(
                "{}",
                format!(
                    "Cannot remove {}: other packages depend on it",
                    pkg_ref.to_short_ref()
                )
                .red()
            );
            eprintln!("  Dependents:");
            for dep in &dependents {
                eprintln!("    {}", dep.package_ref());
            }
            eprintln!(
                "  Use --recursive to remove dependents first, or remove dependents manually."
            );
            continue;
        }

        if ctx.cli.recursive && !dependents.is_empty() {
            println!(
                "{}",
                format!("Removing {} dependent package(s)", dependents.len()).yellow()
            );
            for dep in &dependents {
                if let Some(dep_id) = dep.id {
                    remove_package_files(ctx.config, dep, ctx.cli.nosave).await?;
                    db.remove_package(dep_id).await?;
                    println!("  Removed dependent: {}", dep.package_ref());
                }
            }
        }

        if !ctx.cli.noconfirm {
            let accepted = grel_cli::ask_confirmation(
                &format!("Remove {} v{}?", pkg.package_ref(), pkg.version),
                false,
            );
            if !accepted {
                continue;
            }
        }

        if ctx.cli.dry_run {
            println!(
                "  {}",
                format!("(dry-run: would remove {})", pkg.package_ref()).yellow()
            );
            println!("    install dir: {}", pkg.install_path);
            for bin in pkg.binary_list() {
                println!(
                    "    binary: {}",
                    ctx.config.paths.bin_dir.join(&bin).display()
                );
            }
            continue;
        }

        remove_package_files(ctx.config, &pkg, ctx.cli.nosave).await?;
        db.remove_package(id).await?;

        println!("{}", format!("Removed {}", pkg_ref.to_short_ref()).green());

        // --cascade: remove orphaned implicit packages
        if ctx.cli.clean {
            let mut graph = grel_core::DependencyGraph::new();
            let all_packages = db.list_packages().await?;
            let explicit: Vec<_> = all_packages
                .iter()
                .filter(|p| p.is_explicit)
                .map(|p| PackageRef::parse_with_forge(&p.package_ref(), Forge::GitHub).ok())
                .flatten()
                .collect();
            let installed: Vec<_> = all_packages
                .iter()
                .filter_map(|p| PackageRef::parse_with_forge(&p.package_ref(), Forge::GitHub).ok())
                .collect();

            for p in &all_packages {
                if let Ok(deps) = db.get_dependencies(p.id.unwrap_or(0)).await {
                    let from = PackageRef::parse_with_forge(&p.package_ref(), Forge::GitHub)?;
                    for dep in deps {
                        if let Some(to) = dep.as_grel_ref() {
                            graph.add_edge(from.clone(), to);
                        }
                        // System deps are skipped — they are not grel-removable
                    }
                }
            }

            let orphans = graph.find_orphans(&installed, &explicit);
            for orphan in orphans {
                if let Some(orphan_pkg) = db
                    .get_package(&orphan.forge.to_string(), &orphan.owner, &orphan.repo)
                    .await?
                {
                    if let Some(oid) = orphan_pkg.id {
                        println!("  Removing orphaned dependency: {}", orphan.to_short_ref());
                        remove_package_files(ctx.config, &orphan_pkg, ctx.cli.nosave).await?;
                        db.remove_package(oid).await?;
                    }
                }
            }
        }
    }

    db.close().await;
    Ok(())
}

/// Remove unneeded packages (-Ru)
pub async fn cmd_remove_unneeded(ctx: &CommandContext<'_>) -> Result<()> {
    let db = ctx.db().await?;
    let all_packages = db.list_packages().await?;

    // Find implicit packages with no dependents
    let mut to_remove = Vec::new();
    for pkg in &all_packages {
        if !pkg.is_explicit && matches!(pkg.status, models::PackageStatus::Active) {
            let dependents = db.get_dependents(&pkg.package_ref()).await?;
            if dependents.is_empty() {
                to_remove.push(pkg.clone());
            }
        }
    }

    if to_remove.is_empty() {
        println!("No unneeded packages to remove.");
        db.close().await;
        return Ok(());
    }

    // Sort in reverse dependency order using the dependency graph
    let mut graph = grel_core::DependencyGraph::new();
    let _installed_refs: Vec<_> = all_packages
        .iter()
        .filter_map(|p| PackageRef::parse_with_forge(&p.package_ref(), Forge::GitHub).ok())
        .collect();

    for p in &all_packages {
        if let Ok(deps) = db.get_dependencies(p.id.unwrap_or(0)).await {
            let from = PackageRef::parse_with_forge(&p.package_ref(), Forge::GitHub)?;
            for dep in deps {
                if let Some(to) = dep.as_grel_ref() {
                    graph.add_edge(from.clone(), to);
                }
                // System deps are skipped — they are not grel-removable
            }
        }
    }

    let sorted = graph.topological_sort()?;
    let sorted_refs: Vec<String> = sorted.iter().map(|r| r.to_short_ref()).collect();

    let mut ordered: Vec<_> = to_remove
        .into_iter()
        .map(|p| (p.package_ref(), p))
        .collect();
    ordered.sort_by_key(|(ref_str, _)| {
        sorted_refs
            .iter()
            .position(|r| r == ref_str)
            .unwrap_or(usize::MAX)
    });
    ordered.reverse(); // Remove dependents first

    println!(
        "{}",
        format!("Unneeded packages ({}):", ordered.len()).bold()
    );
    for (_, pkg) in &ordered {
        println!("  {} v{}", pkg.package_ref(), pkg.version);
    }
    println!();

    if !ctx.cli.noconfirm {
        let accepted = grel_cli::ask_confirmation("Remove these packages?", false);
        if !accepted {
            println!("Cancelled.");
            db.close().await;
            return Ok(());
        }
    }

    for (_, pkg) in ordered {
        if let Some(id) = pkg.id {
            remove_package_files(ctx.config, &pkg, ctx.cli.nosave).await?;
            db.remove_package(id).await?;
            println!("{}", format!("Removed {}", pkg.package_ref()).green());
        }
    }

    db.close().await;
    Ok(())
}

/// Remove a package's files from disk.
pub async fn remove_package_files(
    config: &grel_config::Config,
    pkg: &grel_cache::models::InstalledPackage,
    _nosave: bool,
) -> Result<()> {
    // Delete binaries from bin_dir
    for bin_name in pkg.binary_list() {
        let bin_path = config.paths.bin_dir.join(&bin_name);
        if bin_path.exists() {
            std::fs::remove_file(&bin_path).ok();
        }
    }

    // Delete the entire install directory
    let install_path = std::path::Path::new(&pkg.install_path);
    if install_path.exists() {
        if install_path.is_dir() {
            std::fs::remove_dir_all(install_path).ok();
        } else {
            std::fs::remove_file(install_path).ok();
        }
    }

    // NOTE: nosave is currently a no-op because grel does not yet implement
    // config file preservation (.grelnew). When that feature is added,
    // nosave=false should preserve config files while nosave=true removes everything.

    Ok(())
}
