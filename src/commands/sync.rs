//! Sync / Install commands (-S operation)

use std::io;

use anyhow::{Context, Result};
use grel_cache::{Database, models};
use grel_config::Config;
use grel_core::Forge;
use grel_core::{Arch, ChecksumVerifier, Manifest, Os, PackageRef, RemoteAsset, ResolverConfig, SignatureVerifier};
use grel_network::{Client, build_http_client};
use grel_providers::ProviderRegistry;
use owo_colors::OwoColorize;

use crate::commands::{CommandContext, format_size, is_asset_managed};

/// Run ELF dependency resolution on newly installed binaries and offer to install missing ones.
///
/// Gracefully degrades: any failure just prints a warning and continues.
#[cfg(target_os = "linux")]
async fn check_elf_deps(bin_paths: &[std::path::PathBuf], ctx: &CommandContext<'_>, db: &Database) {
    if !ctx.config.elf_deps.auto_resolve_system_deps || bin_paths.is_empty() {
        return;
    }

    let report = grel_elf::resolve_elf_deps(bin_paths, &ctx.config.elf_deps, Some(db)).await;

    if ctx.config.elf_deps.show_parsed_deps && !report.missing_libs.is_empty() {
        println!(
            "  {}",
            format!(
                "Missing system libraries: {}",
                report.missing_libs.join(", ")
            )
            .yellow()
        );
    }

    if report.unresolved_libs.len() > 0 {
        for lib in &report.unresolved_libs {
            println!(
                "  {}",
                format!("Could not resolve package for: {lib}").yellow()
            );
        }
    }

    let Some(install_cmd) = report.install_cmd else {
        return;
    };

    let cmd_str = install_cmd.join(" ");
    println!("  {}", format!("Suggested: {cmd_str}").cyan());

    if ctx.cli.noconfirm {
        run_install_cmd(&install_cmd);
    } else if grel_cli::is_interactive() {
        let accepted = grel_cli::ask_confirmation("Install missing system libraries?", false);
        if accepted {
            run_install_cmd(&install_cmd);
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn check_elf_deps(
    _bin_paths: &[std::path::PathBuf],
    _ctx: &CommandContext<'_>,
    _db: &Database,
) {
}

#[cfg(target_os = "linux")]
fn run_install_cmd(tokens: &[String]) {
    if tokens.is_empty() {
        return;
    }
    let status = std::process::Command::new(&tokens[0])
        .args(&tokens[1..])
        .status();
    match status {
        Ok(s) if s.success() => println!("  {}", "System libraries installed.".green()),
        Ok(s) => eprintln!(
            "  {}",
            format!("Install command exited with status {s}").red()
        ),
        Err(e) => eprintln!("  {}", format!("Failed to run install command: {e}").red()),
    }
}

/// Three-tier manifest lookup:
/// 1. Central registry (local cache)
/// 2. In-repo `.grel.toml` via raw content API
/// 3. Return None (fall back to heuristic)
async fn resolve_manifest(
    pkg_ref: &PackageRef,
    registry: &grel_core::Registry,
    provider: &dyn grel_providers::ReleaseProvider,
) -> Option<(grel_core::Manifest, grel_cache::models::ManifestSource)> {
    // Tier 1: Central registry
    if registry.is_available() {
        if let Ok(Some(manifest)) =
            registry.get_manifest(&pkg_ref.forge, &pkg_ref.owner, &pkg_ref.repo)
        {
            tracing::debug!("Found manifest in registry for {}", pkg_ref.to_short_ref());
            return Some((manifest, grel_cache::models::ManifestSource::Registry));
        }
    }

    // Tier 2: In-repo .grel.toml
    for branch in &["main", "master", "HEAD"] {
        match provider
            .fetch_raw_file(&pkg_ref.owner, &pkg_ref.repo, ".grel.toml", branch)
            .await
        {
            Ok(content) => match grel_core::Manifest::load_from_str(&content) {
                Ok(manifest) => {
                    tracing::debug!("Found in-repo manifest for {}", pkg_ref.to_short_ref());
                    return Some((manifest, grel_cache::models::ManifestSource::InRepo));
                }
                Err(e) => {
                    tracing::warn!("Invalid .grel.toml in {}: {}", pkg_ref.to_short_ref(), e);
                }
            },
            Err(grel_providers::ProviderError::NotFound(_)) => continue,
            Err(e) => {
                tracing::debug!(
                    "Failed to fetch .grel.toml for {}: {}",
                    pkg_ref.to_short_ref(),
                    e
                );
            }
        }
    }

    None
}

/// Check whether a package should be skipped because it is already up-to-date.
async fn check_needed_skip(
    db: &grel_cache::Database,
    forge: &str,
    owner: &str,
    repo: &str,
    target_version: &str,
) -> Result<bool> {
    if let Some(existing) = db.get_package(forge, owner, repo).await? {
        if existing.version == target_version {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Save manifest to the package install directory for later use (e.g., pre_remove hooks).
pub(crate) fn save_manifest_to_dir(
    manifest: &grel_core::Manifest,
    install_dir: &std::path::Path,
) -> Result<()> {
    let manifest_path = install_dir.join(".grel.toml");
    let content = manifest.to_toml().map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(&manifest_path, content)
        .with_context(|| format!("Failed to write manifest to {}", manifest_path.display()))?;
    Ok(())
}

/// Run a hook script from the package install directory.
pub(crate) fn run_hook(hook: &str, install_dir: &std::path::Path, label: &str) {
    println!("  Running {label} hook...");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(hook)
        .current_dir(install_dir)
        .status();
    match status {
        Ok(s) if s.success() => println!("  {}", format!("{label} hook completed").green()),
        Ok(s) => eprintln!(
            "  {}",
            format!("{label} hook exited with status {s}").yellow()
        ),
        Err(e) => eprintln!(
            "  {}",
            format!("Failed to run {label} hook: {e}").yellow()
        ),
    }
}

/// Download and parse the upstream checksum file for an asset.
/// Returns `Some(expected_hash)` when verification is enabled and a checksum file is found.
async fn fetch_expected_checksum(
    client: &Client,
    release: &grel_providers::Release,
    asset: &RemoteAsset,
    config: &Config,
    manifest: Option<&Manifest>,
) -> Result<Option<String>, anyhow::Error> {
    if !config.security.verify_checksums {
        return Ok(None);
    }

    let Some((checksum_asset, _algo)) =
        ChecksumVerifier::find_checksum_asset(&release.assets, asset, manifest)
    else {
        return Err(anyhow::anyhow!(
            "Checksum verification enabled but no checksum file found for {}",
            asset.filename
        ));
    };

    println!(
        "  {}",
        format!("Verifying checksum from {}", checksum_asset.filename)
            .dimmed()
    );

    let temp_path = std::env::temp_dir().join(&checksum_asset.filename);
    let (_, _) = grel_network::download::download_file(
        client,
        &checksum_asset.url,
        &temp_path,
        None,
        false,
        None,
    )
    .await
    .with_context(|| {
        format!(
            "Failed to download checksum file {}",
            checksum_asset.filename
        )
    })?;

    let content = tokio::fs::read_to_string(&temp_path)
        .await
        .with_context(|| {
            format!(
                "Failed to read checksum file {}",
                temp_path.display()
            )
        })?;

    tokio::fs::remove_file(&temp_path).await.ok();

    let expected = ChecksumVerifier::parse_checksum(&content, &asset.filename)
        .with_context(|| {
            format!(
                "Failed to parse checksum file {}",
                checksum_asset.filename
            )
        })?;

    Ok(Some(expected))
}

/// Verify a downloaded archive against an expected checksum.
/// On mismatch, deletes the archive and returns an error.
fn verify_checksum_or_clean(
    computed: &str,
    expected: &str,
    archive_path: &std::path::Path,
    filename: &str,
) -> Result<(), anyhow::Error> {
    if let Err(e) = ChecksumVerifier::verify(computed, expected) {
        std::fs::remove_file(archive_path).ok();
        return Err(anyhow::anyhow!(
            "Checksum verification failed for {}: {}",
            filename,
            e
        ));
    }
    println!("  {}", "Checksum verified".dimmed());
    Ok(())
}

/// Download a signature file and cryptographically verify `archive_path`.
async fn verify_signature_for_asset(
    client: &Client,
    release: &grel_providers::Release,
    asset: &RemoteAsset,
    archive_path: &std::path::Path,
    security: &grel_config::SecurityConfig,
    manifest: Option<&grel_core::Manifest>,
) -> Result<(), anyhow::Error> {
    let Some((sig_asset, kind)) =
        SignatureVerifier::find_signature_asset(&release.assets, asset, manifest)
    else {
        return Err(anyhow::anyhow!(
            "No signature file found for {}",
            asset.filename
        ));
    };

    println!(
        "  {}",
        format!("Verifying signature from {}", sig_asset.filename)
            .dimmed()
    );

    // Download signature file to temp
    let sig_temp = std::env::temp_dir().join(&sig_asset.filename);
    let (_, _) = grel_network::download::download_file(
        client,
        &sig_asset.url,
        &sig_temp,
        None,
        false,
        None,
    )
    .await
    .with_context(|| {
        format!(
            "Failed to download signature {}",
            sig_asset.filename
        )
    })?;

    let sig_bytes = tokio::fs::read(&sig_temp).await?;
    tokio::fs::remove_file(&sig_temp).await.ok();

    // Read the message (downloaded archive)
    let message_bytes = tokio::fs::read(archive_path).await?;

    match kind {
        grel_core::SignatureKind::GpgAsc | grel_core::SignatureKind::GpgBinary => {
            if security.trusted_pgp_keys.is_empty() {
                return Err(anyhow::anyhow!(
                    "Signature verification requires trusted_pgp_keys in config"
                ));
            }
            SignatureVerifier::verify_gpg(
                &sig_bytes,
                &message_bytes,
                &security.trusted_pgp_keys,
            )
            .with_context(|| "GPG signature verification failed")?;
        }
        grel_core::SignatureKind::Minisign => {
            let Some(ref pk) = security.minisign_public_key else {
                return Err(anyhow::anyhow!(
                    "Minisign signature verification requires minisign_public_key in config"
                ));
            };
            SignatureVerifier::verify_minisign(&sig_bytes, &message_bytes, pk)
                .with_context(|| "Minisign signature verification failed")?;
        }
    }

    Ok(())
}

/// Clean stale archives under a managed package directory.
/// Returns (removed_count, failed_count).
pub(crate) fn clean_package_dir(
    pkg_dir: &std::path::Path,
    expected_archive: &std::path::Path,
) -> (usize, usize) {
    let mut removed = 0;
    let mut failed = 0;

    let Ok(entries) = std::fs::read_dir(pkg_dir) else {
        return (removed, failed);
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path != expected_archive {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let is_archive = matches!(ext, "gz" | "xz" | "zip" | "bz2" | "zst" | "7z")
                || name.ends_with(".tar.gz")
                || name.ends_with(".tar.xz")
                || name.ends_with(".tar.bz2")
                || name.ends_with(".tar.zst");
            if is_archive {
                match std::fs::remove_file(&path) {
                    Ok(_) => removed += 1,
                    Err(_) => failed += 1,
                }
            }
        }
    }

    (removed, failed)
}

/// Install a single package (used by cmd_sync and dependency resolution).
#[allow(clippy::too_many_arguments)]
async fn install_single_package(
    config: &Config,
    pkg_ref: &PackageRef,
    provider_registry: &ProviderRegistry,
    local_registry: &grel_core::Registry,
    db: &Database,
    client: &grel_network::Client,
    resolver_config: &ResolverConfig,
    host_os: &Os,
    host_arch: &Arch,
    noconfirm: bool,
    allow_keyword: bool,
    dry_run: bool,
    is_explicit: bool,
    overwrite: bool,
    download_only: bool,
    needed: bool,
) -> Result<i64, anyhow::Error> {
    println!("\n{}", format!("→ {}", pkg_ref.to_short_ref()).bold());

    let provider = provider_registry
        .get_provider(&pkg_ref.forge)
        .with_context(|| format!("Failed to get provider for {}", pkg_ref.forge))?;

    let release = if let Some(ref tag) = pkg_ref.version {
        println!("  Pinning to version {tag}");
        match provider
            .get_release(&pkg_ref.owner, &pkg_ref.repo, tag)
            .await
        {
            Ok(r) => r,
            Err(grel_providers::ProviderError::NotFound(e)) => {
                return Err(anyhow::anyhow!("Release not found: {e}"));
            }
            Err(grel_providers::ProviderError::RateLimitExceeded) => {
                return Err(anyhow::anyhow!(
                    "Rate limit exceeded. Set GREL_GITHUB_TOKEN for higher limits."
                ));
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to fetch release: {e}"));
            }
        }
    } else {
        match provider.latest_release(&pkg_ref.owner, &pkg_ref.repo).await {
            Ok(r) => r,
            Err(grel_providers::ProviderError::NotFound(e)) => {
                return Err(anyhow::anyhow!("Package not found: {e}"));
            }
            Err(grel_providers::ProviderError::RateLimitExceeded) => {
                return Err(anyhow::anyhow!(
                    "Rate limit exceeded. Set GREL_GITHUB_TOKEN for higher limits."
                ));
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to fetch release: {e}"));
            }
        }
    };

    println!(
        "  Found release {} v{}",
        if release.name.is_empty() {
            &release.tag
        } else {
            &release.name
        },
        release.tag
    );

    if release.prerelease {
        println!("  {}", "Warning: This is a pre-release".yellow());
    }

    let remote_assets: Vec<RemoteAsset> = release.assets.clone();

    let manifest = resolve_manifest(pkg_ref, local_registry, provider).await;
    let manifest_source = manifest
        .as_ref()
        .map(|(_, s)| s.clone())
        .unwrap_or(models::ManifestSource::Heuristic);

    let manifest_asset = manifest.as_ref().and_then(|(m, _src)| {
        m.resolve_asset(host_os, host_arch, &release.tag)
            .and_then(|mapping| {
                let concrete_name = mapping.concrete_filename(&release.tag);
                remote_assets
                    .iter()
                    .find(|a| a.filename == concrete_name)
                    .cloned()
            })
    });

    let chosen_asset = if let Some(asset) = manifest_asset {
        println!(
            "  {}",
            format!("Using manifest asset: {}", asset.filename).dimmed()
        );
        asset
    } else {
        if manifest.is_some() {
            eprintln!(
                "  {}",
                "Warning: Manifest asset not found in release, falling back to heuristic.".yellow()
            );
        }

        let Some(mut sel) = grel_core::resolve_assets_detailed(
            &remote_assets,
            host_os,
            host_arch,
            resolver_config,
            allow_keyword,
        ) else {
            return Err(anyhow::anyhow!(
                "No compatible assets for {host_os}/{host_arch}"
            ));
        };

        let is_managed_default = is_asset_managed(&sel.default, &config.assets);
        sel.default_is_managed = is_managed_default;

        if !is_explicit && !noconfirm && !grel_cli::is_interactive() {
            sel.default.clone()
        } else {
            match confirm_asset_selection(&sel, config, noconfirm, allow_keyword) {
                Some(asset) => asset,
                None => {
                    return Err(anyhow::anyhow!("Skipped by user"));
                }
            }
        }
    };

    // Fail-fast if signature verification is enabled but no signature file is available
    if config.security.verify_signatures {
        if SignatureVerifier::find_signature_asset(
            &release.assets,
            &chosen_asset,
            manifest.as_ref().map(|(m, _)| m),
        )
        .is_none()
        {
            return Err(anyhow::anyhow!(
                "Signature verification enabled but no signature file found for {}",
                chosen_asset.filename
            ));
        }
    }

    let is_managed = if download_only {
        false
    } else {
        is_asset_managed(&chosen_asset, &config.assets)
    };

    if needed {
        match check_needed_skip(db, &pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo, &release.tag).await {
            Ok(true) => {
                println!(
                    "  {}",
                    format!(
                        "{} {} is up-to-date -- skipping",
                        pkg_ref.to_short_ref(),
                        release.tag
                    )
                    .dimmed()
                );
                return Ok(db
                    .get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo)
                    .await?
                    .and_then(|p| p.id)
                    .unwrap_or(-1));
            }
            Err(e) => tracing::warn!("Failed to check needed skip: {e}"),
            _ => {}
        }
    }

    let install_dir = if is_managed {
        config.paths.install_root.join(format!(
            "{}/{}/{}",
            pkg_ref.forge, pkg_ref.owner, pkg_ref.repo
        ))
    } else {
        config.paths.download_dir.clone()
    };
    let archive_path = install_dir.join(&chosen_asset.filename);

    if dry_run {
        println!(
            "  {}",
            format!("(dry-run: would download {})", chosen_asset.filename).yellow()
        );
        return Ok(-1);
    }

    std::fs::create_dir_all(&install_dir).map_err(|e| {
        anyhow::anyhow!(
            "Failed to create directory '{}': {e}",
            install_dir.display()
        )
    })?;

    println!(
        "  Downloading: {} ({})",
        chosen_asset.filename,
        format_size(chosen_asset.size_bytes.unwrap_or(0))
    );

    // Download checksum file if verification is enabled
    let expected_checksum = fetch_expected_checksum(
        client,
        &release,
        &chosen_asset,
        config,
        manifest.as_ref().map(|(m, _)| m),
    )
    .await?;

    // Check for cached ETag
    let cached_etag = db.get_etag(&chosen_asset.url).await.ok().flatten();

    let (checksum, response_etag) =
        grel_network::download::download_file(client, &chosen_asset.url, &archive_path, None, true, cached_etag.as_deref())
            .await
            .with_context(|| format!("Failed to download {}", chosen_asset.filename))?;

    // Verify checksum
    if let Some(expected) = expected_checksum {
        verify_checksum_or_clean(&checksum, &expected, &archive_path, &chosen_asset.filename)?;
    }

    // Verify cryptographic signature
    if config.security.verify_signatures {
        if let Err(e) = verify_signature_for_asset(
            client,
            &release,
            &chosen_asset,
            &archive_path,
            &config.security,
            manifest.as_ref().map(|(m, _)| m),
        )
        .await
        {
            std::fs::remove_file(&archive_path).ok();
            return Err(anyhow::anyhow!(
                "Signature verification failed for {}: {}",
                chosen_asset.filename,
                e
            ));
        }
        println!("  {}", "Signature verified".dimmed());
    }

    // Store ETag for future conditional requests
    if let Some(etag) = response_etag {
        db.store_etag(&chosen_asset.url, &etag).await.ok();
    }

    let installed_binaries = if is_managed {
        match grel_network::archive::install_asset(
            &archive_path,
            &install_dir,
            &config.paths.bin_dir,
            &chosen_asset.filename,
            overwrite,
        ) {
            Ok(result) => {
                if !result.installed_binaries.is_empty() {
                    println!(
                        "  {}",
                        format!(
                            "Installed {} binary(s) to {}",
                            result.installed_binaries.len(),
                            config.paths.bin_dir.display()
                        )
                        .green()
                    );
                } else if result.is_plain_binary {
                    println!("  {}", "Installed binary".green());
                }
                result.installed_binaries
            }
            Err(e) => {
                eprintln!("  {}", format!("Warning: Failed to extract: {e}").yellow());
                vec![]
            }
        }
    } else {
        vec![]
    };

    let bin_filenames: Vec<String> = installed_binaries
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();

    // Save manifest and run post_install hook
    if let Some((ref manifest, _)) = manifest {
        if let Err(e) = save_manifest_to_dir(manifest, &install_dir) {
            tracing::warn!("Failed to save manifest: {e}");
        }
        if !download_only && config.security.enable_hooks {
            if let Some(ref hook) = manifest.hooks.post_install {
                run_hook(hook, &install_dir, "post_install");
            }
        }
    }

    if is_managed && !config.general.keep_archives && archive_path.exists() {
        std::fs::remove_file(&archive_path).ok();
        println!("  {}", "Cleaned up downloaded archive".dimmed());
    }

    let mut pkg = models::InstalledPackage::new(
        pkg_ref.forge.to_string(),
        pkg_ref.owner.clone(),
        pkg_ref.repo.clone(),
    );
    pkg.version = release.tag.clone();
    pkg.asset_filename = chosen_asset.filename.clone();
    pkg.checksum = Some(checksum);
    pkg.install_path = if is_managed {
        install_dir.to_string_lossy().to_string()
    } else {
        archive_path.to_string_lossy().to_string()
    };
    pkg.set_binary_list(bin_filenames);
    pkg.is_managed = is_managed;
    pkg.status = models::PackageStatus::Active;
    pkg.manifest_source = manifest_source;
    pkg.is_explicit = is_explicit;

    db.upsert_package(&pkg)
        .await
        .with_context(|| "Failed to update package record")?;

    let updated_pkg = db
        .get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Package not found after upsert"))?;

    // Record extracted files in the package_files index
    if is_managed {
        if let Some(pkg_id) = updated_pkg.id {
            let files = collect_package_files(&install_dir, &config.paths.bin_dir, &archive_path);
            let file_paths: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();

            // Conflict detection: check if any files already belong to other packages
            if let Ok(conflicts) = db.find_conflicting_files(&file_paths).await {
                let other_conflicts: Vec<_> = conflicts
                    .into_iter()
                    .filter(|(pkg, _)| pkg.id != Some(pkg_id))
                    .collect();
                if !other_conflicts.is_empty() {
                    eprintln!(
                        "  {}",
                        format!("Warning: {} file(s) conflict with other packages:", other_conflicts.len()).yellow()
                    );
                    let mut shown = std::collections::HashSet::new();
                    for (conflict_pkg, conflict_path) in &other_conflicts {
                        let key = format!("{} -> {}", conflict_pkg.package_ref(), conflict_path);
                        if shown.insert(key.clone()) {
                            eprintln!("    {} owns '{}'", conflict_pkg.package_ref(), conflict_path);
                        }
                    }
                }
            }

            let file_models: Vec<grel_cache::models::PackageFile> = files
                .into_iter()
                .map(|(path, ftype)| grel_cache::models::PackageFile::new(pkg_id, path, ftype))
                .collect();
            if let Err(e) = db.set_package_files(pkg_id, &file_models).await {
                tracing::warn!("Failed to record package files: {e}");
            }
        }
    }

    if download_only {
        println!(
            "  {}",
            format!("Downloaded {} v{}", pkg_ref.to_short_ref(), release.tag).green()
        );
    } else {
        println!(
            "  {}",
            format!("Installed {} v{}", pkg_ref.to_short_ref(), release.tag).green()
        );
    }

    Ok(updated_pkg.id.unwrap_or(-1))
}

/// Walk the install directory and collect all files with type hints.
fn collect_package_files(
    install_dir: &std::path::Path,
    bin_dir: &std::path::Path,
    archive_path: &std::path::Path,
) -> Vec<(String, String)> {
    let mut files = Vec::new();

    // Collect all files under install_dir
    fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else {
                files.push(path);
            }
        }
    }

    let mut found = Vec::new();
    walk(install_dir, &mut found);

    for path in found {
        let Some(abs_str) = path.to_str() else { continue };
        let ftype = classify_file_type(&path, bin_dir, archive_path);
        files.push((abs_str.to_string(), ftype));
    }

    files
}

/// Classify a file into a type hint.
fn classify_file_type(
    path: &std::path::Path,
    bin_dir: &std::path::Path,
    archive_path: &std::path::Path,
) -> String {
    if path == archive_path {
        return "archive".into();
    }

    // Check if this file is a linked binary (resides in bin_dir)
    if path.starts_with(bin_dir) {
        return "binary".into();
    }

    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return "data".into();
    };
    let lower = name.to_lowercase();

    // Config files
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

    // Documentation
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

    // Executable binaries (inside extracted tree)
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

/// Present the resolved asset selection to the user, show alternatives,
/// and return the chosen asset (or None if skipped).
fn confirm_asset_selection(
    sel: &grel_core::AssetSelection,
    config: &Config,
    noconfirm: bool,
    _allow_keyword: bool,
) -> Option<RemoteAsset> {
    if !noconfirm && !grel_cli::is_interactive() {
        return Some(sel.default.clone());
    }

    let managed_tag = if sel.default_is_managed {
        "managed"
    } else {
        "unmanaged"
    };

    if !sel.default_is_managed {
        eprintln!(
            "  {}",
            format!(
                "Warning: Asset \"{}\" requires manual installation.",
                sel.default.filename
            )
            .yellow()
        );
        eprintln!(
            "  {}",
            format!(
                "Info: Will download to {}.",
                config.paths.download_dir.display()
            )
            .yellow()
        );
    }

    println!(
        "  Selected: {} ({}, {})",
        sel.default.filename,
        format_size(sel.default.size_bytes.unwrap_or(0)),
        managed_tag,
    );

    if !sel.alternatives.is_empty() {
        println!("  {}", "Other compatible assets:".dimmed());
        for (i, alt) in sel.alternatives.iter().enumerate() {
            let alt_managed = is_asset_managed(alt, &config.assets);
            let alt_tag = if alt_managed { "managed" } else { "unmanaged" };
            println!(
                "    {}) {} ({}, {})",
                i + 2,
                alt.filename,
                format_size(alt.size_bytes.unwrap_or(0)),
                alt_tag,
            );
        }
    }

    if noconfirm {
        return Some(sel.default.clone());
    }

    let total_options = 1 + sel.alternatives.len();

    loop {
        let prompt = if total_options > 1 {
            format!("Proceed with download? [1-{}, s=skip]: ", total_options)
        } else {
            "Proceed with download? [Y/n]: ".to_string()
        };

        print!("  {prompt}");
        io::Write::flush(&mut io::stdout()).ok();

        let mut input = String::new();
        io::stdin().read_line(&mut input).ok();

        let input = input.trim().to_lowercase();

        if input.is_empty() {
            return Some(sel.default.clone());
        }

        if input == "s" || input == "skip" {
            return None;
        }

        if let Ok(n) = input.parse::<usize>() {
            if n == 1 {
                return Some(sel.default.clone());
            }
            if n >= 2 && n <= 1 + sel.alternatives.len() {
                let chosen = sel.alternatives[n - 2].clone();
                if !is_asset_managed(&chosen, &config.assets) {
                    eprintln!(
                        "  {}",
                        "Warning: This asset requires manual installation.".yellow()
                    );
                }
                return Some(chosen);
            }
        }

        if total_options == 1 {
            if input == "y" || input == "yes" {
                return Some(sel.default.clone());
            }
            if input == "n" || input == "no" {
                return None;
            }
        }
    }
}

/// Sync/install packages
pub async fn cmd_sync(ctx: &CommandContext<'_>, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        eprintln!("No packages specified. Usage: grel -S owner/repo");
        return Ok(());
    }

    // Validate --asdeps / --asexplicit mutual exclusivity
    if ctx.cli.asdeps && ctx.cli.asexplicit {
        anyhow::bail!("--asdeps and --asexplicit are mutually exclusive");
    }

    let is_explicit = if ctx.cli.asdeps {
        false
    } else {
        true // default or --asexplicit
    };

    let host_os = Os::host();
    let host_arch = Arch::host();

    println!("{}", "Resolving packages...".bold());

    let db = ctx.db().await?;
    let client = build_http_client(&ctx.config.general)?;
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let provider_registry = ProviderRegistry::new(client.clone(), github_token);

    let local_registry = grel_core::Registry::new(ctx.config.paths.install_root.join("registry"));
    if ctx.config.registry.auto_update && !ctx.config.registry.url.is_empty() {
        if let Err(e) = local_registry.fetch_index(&ctx.config.registry.url) {
            tracing::warn!("Failed to update registry index: {}", e);
        }
    }

    let selection_policy = match ctx.config.assets.default_selection_policy {
        grel_config::SelectionPolicy::First => grel_core::SelectionPolicy::First,
        grel_config::SelectionPolicy::Largest => grel_core::SelectionPolicy::Largest,
    };

    let mut exclude_keywords = ctx.config.assets.exclude_keywords.clone();
    if let Some(ref extra) = ctx.cli.exclude_keywords {
        exclude_keywords.extend(extra.clone());
    }

    let mut ignore_formats = ctx.config.assets.ignore_formats.clone();
    if let Some(ref allowed) = ctx.cli.allow_format {
        ignore_formats.retain(|f| f != allowed);
    }

    let resolver_config = ResolverConfig {
        default_selection_policy: selection_policy,
        exclude_keywords,
        ignore_formats,
        prefer_formats: ctx.config.assets.prefer_formats.clone(),
        prefer_32bit_on_64bit: ctx.config.assets.prefer_32bit_on_64bit,
        fallback_to_32bit: ctx.config.assets.fallback_to_32bit,
        prefer_musl: ctx.config.assets.prefer_musl,
    };

    let allow_keyword = ctx.cli.allow_keyword();
    let noconfirm = ctx.cli.noconfirm;

    if !ctx.config.security.verify_checksums {
        eprintln!(
            "{}",
            "Warning: Checksum verification is disabled. Set security.verify_checksums = true in your config.".yellow()
        );
    }
    if !ctx.config.security.verify_signatures {
        eprintln!(
            "{}",
            "Warning: Signature verification is disabled. Set security.verify_signatures = true in your config.".yellow()
        );
    }

    for pkg_str in packages {
        println!("\n{}", format!("→ {pkg_str}").bold());

        let pkg_ref = PackageRef::parse_with_forge(pkg_str, ctx.default_forge())
            .with_context(|| format!("Invalid package reference: {pkg_str}"))?;

        let provider = provider_registry
            .get_provider(&pkg_ref.forge)
            .with_context(|| format!("Failed to get provider for {}", pkg_ref.forge))?;

        let release = if let Some(ref tag) = pkg_ref.version {
            println!("  Pinning to version {tag}");
            match provider
                .get_release(&pkg_ref.owner, &pkg_ref.repo, tag)
                .await
            {
                Ok(r) => r,
                Err(grel_providers::ProviderError::NotFound(e)) => {
                    eprintln!("{}", format!("Release not found: {e}").red());
                    continue;
                }
                Err(grel_providers::ProviderError::RateLimitExceeded) => {
                    eprintln!(
                        "{}",
                        "Rate limit exceeded. Set GREL_GITHUB_TOKEN for higher limits.".red()
                    );
                    continue;
                }
                Err(e) => {
                    eprintln!("{}", format!("Failed to fetch release: {e}").red());
                    continue;
                }
            }
        } else {
            match provider.latest_release(&pkg_ref.owner, &pkg_ref.repo).await {
                Ok(r) => r,
                Err(grel_providers::ProviderError::NotFound(e)) => {
                    eprintln!("{}", format!("Package not found: {e}").red());
                    continue;
                }
                Err(grel_providers::ProviderError::RateLimitExceeded) => {
                    eprintln!(
                        "{}",
                        "Rate limit exceeded. Set GREL_GITHUB_TOKEN for higher limits.".red()
                    );
                    continue;
                }
                Err(e) => {
                    eprintln!("{}", format!("Failed to fetch release: {e}").red());
                    continue;
                }
            }
        };

        println!(
            "  Found release {} v{}",
            if release.name.is_empty() {
                &release.tag
            } else {
                &release.name
            },
            release.tag
        );

        if release.prerelease {
            println!("  {}", "Warning: This is a pre-release".yellow());
        }

        let remote_assets: Vec<RemoteAsset> = release.assets.clone();

        let manifest = resolve_manifest(&pkg_ref, &local_registry, provider).await;
        let manifest_source = manifest
            .as_ref()
            .map(|(_, s)| s.clone())
            .unwrap_or(models::ManifestSource::Heuristic);

        let chosen_asset = if let Some(ref asset_name) = ctx.cli.asset {
            let found = remote_assets.iter().find(|a| a.filename == *asset_name);
            match found {
                Some(a) => {
                    println!(
                        "  {}",
                        format!("Using specified asset: {asset_name}").dimmed()
                    );
                    a.clone()
                }
                None => {
                    eprintln!(
                        "  {}",
                        format!("Asset '{asset_name}' not found in release. Available assets:")
                            .red()
                    );
                    for a in &remote_assets {
                        eprintln!("    {}", a.filename);
                    }
                    continue;
                }
            }
        } else {
            let (target_os, target_arch) = if let Some(ref platform_str) = ctx.cli.platform {
                let parts: Vec<&str> = platform_str.split('/').collect();
                if parts.len() == 2 {
                    let os = parts[0]
                        .parse::<Os>()
                        .unwrap_or(Os::Unknown(parts[0].to_string()));
                    let arch = parts[1]
                        .parse::<Arch>()
                        .unwrap_or(Arch::Unknown(parts[1].to_string()));
                    println!("  {}", format!("Platform override: {os}/{arch}").dimmed());
                    (os, arch)
                } else {
                    eprintln!(
                        "  {}",
                        "Invalid --platform format, expected os/arch (e.g. linux/aarch64)".red()
                    );
                    continue;
                }
            } else {
                (host_os.clone(), host_arch.clone())
            };

            let manifest_asset = manifest.as_ref().and_then(|(m, _src)| {
                m.resolve_asset(&target_os, &target_arch, &release.tag)
                    .and_then(|mapping| {
                        let concrete_name = mapping.concrete_filename(&release.tag);
                        remote_assets
                            .iter()
                            .find(|a| a.filename == concrete_name)
                            .cloned()
                    })
            });

            if let Some(asset) = manifest_asset {
                println!(
                    "  {}",
                    format!("Using manifest asset: {}", asset.filename).dimmed()
                );
                asset
            } else {
                if manifest.is_some() {
                    eprintln!(
                        "  {}",
                        "Warning: Manifest asset not found in release, falling back to heuristic."
                            .yellow()
                    );
                }

                let Some(mut sel) = grel_core::resolve_assets_detailed(
                    &remote_assets,
                    &target_os,
                    &target_arch,
                    &resolver_config,
                    allow_keyword,
                ) else {
                    eprintln!(
                        "  {}",
                        format!("No compatible assets for {target_os}/{target_arch}").red()
                    );
                    continue;
                };

                let is_managed_default = is_asset_managed(&sel.default, &ctx.config.assets);
                sel.default_is_managed = is_managed_default;

                match confirm_asset_selection(&sel, ctx.config, noconfirm, allow_keyword) {
                    Some(asset) => asset,
                    None => {
                        eprintln!("  Skipped");
                        continue;
                    }
                }
            }
        };

        // Check if a signature file is available (fail-fast before download)
        if ctx.config.security.verify_signatures {
            if SignatureVerifier::find_signature_asset(
                &release.assets,
                &chosen_asset,
                manifest.as_ref().map(|(m, _)| m),
            )
            .is_none()
            {
                eprintln!(
                    "  {}",
                    format!(
                        "Signature verification enabled but no signature file found for {}. Skipping.",
                        chosen_asset.filename
                    )
                    .red()
                );
                continue;
            }
        }

        let is_managed = if ctx.cli.download_only {
            false
        } else {
            is_asset_managed(&chosen_asset, &ctx.config.assets)
        };

        // --needed: skip if already installed at this version
        if ctx.cli.needed {
            match check_needed_skip(&db, &pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo, &release.tag).await {
                Ok(true) => {
                    println!(
                        "  {}",
                        format!(
                            "{} {} is up-to-date -- skipping",
                            pkg_ref.to_short_ref(),
                            release.tag
                        )
                        .dimmed()
                    );
                    continue;
                }
                Err(e) => tracing::warn!("Failed to check needed skip: {e}"),
                _ => {}
            }
        }

        let install_dir = if is_managed {
            ctx.config.paths.install_root.join(format!(
                "{}/{}/{}",
                pkg_ref.forge, pkg_ref.owner, pkg_ref.repo
            ))
        } else {
            ctx.config.paths.download_dir.clone()
        };
        let archive_path = install_dir.join(&chosen_asset.filename);

        if ctx.cli.dry_run {
            println!(
                "  {}",
                format!("(dry-run: would download {})", chosen_asset.filename).yellow()
            );
            continue;
        }

        std::fs::create_dir_all(&install_dir).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create directory '{}': {e}",
                install_dir.display()
            )
        })?;

        println!(
            "  Downloading: {} ({})",
            chosen_asset.filename,
            format_size(chosen_asset.size_bytes.unwrap_or(0))
        );

        // Download checksum file if verification is enabled
        let expected_checksum = fetch_expected_checksum(
            &client,
            &release,
            &chosen_asset,
            &ctx.config,
            manifest.as_ref().map(|(m, _)| m),
        )
        .await?;

        // Check for cached ETag
        let cached_etag = db.get_etag(&chosen_asset.url).await.ok().flatten();

        let (checksum, response_etag) =
            grel_network::download::download_file(&client, &chosen_asset.url, &archive_path, None, true, cached_etag.as_deref())
                .await
                .with_context(|| format!("Failed to download {}", chosen_asset.filename))?;

        // Verify checksum
        if let Some(expected) = expected_checksum {
            verify_checksum_or_clean(&checksum, &expected, &archive_path, &chosen_asset.filename)?;
        }

        // Verify cryptographic signature
        if ctx.config.security.verify_signatures {
            if let Err(e) = verify_signature_for_asset(
                &client,
                &release,
                &chosen_asset,
                &archive_path,
                &ctx.config.security,
                manifest.as_ref().map(|(m, _)| m),
            )
            .await
            {
                std::fs::remove_file(&archive_path).ok();
                return Err(anyhow::anyhow!(
                    "Signature verification failed for {}: {}",
                    chosen_asset.filename,
                    e
                ));
            }
            println!("  {}", "Signature verified".dimmed());
        }

        // Store ETag for future conditional requests
        if let Some(etag) = response_etag {
            db.store_etag(&chosen_asset.url, &etag).await.ok();
        }

        let installed_binaries = if is_managed {
            match grel_network::archive::install_asset(
                &archive_path,
                &install_dir,
                &ctx.config.paths.bin_dir,
                &chosen_asset.filename,
                ctx.cli.overwrite,
            ) {
                Ok(result) => {
                    if !result.installed_binaries.is_empty() {
                        println!(
                            "  {}",
                            format!(
                                "Installed {} binary(s) to {}",
                                result.installed_binaries.len(),
                                ctx.config.paths.bin_dir.display()
                            )
                            .green()
                        );
                    } else if result.is_plain_binary {
                        println!("  {}", "Installed binary".green());
                    }
                    result.installed_binaries
                }
                Err(e) => {
                    eprintln!("  {}", format!("Warning: Failed to extract: {e}").yellow());
                    vec![]
                }
            }
        } else {
            vec![]
        };

        let bin_filenames: Vec<String> = installed_binaries
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .collect();

        // Save manifest and run post_install hook
        if let Some((ref manifest, _)) = manifest {
            if let Err(e) = save_manifest_to_dir(manifest, &install_dir) {
                tracing::warn!("Failed to save manifest: {e}");
            }
            if !ctx.cli.download_only && ctx.config.security.enable_hooks {
                if let Some(ref hook) = manifest.hooks.post_install {
                    run_hook(hook, &install_dir, "post_install");
                }
            }
        }

        if !ctx.cli.download_only {
            check_elf_deps(&installed_binaries, ctx, &db).await;
        }

        if is_managed && !ctx.config.general.keep_archives && archive_path.exists() {
            std::fs::remove_file(&archive_path).ok();
            println!("  {}", "Cleaned up downloaded archive".dimmed());
        }

        let mut pkg = models::InstalledPackage::new(
            pkg_ref.forge.to_string(),
            pkg_ref.owner.clone(),
            pkg_ref.repo.clone(),
        );
        pkg.version = release.tag.clone();
        pkg.asset_filename = chosen_asset.filename.clone();
        pkg.checksum = Some(checksum);
        pkg.install_path = if is_managed {
            install_dir.to_string_lossy().to_string()
        } else {
            archive_path.to_string_lossy().to_string()
        };
        pkg.set_binary_list(bin_filenames);
        pkg.is_managed = is_managed;
        pkg.status = models::PackageStatus::Active;
        pkg.manifest_source = manifest_source;
        pkg.is_explicit = is_explicit;

        db.upsert_package(&pkg)
            .await
            .with_context(|| "Failed to update package record")?;

        let target_pkg_id = db
            .get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo)
            .await?
            .and_then(|p| p.id)
            .unwrap_or(-1);

        if let Some((ref manifest, _)) = manifest {
            let grel_deps = manifest.parse_grel_deps();
            let mut dep_records: Vec<models::Dependency> = Vec::new();

            for dep_ref in &grel_deps {
                let already_installed = db
                    .get_package(&dep_ref.forge.to_string(), &dep_ref.owner, &dep_ref.repo)
                    .await?
                    .is_some();

                if !already_installed && !ctx.cli.dry_run {
                    println!("  Installing dependency: {}", dep_ref.to_short_ref());
                    match install_single_package(
                        ctx.config,
                        dep_ref,
                        &provider_registry,
                        &local_registry,
                        &db,
                        &client,
                        &resolver_config,
                        &host_os,
                        &host_arch,
                        noconfirm,
                        allow_keyword,
                        ctx.cli.dry_run,
                        false,
                        ctx.cli.overwrite,
                        ctx.cli.download_only,
                        ctx.cli.needed,
                    )
                    .await
                    {
                        Ok(dep_id) if dep_id > 0 => {
                            dep_records.push(models::Dependency::grel(target_pkg_id, dep_ref));
                        }
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("    {}", format!("Failed: {e}").red());
                        }
                    }
                } else if already_installed && target_pkg_id > 0 {
                    dep_records.push(models::Dependency::grel(target_pkg_id, dep_ref));
                }
            }

            // Persist system dependencies
            for lib in &manifest.dependencies.system {
                dep_records.push(models::Dependency::system(target_pkg_id, lib));
            }

            if target_pkg_id > 0 && !dep_records.is_empty() {
                if let Err(e) = db.set_dependencies(target_pkg_id, &dep_records).await {
                    tracing::warn!("Failed to record dependencies: {}", e);
                }
            }

            // Check availability of required system libraries
            if !manifest.dependencies.system.is_empty() {
                let report =
                    grel_network::system_deps::check_system_deps(&manifest.dependencies.system);
                if !report.all_found() {
                    eprintln!("  {}", "Warning: Missing system libraries:".yellow());
                    for lib in &report.missing {
                        let advice = grel_network::system_deps::format_missing_lib_advice(lib);
                        eprintln!("    - {}: {}", lib, advice);
                    }
                }
            }

            let opt_deps = manifest.parse_grel_opt_deps();
            if !opt_deps.is_empty() {
                println!("  {}", "Optional dependencies:".dimmed());
                for (opt, reason) in &opt_deps {
                    println!("    {} ({})", opt.to_short_ref(), reason);
                }
            }
        }

        if ctx.cli.download_only {
            println!(
                "  {}",
                format!("Downloaded {} v{}", pkg_ref.to_short_ref(), release.tag).green()
            );
        } else {
            println!(
                "  {}",
                format!("Installed {} v{}", pkg_ref.to_short_ref(), release.tag).green()
            );
        }
    }

    db.close().await;
    Ok(())
}

