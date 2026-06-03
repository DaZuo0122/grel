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

        let Some(id) = pkg.id else {
            eprintln!("{}", "Package record is missing an ID".red());
            continue;
        };

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
    nosave: bool,
) -> Result<()> {
    let install_path = std::path::Path::new(&pkg.install_path);

    // Run pre_remove hook if present and enabled
    if config.security.enable_hooks {
        let manifest_path = if install_path.is_dir() {
            install_path.join(".grel.toml")
        } else if let Some(parent) = install_path.parent() {
            parent.join(".grel.toml")
        } else {
            std::path::PathBuf::new()
        };

        if manifest_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                if let Ok(manifest) = grel_core::Manifest::load_from_str(&content) {
                    if let Some(hook) = manifest.hooks.resolved_pre_remove() {
                        let hook_dir = if install_path.is_dir() {
                            install_path.to_path_buf()
                        } else if let Some(parent) = install_path.parent() {
                            parent.to_path_buf()
                        } else {
                            std::path::PathBuf::from(".")
                        };
                        grel_core::run_hook(hook, &hook_dir, "pre_remove", config.security.allow_sh_hooks_on_windows);
                    }
                }
            }
        }
    }

    // Delete binaries from bin_dir
    for bin_name in pkg.binary_list() {
        let bin_path = config.paths.bin_dir.join(&bin_name);
        if bin_path.exists() {
            std::fs::remove_file(&bin_path).ok();
        }
    }

    // Delete the install path
    if install_path.exists() {
        if install_path.is_dir() {
            if !nosave {
                // Preserve config files before deletion
                preserve_config_files(install_path);
            }
            std::fs::remove_dir_all(install_path).ok();
        } else {
            std::fs::remove_file(install_path).ok();
        }
    }

    Ok(())
}

