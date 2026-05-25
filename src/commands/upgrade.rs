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

    // Create crash-recovery journal before extraction
    let mut journal = crate::commands::journal::JournalEntry::new(
        crate::commands::journal::Operation::Install,
        &pkg_ref.forge.to_string(),
        &pkg_ref.owner,
        &pkg_ref.repo,
    );
    journal.install_dir = Some(install_dir.clone());
    journal.bin_dir = Some(ctx.config.paths.bin_dir.clone());
    journal.archive_path = Some(install_dir.join(&filename));
    journal.is_managed = true;
    journal.write()?;

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
            // Delete old install now that new one succeeded
            if let Some(ref existing_pkg) = existing {
                let old_install_dir = std::path::Path::new(&existing_pkg.install_path);
                if old_install_dir.exists() {
                    if old_install_dir.is_dir() {
                        let _ = std::fs::remove_dir_all(old_install_dir);
                    } else {
                        let _ = std::fs::remove_file(old_install_dir);
                    }
                }
                for bin_name in existing_pkg.binary_list() {
                    let _ = std::fs::remove_file(ctx.config.paths.bin_dir.join(&bin_name));
                }
            }

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
        Err(e) => {
            // Clean up partial install
            if install_dir.exists() {
                let _ = std::fs::remove_dir_all(&install_dir);
            }
            return Err(anyhow::anyhow!("Failed to extract local archive: {e}"));
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

    match db.begin_transaction().await {
        Ok(mut tx) => {
            tx.upsert_package(&pkg).await?;
            tx.commit().await?;
            // Crash-recovery: mark operation complete
            journal.commit()?;
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to begin database transaction: {e}"));
        }
    }

    println!(
        "  {}",
        format!("Installed {} from local file", pkg_ref.to_short_ref()).green()
    );

    db.close().await;
    Ok(())
}