/// Search for packages on the forge
pub async fn cmd_search(
    ctx: &CommandContext<'_>,
    pattern: String,
    max_results: usize,
) -> Result<()> {
    let local_registry = grel_core::Registry::new(ctx.config.paths.install_root.join("registry"));
    if local_registry.is_available() {
        match local_registry.search(&pattern) {
            Ok(reg_results) if !reg_results.is_empty() => {
                println!(
                    "{}",
                    format!("Searching registry for \"{pattern}\"...").bold()
                );
                println!("\n{} results found:\n", reg_results.len());

                for (key, manifest) in reg_results {
                    println!("{}  {}", key.bold().cyan(), manifest.name.dimmed());
                    if !manifest.description.is_empty() {
                        let desc = if manifest.description.len() > 100 {
                            format!("{}...", &manifest.description[..100])
                        } else {
                            manifest.description.clone()
                        };
                        println!("    {desc}");
                    }
                    println!();
                }
                return Ok(());
            }
            _ => {}
        }
    }

    println!(
        "{}",
        format!("Searching for \"{pattern}\" on {}...", ctx.default_forge()).bold()
    );

    let client = build_http_client(&ctx.config.general)?;
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let provider_registry = ProviderRegistry::new(client, github_token);

    let provider = provider_registry
        .get_provider(&ctx.default_forge())
        .with_context(|| format!("Provider for {} not available", ctx.default_forge()))?;

    let results = provider.search_repos(&pattern, max_results).await?;

    if results.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    println!("\n{} results found:\n", results.len());

    for result in &results {
        let stars = if result.stargazers_count >= 1000 {
            format!("{:.1}k", result.stargazers_count as f64 / 1000.0)
        } else {
            result.stargazers_count.to_string()
        };

        println!(
            "{}  {}",
            format!("{}/{}", result.owner, result.repo).bold().cyan(),
            format!("({stars} stars)").dimmed()
        );

        if !result.description.is_empty() {
            let desc = if result.description.len() > 100 {
                format!("{}...", &result.description[..100])
            } else {
                result.description.clone()
            };
            println!("    {desc}");
        }
        println!();
    }

    Ok(())
}