/// Preserve config files by copying them to `.grelnew` backups.
/// Only called when `nosave = false`.
fn preserve_config_files(install_path: &std::path::Path) {
    const CONFIG_EXTS: &[&str] = &["toml", "conf", "yaml", "yml", "json", "ini", "cfg"];

    let backup_dir = install_path.with_extension("grelnew");
    let mut preserved = 0;

    fn walk(
        dir: &std::path::Path,
        install_path: &std::path::Path,
        backup_dir: &std::path::Path,
        config_exts: &[&str],
        preserved: &mut usize,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, install_path, backup_dir, config_exts, preserved);
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if config_exts.contains(&ext) {
                    let relative = path.strip_prefix(install_path).unwrap_or(&path);
                    let dest = backup_dir.join(relative);
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    if std::fs::copy(&path, dest).is_ok() {
                        *preserved += 1;
                    }
                }
            }
        }
    }

    walk(install_path, install_path, &backup_dir, CONFIG_EXTS, &mut preserved);

    if preserved > 0 {
        println!(
            "  Preserved {preserved} config file(s) to {}",
            backup_dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grel_cache::models::{InstalledPackage, PackageStatus};

    fn make_temp_dir(label: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!("grel-remove-test-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    fn cleanup(tmp: &std::path::Path) {
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn remove_package_files_deletes_binaries_and_install_path() {
        let tmp = make_temp_dir("basic-remove");
        let bin_dir = tmp.join("bin");
        let install_dir = tmp.join("install");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&install_dir).unwrap();

        let bin_path = bin_dir.join("mytool");
        std::fs::File::create(&bin_path).unwrap();

        let marker = install_dir.join("marker");
        std::fs::File::create(&marker).unwrap();

        let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
        pkg.install_path = install_dir.to_string_lossy().to_string();
        pkg.set_binary_list(vec!["mytool".into()]);
        pkg.is_managed = true;
        pkg.status = PackageStatus::Active;

        let config = grel_config::Config::default();
        // We can't easily override bin_dir in Config::default(), so this test
        // only verifies the install_path deletion.
        remove_package_files(&config, &pkg, false).await.unwrap();

        assert!(!marker.exists(), "install directory should be removed");

        cleanup(&tmp);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn remove_package_files_runs_pre_remove_hook_when_enabled() {
        let tmp = make_temp_dir("pre-remove-hook");
        let install_dir = tmp.join("install");
        std::fs::create_dir_all(&install_dir).unwrap();

        let hook_marker = tmp.join("hook_ran");

        // Write a manifest with a pre_remove hook
        let manifest = grel_core::Manifest {
            name: "test".into(),
            description: "".into(),
            license: "".into(),
            source: Default::default(),
            assets: Default::default(),
            checksum_filename: Default::default(),
            signature_filename: Default::default(),
            signature_kind: Default::default(),
            dependencies: Default::default(),
            hooks: grel_core::HookSpec {
                pre_remove: Some(format!("touch {}", hook_marker.display())),
                ..Default::default()
            },
        };
        let manifest_path = install_dir.join(".grel.toml");
        std::fs::write(&manifest_path, manifest.to_toml().unwrap()).unwrap();

        let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
        pkg.install_path = install_dir.to_string_lossy().to_string();
        pkg.is_managed = true;
        pkg.status = PackageStatus::Active;

        let mut config = grel_config::Config::default();
        config.security.enable_hooks = true;
        remove_package_files(&config, &pkg, false).await.unwrap();

        assert!(hook_marker.exists(), "pre_remove hook should have created marker");

        cleanup(&tmp);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn remove_package_files_skips_pre_remove_hook_when_disabled() {
        let tmp = make_temp_dir("pre-remove-hook-disabled");
        let install_dir = tmp.join("install");
        std::fs::create_dir_all(&install_dir).unwrap();

        let hook_marker = tmp.join("hook_ran");

        // Write a manifest with a pre_remove hook
        let manifest = grel_core::Manifest {
            name: "test".into(),
            description: "".into(),
            license: "".into(),
            source: Default::default(),
            assets: Default::default(),
            checksum_filename: Default::default(),
            signature_filename: Default::default(),
            signature_kind: Default::default(),
            dependencies: Default::default(),
            hooks: grel_core::HookSpec {
                pre_remove: Some(format!("touch {}", hook_marker.display())),
                ..Default::default()
            },
        };
        let manifest_path = install_dir.join(".grel.toml");
        std::fs::write(&manifest_path, manifest.to_toml().unwrap()).unwrap();

        let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
        pkg.install_path = install_dir.to_string_lossy().to_string();
        pkg.is_managed = true;
        pkg.status = PackageStatus::Active;

        let mut config = grel_config::Config::default();
        config.security.enable_hooks = false; // disabled by default
        remove_package_files(&config, &pkg, false).await.unwrap();

        assert!(!hook_marker.exists(), "pre_remove hook should NOT run when disabled");

        cleanup(&tmp);
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn remove_package_files_runs_pre_remove_hook_on_windows() {
        let tmp = make_temp_dir("pre-remove-hook-win");
        let install_dir = tmp.join("install");
        std::fs::create_dir_all(&install_dir).unwrap();

        let hook_marker = tmp.join("hook_ran.txt");

        // Write a manifest with a pre_remove hook using PowerShell syntax
        let manifest = grel_core::Manifest {
            name: "test".into(),
            description: "".into(),
            license: "".into(),
            source: Default::default(),
            assets: Default::default(),
            checksum_filename: Default::default(),
            signature_filename: Default::default(),
            signature_kind: Default::default(),
            dependencies: Default::default(),
            hooks: grel_core::HookSpec {
                pre_remove: Some(format!(
                    "Write-Output 'test' | Out-File -FilePath '{}'",
                    hook_marker.display()
                )),
                ..Default::default()
            },
        };
        let manifest_path = install_dir.join(".grel.toml");
        std::fs::write(&manifest_path, manifest.to_toml().unwrap()).unwrap();

        let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
        pkg.install_path = install_dir.to_string_lossy().to_string();
        pkg.is_managed = true;
        pkg.status = PackageStatus::Active;

        let mut config = grel_config::Config::default();
        config.security.enable_hooks = true;
        remove_package_files(&config, &pkg, false).await.unwrap();

        assert!(hook_marker.exists(), "pre_remove hook should have created marker on Windows");

        cleanup(&tmp);
    }
}
