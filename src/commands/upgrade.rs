//! Local file install commands (-U operation)

use anyhow::{Context, Result};
use grel_core::PackageRef;
use owo_colors::OwoColorize;

use crate::commands::CommandContext;

/// Install from a local file (-U)
pub async fn cmd_upgrade_local(ctx: &CommandContext<'_>) -> Result<()> {
    let targets = &ctx.cli.targets;

    // Get file path from --asset or first target
    let file_path = if let Some(ref path) = ctx.cli.asset {
        std::path::PathBuf::from(path)
    } else if let Some(first) = targets.first() {
        std::path::PathBuf::from(first)
    } else {
        eprintln!("No file specified. Usage: grel -U ./package.tar.gz");
        return Ok(());
    };

    if !file_path.exists() {
        eprintln!(
            "{}",
            format!("File not found: {}", file_path.display()).red()
        );
        return Ok(());
    }

    let filename = file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?;

    println!("  Installing from local file: {}", filename.bold());

    let (pkg_ref_str, asset_path) = if targets.len() >= 2 {
        (targets[0].clone(), file_path)
    } else if ctx.cli.asset.is_some() && !targets.is_empty() {
        (targets[0].clone(), file_path)
    } else {
        let stem = std::path::Path::new(&filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".into());
        (format!("local/{stem}"), file_path)
    };

    let pkg_ref = PackageRef::parse_with_forge(&pkg_ref_str, ctx.cli.default_forge.0)
        .with_context(|| format!("Invalid package reference: {pkg_ref_str}"))?;

    let db = ctx.db().await?;

    let existing = db
        .get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo)
        .await?;
    if let Some(ref existing_pkg) = existing {
        if !ctx.cli.noconfirm {
            let accepted = grel_cli::ask_confirmation(
                &format!(
                    "{} v{} is already installed. Reinstall?",
                    pkg_ref.to_short_ref(),
                    existing_pkg.version
                ),
                false,
            );
            if !accepted {
                println!("Cancelled.");
                db.close().await;
                return Ok(());
            }
        }

        let old_install_dir = std::path::Path::new(&existing_pkg.install_path);
        if old_install_dir.exists() {
            if old_install_dir.is_dir() {
                std::fs::remove_dir_all(old_install_dir).ok();
            } else {
                std::fs::remove_file(old_install_dir).ok();
            }
        }
        for bin_name in existing_pkg.binary_list() {
            std::fs::remove_file(ctx.config.paths.bin_dir.join(&bin_name)).ok();
        }
    }

    let install_dir = ctx.config.paths.install_root.join(format!(
        "{}/{}/{}",
        pkg_ref.forge, pkg_ref.owner, pkg_ref.repo
    ));

    if ctx.cli.dry_run {
        println!(
            "  {}",
            format!(
                "(dry-run: would install {filename} to {})",
                install_dir.display()
            )
            .yellow()
        );
        db.close().await;
        return Ok(());
    }

    std::fs::create_dir_all(&install_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create directory: {e}"))?;

    let dest_path = install_dir.join(&filename);
    std::fs::copy(&asset_path, &dest_path)
        .map_err(|e| anyhow::anyhow!("Failed to copy file: {e}"))?;

    let checksum = {
        use sha2::Digest;
        use std::io::Read;
        let mut hasher = sha2::Sha256::new();
        let mut file = std::fs::File::open(&dest_path)?;
        let mut buf = [0u8; 8192];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        format!("{:x}", hasher.finalize())
    };

    let install_result = grel_network::archive::install_asset(
        &dest_path,
        &install_dir,
        &ctx.config.paths.bin_dir,
        &filename,
        ctx.cli.overwrite,
    );

    let (installed_binaries, is_managed) = match install_result {
        Ok(result) => {
            let bins: Vec<String> = result
                .installed_binaries
                .iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .collect();
            if !result.is_plain_binary {
                println!(
                    "  {}",
                    format!("Extracted {} binary(s)", bins.len()).green()
                );
            } else {
                println!("  {}", "Installed binary".green());
            }
            (bins, true)
        }
        Err(_) => {
            println!(
                "  {}",
                "Warning: Could not extract, keeping as unmanaged".yellow()
            );
            (vec![], false)
        }
    };

    if !ctx.config.general.keep_archives {
        std::fs::remove_file(&dest_path).ok();
    }

    let mut pkg = grel_cache::models::InstalledPackage::new(
        pkg_ref.forge.to_string(),
        pkg_ref.owner.clone(),
        pkg_ref.repo.clone(),
    );
    pkg.version = "local".into();
    pkg.asset_filename = filename;
    pkg.checksum = Some(checksum);
    pkg.install_path = if is_managed {
        install_dir.to_string_lossy().to_string()
    } else {
        dest_path.to_string_lossy().to_string()
    };
    pkg.set_binary_list(installed_binaries);
    pkg.is_managed = is_managed;
    pkg.status = grel_cache::models::PackageStatus::Active;
    pkg.manifest_source = grel_cache::models::ManifestSource::Heuristic;

    db.upsert_package(&pkg).await?;

    println!(
        "  {}",
        format!("Installed {} from local file", pkg_ref.to_short_ref()).green()
    );

    db.close().await;
    Ok(())
}