/// Sync package database (refresh only)
pub async fn cmd_sync_refresh(ctx: &CommandContext<'_>) -> Result<()> {
    println!("{}", "Synchronizing package database...".bold());

    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let active_packages: Vec<_> = packages
        .into_iter()
        .filter(|p| matches!(p.status, models::PackageStatus::Active))
        .collect();

    if active_packages.is_empty() {
        println!("No packages to synchronize.");
        db.close().await;
        return Ok(());
    }

    let client = build_http_client(&ctx.config.general)?;
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let registry = ProviderRegistry::new(client, github_token);

    let mut up_to_date = 0;
    let mut available_updates = 0;
    let mut orphaned = 0;
    let mut skipped = 0;

    let now = chrono::Utc::now().timestamp();

    for pkg in &active_packages {
        let forge: Forge = pkg.forge.parse().unwrap_or(ctx.default_forge());

        print!("  {:<35} ", pkg.package_ref());

        if let Some(last) = pkg.last_checked {
            if now - last < (ctx.config.upgrade.check_interval_hours as i64) * 3600 {
                println!("{}", "skipped (recently checked)".dimmed());
                skipped += 1;
                continue;
            }
        }

        let provider = match registry.get_provider(&forge) {
            Ok(p) => p,
            Err(_) => {
                println!("{}", "provider unavailable".red());
                continue;
            }
        };

        match provider.latest_release(&pkg.owner, &pkg.repo).await {
            Ok(release) => {
                if let Some(id) = pkg.id {
                    db.update_last_checked(id).await.ok();
                }
                if release.tag == pkg.version {
                    println!("{}", "up to date".green());
                    up_to_date += 1;
                } else {
                    println!("{}", format!("{} -> {}", pkg.version, release.tag).yellow());
                    available_updates += 1;
                }
            }
            Err(grel_providers::ProviderError::NotFound(_)) => {
                if let Some(id) = pkg.id {
                    db.mark_orphaned(id).await.ok();
                }
                println!("{}", "orphaned".red());
                orphaned += 1;
            }
            Err(_) => {
                println!("{}", "error".red());
            }
        }
    }

    println!();
    println!("{} packages checked", active_packages.len());
    println!("  Up to date:       {up_to_date}");
    if available_updates > 0 {
        println!("  Updates available: {available_updates}");
        println!("  Run 'grel -Syu' to upgrade all");
    }
    if orphaned > 0 {
        println!("  Orphaned:         {orphaned}");
    }
    if skipped > 0 {
        println!("  Skipped:          {skipped} (within check interval)");
    }

    db.close().await;
    Ok(())
}

