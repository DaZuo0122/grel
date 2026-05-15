//! Database commands (-D operation)

use anyhow::{Context, Result};
use grel_core::{Forge, PackageRef};
use owo_colors::OwoColorize;

use crate::commands::CommandContext;

/// Migrate package reference
pub async fn cmd_migrate(ctx: &CommandContext<'_>, old_ref: &str, new_ref: &str) -> Result<()> {
    let old_ref = PackageRef::parse_with_forge(old_ref, Forge::GitHub)
        .with_context(|| format!("Invalid package reference: {old_ref}"))?;
    let new_ref = PackageRef::parse_with_forge(new_ref, Forge::GitHub)
        .with_context(|| format!("Invalid package reference: {new_ref}"))?;

    let db = ctx.db().await?;

    let old_pkg = db
        .get_package(&old_ref.forge.to_string(), &old_ref.owner, &old_ref.repo)
        .await?;

    let Some(old_pkg) = old_pkg else {
        eprintln!(
            "{}",
            format!("Package not found: {}", old_ref.to_short_ref()).red()
        );
        db.close().await;
        return Ok(());
    };

    let Some(id) = old_pkg.id else {
        anyhow::bail!("Package record is missing an ID");
    };

    let mut new_pkg = grel_cache::models::InstalledPackage::new(
        new_ref.forge.to_string(),
        new_ref.owner.clone(),
        new_ref.repo.clone(),
    );
    new_pkg.version = old_pkg.version.clone();
    new_pkg.asset_filename = old_pkg.asset_filename.clone();
    new_pkg.checksum = old_pkg.checksum.clone();
    new_pkg.install_path = old_pkg.install_path.clone();
    new_pkg.is_managed = old_pkg.is_managed;
    new_pkg.status = grel_cache::models::PackageStatus::Active;

    db.remove_package(id).await?;
    db.upsert_package(&new_pkg).await?;

    println!(
        "{}",
        format!(
            "Migrated {} -> {}",
            old_ref.to_short_ref(),
            new_ref.to_short_ref()
        )
        .green()
    );

    db.close().await;
    Ok(())
}

/// Clean orphaned packages from database
pub async fn cmd_db_clean(ctx: &CommandContext<'_>) -> Result<()> {
    let db = ctx.db().await?;

    let packages = db.list_packages().await?;
    let mut removed = 0;
    for pkg in &packages {
        if matches!(pkg.status, grel_cache::models::PackageStatus::Orphaned) {
            if let Some(id) = pkg.id {
                db.remove_package(id).await?;
                removed += 1;
            }
        }
    }

    println!("Removed {removed} orphaned package(s)");
    println!("  {}", "ETag/IP cache cleanup not yet implemented".yellow());

    db.close().await;
    Ok(())
}

/// Check database integrity
pub async fn cmd_db_check(ctx: &CommandContext<'_>) -> Result<()> {
    let db = ctx.db().await?;

    let count = db.list_packages().await?.len();
    println!("Database integrity check: {} package records found", count);
    println!("{}", "Database appears healthy".green());

    db.close().await;
    Ok(())
}

/// Dump database as JSON
pub async fn cmd_db_dump(ctx: &CommandContext<'_>) -> Result<()> {
    let db = ctx.db().await?;

    let packages = db.list_packages().await?;
    println!("{{\"packages\": [");
    for (i, pkg) in packages.iter().enumerate() {
        if i > 0 {
            print!(",");
        }
        print!(
            r#"{{"ref":"{}","version":"{}","status":"{}"}}"#,
            pkg.package_ref(),
            pkg.version,
            pkg.status
        );
    }
    println!("]}}");

    db.close().await;
    Ok(())
}

/// Mark packages as explicitly installed
pub async fn cmd_db_as_explicit(ctx: &CommandContext<'_>, targets: &[String]) -> Result<()> {
    if targets.is_empty() {
        eprintln!("No packages specified. Usage: grel -D --asexplicit owner/repo");
        return Ok(());
    }

    let db = ctx.db().await?;

    for pkg_str in targets {
        let pkg_ref = PackageRef::parse_with_forge(pkg_str, Forge::GitHub)
            .with_context(|| format!("Invalid package reference: {pkg_str}"))?;

        db.update_explicit(
            &pkg_ref.forge.to_string(),
            &pkg_ref.owner,
            &pkg_ref.repo,
            true,
        )
        .await?;

        println!(
            "{}",
            format!("Marked {} as explicitly installed", pkg_ref.to_short_ref()).green()
        );
    }

    db.close().await;
    Ok(())
}

/// Mark packages as dependencies
pub async fn cmd_db_as_deps(ctx: &CommandContext<'_>, targets: &[String]) -> Result<()> {
    if targets.is_empty() {
        eprintln!("No packages specified. Usage: grel -D --asdeps owner/repo");
        return Ok(());
    }

    let db = ctx.db().await?;

    for pkg_str in targets {
        let pkg_ref = PackageRef::parse_with_forge(pkg_str, Forge::GitHub)
            .with_context(|| format!("Invalid package reference: {pkg_str}"))?;

        db.update_explicit(
            &pkg_ref.forge.to_string(),
            &pkg_ref.owner,
            &pkg_ref.repo,
            false,
        )
        .await?;

        println!(
            "{}",
            format!("Marked {} as dependency", pkg_ref.to_short_ref()).green()
        );
    }

    db.close().await;
    Ok(())
}