/// Clean artifact cache
pub async fn cmd_clean_cache(ctx: &CommandContext<'_>) -> Result<()> {
    println!("{}", "Cleaning artifact cache...".bold());

    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let active_packages: Vec<_> = packages
        .into_iter()
        .filter(|p| matches!(p.status, models::PackageStatus::Active))
        .collect();

    // Build set of valid archive paths for active managed packages
    // Build set of valid archive paths for active managed packages
    let _valid_archives: std::collections::HashSet<std::path::PathBuf> = active_packages
        .iter()
        .filter(|p| p.is_managed)
        .map(|p| {
            std::path::PathBuf::from(&p.install_path).join(&p.asset_filename)
        })
        .collect();

    let mut removed = 0;
    let mut failed = 0;

    // Clean download_dir
    let download_dir = &ctx.config.paths.download_dir;
    let active_download_paths: std::collections::HashSet<String> = active_packages
        .iter()
        .filter(|p| !p.is_managed)
        .map(|p| p.install_path.clone())
        .collect();

    if download_dir.exists() {
        for entry in std::fs::read_dir(download_dir)? {
            let entry = entry?;
            let path = entry.path();
            let path_str = path.to_string_lossy().to_string();

            if !active_download_paths.contains(&path_str) {
                if path.is_file() {
                    match std::fs::remove_file(&path) {
                        Ok(_) => removed += 1,
                        Err(_) => failed += 1,
                    }
                }
            }
        }
    }

    // Clean old archives under install_root for managed packages
    for pkg in &active_packages {
        if !pkg.is_managed {
            continue;
        }
        let pkg_dir = std::path::PathBuf::from(&pkg.install_path);
        if !pkg_dir.exists() || !pkg_dir.is_dir() {
            continue;
        }
        let expected_archive = pkg_dir.join(&pkg.asset_filename);
        let (r, f) = clean_package_dir(&pkg_dir, &expected_archive);
        removed += r;
        failed += f;
    }

    println!("  Removed {removed} orphaned archive(s)");
    if failed > 0 {
        println!("  Failed to remove {failed} file(s)");
    }

    db.close().await;
    Ok(())
}

/// Upgrade all packages (-Su / -Syu)
pub async fn cmd_upgrade(ctx: &CommandContext<'_>) -> Result<()> {
    println!("{}", "Checking for upgrades...".bold());

    let db = ctx.db().await?;
    let packages = db.list_packages().await?;
    let active_packages: Vec<_> = packages
        .into_iter()
        .filter(|p| matches!(p.status, models::PackageStatus::Active))
        .collect();

    if active_packages.is_empty() {
        println!("No packages to upgrade.");
        db.close().await;
        return Ok(());
    }

    let client = build_http_client(&ctx.config.general)?;
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let registry = ProviderRegistry::new(client.clone(), github_token);

    let host_os = Os::host();
    let host_arch = Arch::host();

    let selection_policy = match ctx.config.assets.default_selection_policy {
        grel_config::SelectionPolicy::First => grel_core::SelectionPolicy::First,
        grel_config::SelectionPolicy::Largest => grel_core::SelectionPolicy::Largest,
    };
    let resolver_config = ResolverConfig {
        default_selection_policy: selection_policy,
        exclude_keywords: ctx.config.assets.exclude_keywords.clone(),
        ignore_formats: ctx.config.assets.ignore_formats.clone(),
        prefer_formats: ctx.config.assets.prefer_formats.clone(),
        prefer_32bit_on_64bit: ctx.config.assets.prefer_32bit_on_64bit,
        fallback_to_32bit: ctx.config.assets.fallback_to_32bit,
        prefer_musl: ctx.config.assets.prefer_musl,
    };

    let mut upgrades = Vec::new();
    let mut up_to_date = Vec::new();
    let mut orphaned = Vec::new();
    let mut errors = Vec::new();

    let now = chrono::Utc::now().timestamp();

    for pkg in &active_packages {
        let forge: Forge = pkg.forge.parse().unwrap_or(ctx.default_forge());
        let pkg_ref_str = format!("{}/{}", pkg.owner, pkg.repo);

        let provider = match registry.get_provider(&forge) {
            Ok(p) => p,
            Err(e) => {
                errors.push((pkg_ref_str.clone(), e.to_string()));
                continue;
            }
        };

        if let Some(last) = pkg.last_checked {
            if now - last < (ctx.config.upgrade.check_interval_hours as i64) * 3600 {
                up_to_date.push(pkg_ref_str);
                continue;
            }
        }

        let release = match provider.latest_release(&pkg.owner, &pkg.repo).await {
            Ok(r) => r,
            Err(grel_providers::ProviderError::NotFound(_)) => {
                if let Some(id) = pkg.id {
                    db.mark_orphaned(id).await.ok();
                }
                orphaned.push(pkg_ref_str.clone());
                continue;
            }
            Err(e) => {
                errors.push((pkg_ref_str.clone(), e.to_string()));
                continue;
            }
        };

        if release.tag == pkg.version
            && release
                .assets
                .iter()
                .any(|a| a.filename == pkg.asset_filename)
        {
            up_to_date.push(pkg_ref_str.clone());
            if let Some(id) = pkg.id {
                db.update_last_checked(id).await.ok();
            }
            continue;
        }

        let remote_assets: Vec<RemoteAsset> = release.assets.clone();
        let selection = grel_core::resolve_assets(
            &remote_assets,
            &host_os,
            &host_arch,
            &resolver_config,
            false,
        );

        let chosen_asset = match selection {
            grel_core::SelectionResult::SingleAsset(a) => a,
            grel_core::SelectionResult::NoCompatibleAssets => {
                errors.push((
                    pkg_ref_str.clone(),
                    "No compatible assets in new release".into(),
                ));
                continue;
            }
            grel_core::SelectionResult::MultipleAssets(assets) => {
                match assets.into_iter().next() {
                    Some(a) => a,
                    None => {
                        errors.push((
                            pkg_ref_str.clone(),
                            "Asset selection returned empty list".into(),
                        ));
                        continue;
                    }
                }
            }
        };

        let is_managed = is_asset_managed(&chosen_asset, &ctx.config.assets);
        let renamed = chosen_asset.filename != pkg.asset_filename;

        if !is_managed {
            eprintln!(
                "  {}",
                format!(
                    "⚠️  {pkg_ref_str}: new asset \"{}\" is unmanaged (requires manual installation)",
                    chosen_asset.filename
                )
                .yellow()
            );
        }

        upgrades.push((
            pkg.clone(),
            release,
            chosen_asset,
            renamed,
            forge,
            is_managed,
        ));
    }

    if upgrades.is_empty() && up_to_date.is_empty() && orphaned.is_empty() && errors.is_empty() {
        println!("Nothing to do.");
        db.close().await;
        return Ok(());
    }

    if !upgrades.is_empty() {
        println!(
            "{}",
            format!("{} package(s) to upgrade:", upgrades.len()).bold()
        );
        for (pkg, release, asset, renamed, _, is_managed) in &upgrades {
            let rename_note = if *renamed {
                format!(
                    " (asset renamed: {} → {})",
                    pkg.asset_filename, asset.filename
                )
            } else {
                String::new()
            };
            let managed_note = if !is_managed {
                " ⚠️ unmanaged".to_string()
            } else {
                String::new()
            };
            println!(
                "  {} {} → {}{}{}",
                format!("{}/{}", pkg.owner, pkg.repo).bold(),
                pkg.version,
                release.tag,
                rename_note,
                managed_note,
            );
        }
        println!();
    }

    if !orphaned.is_empty() {
        println!(
            "{}",
            format!("{} package(s) orphaned (repo not found):", orphaned.len()).yellow()
        );
        for name in &orphaned {
            println!("  {name}");
        }
        println!();
    }

    let noconfirm = ctx.cli.noconfirm;
    if !noconfirm && !grel_cli::is_interactive() {
        eprintln!("Info: Non-interactive mode, proceeding automatically");
    } else if !noconfirm && !upgrades.is_empty() {
        let accepted = grel_cli::ask_confirmation("Proceed with upgrade?", true);
        if !accepted {
            println!("Cancelled.");
            db.close().await;
            return Ok(());
        }
    }

    let mut upgraded = 0;
    let mut failed = 0;

    for (pkg, release, asset, renamed, forge, is_managed) in &upgrades {
        let pkg_ref_str = format!("{}/{}", pkg.owner, pkg.repo);
        print!(
            "  {} {pkg_ref_str} {} → {} ... ",
            "→".cyan(),
            pkg.version,
            release.tag
        );

        match upgrade_single_package(
            &db,
            &client,
            pkg,
            release.clone(),
            asset.clone(),
            *renamed,
            ctx.config,
            *forge,
            *is_managed,
            ctx.cli.overwrite,
        )
        .await
        {
            Ok(installed_bins) => {
                println!("{}", "ok".green());
                upgraded += 1;
                check_elf_deps(&installed_bins, ctx, &db).await;
            }
            Err(e) => {
                println!("{}", format!("FAILED: {e}").red());
                failed += 1;
            }
        }
    }

    println!();
    println!("{}", "Upgrade summary:".bold());
    println!("  Upgraded:  {upgraded}");
    println!("  Up-to-date: {}", up_to_date.len());
    println!("  Orphaned:  {}", orphaned.len());
    println!("  Failed:    {failed}");
    if !errors.is_empty() {
        println!("  Errors:");
        for (pkg, err) in &errors {
            println!("    {pkg}: {err}");
        }
    }

    db.close().await;
    Ok(())
}

/// Upgrade a single package: download → extract → atomic replace → DB update.
/// Returns the installed binary paths for subsequent ELF dep checking.
#[allow(clippy::too_many_arguments)]
async fn upgrade_single_package(
    db: &Database,
    client: &Client,
    pkg: &models::InstalledPackage,
    release: grel_providers::Release,
    asset: RemoteAsset,
    renamed: bool,
    config: &Config,
    _forge: Forge,
    is_managed: bool,
    overwrite: bool,
) -> Result<Vec<std::path::PathBuf>> {
    let install_dir = if is_managed {
        config
            .paths
            .install_root
            .join(format!("{}/{}/{}", pkg.forge, pkg.owner, pkg.repo))
    } else {
        config.paths.download_dir.clone()
    };

    if config.security.verify_signatures {
        if SignatureVerifier::find_signature_asset(
            &release.assets,
            &asset,
            None, // Upgrade path does not currently resolve manifests
        )
        .is_none()
        {
            return Err(anyhow::anyhow!(
                "Signature verification enabled but no signature file found for {}",
                asset.filename
            ));
        }
    } else {
        eprintln!(
            "{}",
            "Warning: Signature verification is disabled. Set security.verify_signatures = true in your config.".yellow()
        );
    }
    if !config.security.verify_checksums {
        eprintln!(
            "{}",
            "Warning: Checksum verification is disabled. Set security.verify_checksums = true in your config.".yellow()
        );
    }

    let archive_path = install_dir.join(&asset.filename);

    std::fs::create_dir_all(&install_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create directory: {e}"))?;

    // Download checksum file if verification is enabled
    let expected_checksum = fetch_expected_checksum(
        client,
        &release,
        &asset,
        config,
        None, // Upgrade path does not currently resolve manifests
    )
    .await?;

    let temp_path = archive_path.with_extension("part");
    // Check for cached ETag
    let cached_etag = db.get_etag(&asset.url).await.ok().flatten();

    let (checksum, response_etag) = grel_network::download::download_file(client, &asset.url, &temp_path, None, true, cached_etag.as_deref())
        .await
        .with_context(|| format!("Failed to download {}", asset.filename))?;

    // Verify checksum
    if let Some(expected) = expected_checksum {
        verify_checksum_or_clean(&checksum, &expected, &temp_path, &asset.filename)?;
    }

    // Verify cryptographic signature
    if config.security.verify_signatures {
        if let Err(e) = verify_signature_for_asset(
            client,
            &release,
            &asset,
            &temp_path,
            &config.security,
            None, // Upgrade path does not currently resolve manifests
        )
        .await
        {
            std::fs::remove_file(&temp_path).ok();
            return Err(anyhow::anyhow!(
                "Signature verification failed for {}: {}",
                asset.filename,
                e
            ));
        }
        println!("  {}", "Signature verified".dimmed());
    }

    // Store ETag for future conditional requests
    if let Some(etag) = response_etag {
        db.store_etag(&asset.url, &etag).await.ok();
    }

    std::fs::rename(&temp_path, &archive_path)
        .map_err(|e| anyhow::anyhow!("Failed to rename downloaded file: {e}"))?;

    let mut updated_pkg = pkg.clone();
    updated_pkg.version = release.tag.clone();
    updated_pkg.asset_filename = asset.filename.clone();
    updated_pkg.checksum = Some(checksum.clone());
    updated_pkg.is_managed = is_managed;
    updated_pkg.install_path = if is_managed {
        install_dir.to_string_lossy().to_string()
    } else {
        archive_path.to_string_lossy().to_string()
    };
    updated_pkg.last_checked = Some(chrono::Utc::now().timestamp());
    updated_pkg.status = models::PackageStatus::Active;

    let mut installed_bins: Vec<std::path::PathBuf> = vec![];

    if is_managed {
        let extracted_dir = install_dir.join("extracted");
        if extracted_dir.exists() {
            std::fs::remove_dir_all(&extracted_dir)
                .map_err(|e| anyhow::anyhow!("Failed to clean old extraction directory: {e}"))?;
        }

        match grel_network::archive::install_asset(
            &archive_path,
            &install_dir,
            &config.paths.bin_dir,
            &asset.filename,
            overwrite,
        ) {
            Ok(result) => {
                let bin_filenames: Vec<String> = result
                    .installed_binaries
                    .iter()
                    .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .collect();

                installed_bins = result.installed_binaries;
                updated_pkg.set_binary_list(bin_filenames.clone());

                let old_bins: std::collections::HashSet<String> =
                    pkg.binary_list().into_iter().collect();
                let new_bins: std::collections::HashSet<String> =
                    bin_filenames.into_iter().collect();
                for stale_bin in old_bins.difference(&new_bins) {
                    let stale_path = config.paths.bin_dir.join(stale_bin);
                    if stale_path.exists() {
                        std::fs::remove_file(&stale_path).ok();
                    }
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to extract: {e}"));
            }
        }
    } else {
        updated_pkg.set_binary_list(vec![]);

        if pkg.is_managed {
            let old_install_dir = config
                .paths
                .install_root
                .join(format!("{}/{}/{}", pkg.forge, pkg.owner, pkg.repo));
            if old_install_dir.exists() {
                std::fs::remove_dir_all(&old_install_dir).ok();
            }
            for old_bin in pkg.binary_list() {
                let old_path = config.paths.bin_dir.join(&old_bin);
                if old_path.exists() {
                    std::fs::remove_file(&old_path).ok();
                }
            }
        }
    }

    if renamed {
        let old_archive = std::path::Path::new(&pkg.install_path).join(&pkg.asset_filename);
        if old_archive.exists() && old_archive != archive_path {
            std::fs::remove_file(&old_archive).ok();
        }
    }

    if !config.general.keep_archives && archive_path.exists() {
        std::fs::remove_file(&archive_path).ok();
    }

    db.upsert_package(&updated_pkg)
        .await
        .with_context(|| "Failed to update package record")?;

    Ok(installed_bins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grel_cache::{Database, models::InstalledPackage};

    fn make_temp_dir(label: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!("grel-sync-test-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    fn cleanup(tmp: &std::path::Path) {
        let _ = std::fs::remove_dir_all(tmp);
    }

    // -----------------------------------------------------------------------
    // check_needed_skip
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn check_needed_skip_returns_true_when_version_matches() {
        let tmp = make_temp_dir("needed-skip-true");
        let db = Database::init(&tmp.join("test.sqlite")).await.expect("init db");

        let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
        pkg.version = "1.2.3".into();
        pkg.is_managed = true;
        db.upsert_package(&pkg).await.expect("upsert");

        let should_skip = check_needed_skip(&db, "github", "owner", "repo", "1.2.3")
            .await
            .expect("check_needed_skip");

        assert!(should_skip, "expected skip when version matches");

        db.close().await;
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn check_needed_skip_returns_false_when_version_differs() {
        let tmp = make_temp_dir("needed-skip-false");
        let db = Database::init(&tmp.join("test.sqlite")).await.expect("init db");

        let mut pkg = InstalledPackage::new("github".into(), "owner".into(), "repo".into());
        pkg.version = "1.2.3".into();
        pkg.is_managed = true;
        db.upsert_package(&pkg).await.expect("upsert");

        let should_skip = check_needed_skip(&db, "github", "owner", "repo", "2.0.0")
            .await
            .expect("check_needed_skip");

        assert!(!should_skip, "expected no skip when version differs");

        db.close().await;
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn check_needed_skip_returns_false_when_package_missing() {
        let tmp = make_temp_dir("needed-skip-missing");
        let db = Database::init(&tmp.join("test.sqlite")).await.expect("init db");

        let should_skip = check_needed_skip(&db, "github", "owner", "repo", "1.0.0")
            .await
            .expect("check_needed_skip");

        assert!(!should_skip, "expected no skip when package not installed");

        db.close().await;
        cleanup(&tmp);
    }

    // -----------------------------------------------------------------------
    // clean_package_dir
    // -----------------------------------------------------------------------

    #[test]
    fn clean_package_dir_removes_stale_archives() {
        let tmp = make_temp_dir("clean-dir");
        let pkg_dir = tmp.join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let current = pkg_dir.join("tool-v2.tar.gz");
        let stale1 = pkg_dir.join("tool-v1.tar.gz");
        let stale2 = pkg_dir.join("tool-v1.zip");
        let other = pkg_dir.join("README.md");

        std::fs::File::create(&current).unwrap();
        std::fs::File::create(&stale1).unwrap();
        std::fs::File::create(&stale2).unwrap();
        std::fs::File::create(&other).unwrap();

        let (removed, failed) = clean_package_dir(&pkg_dir, &current);

        assert_eq!(removed, 2, "expected 2 stale archives removed");
        assert_eq!(failed, 0, "expected 0 failures");
        assert!(current.exists(), "current archive should be kept");
        assert!(!stale1.exists(), "stale archive 1 should be removed");
        assert!(!stale2.exists(), "stale archive 2 should be removed");
        assert!(other.exists(), "non-archive file should be kept");

        cleanup(&tmp);
    }

    #[test]
    fn clean_package_dir_keeps_only_expected_archive() {
        let tmp = make_temp_dir("clean-keep");
        let pkg_dir = tmp.join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let expected = pkg_dir.join("expected.tar.xz");
        std::fs::File::create(&expected).unwrap();

        let (removed, failed) = clean_package_dir(&pkg_dir, &expected);

        assert_eq!(removed, 0);
        assert_eq!(failed, 0);
        assert!(expected.exists());

        cleanup(&tmp);
    }

    // -----------------------------------------------------------------------
    // save_manifest_to_dir
    // -----------------------------------------------------------------------

    #[test]
    fn save_manifest_to_dir_writes_toml() {
        let tmp = make_temp_dir("save-manifest");
        let manifest = grel_core::Manifest {
            name: "test-pkg".into(),
            description: "A test package".into(),
            license: "MIT".into(),
            source: Default::default(),
            assets: Default::default(),
            checksum_filename: Default::default(),
            signature_filename: Default::default(),
            signature_kind: Default::default(),
            dependencies: Default::default(),
            hooks: Default::default(),
        };

        save_manifest_to_dir(&manifest, &tmp).expect("save_manifest_to_dir");

        let manifest_path = tmp.join(".grel.toml");
        assert!(manifest_path.exists(), "manifest file should be created");

        let content = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(content.contains("test-pkg"), "manifest should contain package name");
        assert!(content.contains("MIT"), "manifest should contain license");

        cleanup(&tmp);
    }

    // -----------------------------------------------------------------------
    // run_hook
    // -----------------------------------------------------------------------

    #[test]
    #[cfg(unix)]
    fn run_hook_executes_successfully() {
        let tmp = make_temp_dir("run-hook");
        let marker = tmp.join("hook_ran");

        run_hook(
            &format!("touch {}", marker.display()),
            &tmp,
            "post_install",
        );

        assert!(marker.exists(), "hook should have created marker file");

        cleanup(&tmp);
    }

    #[test]
    #[cfg(unix)]
    fn run_hook_handles_failure_gracefully() {
        let tmp = make_temp_dir("run-hook-fail");

        // This should not panic
        run_hook("exit 1", &tmp, "pre_remove");

        cleanup(&tmp);
    }

    #[test]
    #[cfg(windows)]
    fn run_hook_graceful_on_windows() {
        let tmp = make_temp_dir("run-hook-win");

        // run_hook uses 'sh' which is not available on Windows;
        // it should print a warning but not panic.
        run_hook("echo hello", &tmp, "post_install");

        cleanup(&tmp);
    }
}
