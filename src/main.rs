//! grel - A package manager for pre-built binaries from Git forges

use std::io;

use anyhow::{Context, Result};
use clap::Parser;
use grel_cache::{Database, models};
use grel_cli::{Cli, Operation};
use grel_config::{load_config, Config};
use grel_core::{PackageRef, Os, Arch, ResolverConfig, RemoteAsset};
use grel_core::Forge;
use grel_network::build_http_client;
use grel_providers::ProviderRegistry;
use owo_colors::OwoColorize;
use tracing_indicatif::IndicatifLayer;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing with indicatif layer
    let indicatif_layer: IndicatifLayer<tracing_subscriber::Registry> = IndicatifLayer::new();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(indicatif_layer.get_stderr_writer())
        .with_ansi(true)
        .try_init();

    let cli = Cli::parse();

    // Load configuration
    let config_path = cli.config.clone().map(|p| std::path::PathBuf::from(p));
    let config = load_config(config_path.as_deref())?;

    let default_forge = cli.default_forge.0;
    let noconfirm = cli.noconfirm;

    // Route by operation (first wins)
    match cli.operation() {
        Operation::Sync => {
            if cli.sysupgrade {
                cmd_upgrade(&config, noconfirm, default_forge, cli.refresh).await?;
            } else if let Some(ref pattern) = cli.sync_search {
                cmd_search(&config, pattern.clone(), default_forge, 20).await?;
            } else if let Some(ref pkg) = cli.info {
                cmd_info_remote(&config, pkg.clone(), default_forge).await?;
            } else if cli.refresh {
                cmd_sync_refresh(&config, default_forge).await?;
            } else if cli.clean {
                cmd_clean_cache(&config).await?;
            } else {
                cmd_sync(&config, &cli.targets, default_forge, noconfirm, &cli).await?;
            }
        }
        Operation::Query => {
            if cli.orphans {
                cmd_list_orphans(&config, cli.quiet).await?;
            } else if let Some(ref pkg) = cli.info {
                cmd_info_local(&config, pkg.clone()).await?;
            } else if let Some(ref path) = cli.owns {
                cmd_owns(&config, path.clone()).await?;
            } else if cli.check {
                cmd_verify_checksums(&config).await?;
            } else if cli.list.is_some() || cli.targets.is_empty() {
                // Default: list all installed
                cmd_list(&config, cli.quiet, cli.explicit).await?;
            } else {
                cmd_list(&config, cli.quiet, cli.explicit).await?;
            }
        }
        Operation::Remove => {
            cmd_remove(&config, &cli.targets, noconfirm, &cli).await?;
        }
        Operation::Database => {
            if cli.db_clean {
                cmd_db_clean(&config).await?;
            } else if cli.db_check {
                cmd_db_check(&config).await?;
            } else if cli.db_dump {
                cmd_db_dump(&config).await?;
            } else if let Some(ref pkg) = cli.asexplicit {
                cmd_db_as_explicit(&config, pkg).await?;
            } else if let Some(ref pkg) = cli.asdeps {
                cmd_db_as_deps(&config, pkg).await?;
            } else if let Some(ref args) = cli.migrate {
                if args.len() == 2 {
                    cmd_migrate(&config, &args[0], &args[1]).await?;
                } else {
                    eprintln!("Usage: grel -D --migrate <OLD> <NEW>");
                }
            } else {
                eprintln!("No database operation specified. Use -Dh for help.");
            }
        }
        Operation::Upgrade => {
            cmd_upgrade_local(&config, &cli.targets, noconfirm, &cli).await?;
        }
        Operation::Files => {
            if let Some(ref pattern) = cli.file_search {
                cmd_file_search(&config, pattern.clone(), cli.quiet).await?;
            } else if let Some(ref pkg) = cli.file_list {
                cmd_file_list(&config, pkg.clone(), cli.quiet).await?;
            } else if cli.refresh {
                cmd_reindex(&config).await?;
            } else {
                eprintln!("No files operation specified. Use -Fh for help.");
            }
        }
        Operation::Help => {
            // clap --help was already handled; this means bare `grel` was run
            println!("{}", "grel - A package manager for pre-built binaries from Git forges".bold());
            println!();
            println!("Usage: grel <OPERATION> [OPTIONS] [TARGETS...]");
            println!();
            println!("Operations (first one wins):");
            println!("  -S, --sync       Fetch & install from forges");
            println!("  -Q, --query      Inspect local state");
            println!("  -R, --remove     Uninstall packages");
            println!("  -D, --database   Local DB & state management");
            println!("  -U, --upgrade    Install from local file");
            println!("  -F, --files      File search & index");
            println!();
            println!("Common workflows:");
            println!("  grel -S foo/bar        Install a package");
            println!("  grel -Ss ripgrep       Search for packages");
            println!("  grel -Sy               Refresh metadata");
            println!("  grel -Su               Upgrade installed (cached)");
            println!("  grel -Syu              Refresh + upgrade");
            println!("  grel -Ql               List installed");
            println!("  grel -Qi foo/bar       Show local package info");
            println!("  grel -R foo/bar        Remove a package");
            println!();
            println!("Use `grel -Sh`, `grel -Qh`, etc. for operation-specific help.");
            println!("Use `grel --help` for full flag listing.");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Sync / Install
// ---------------------------------------------------------------------------

/// Sync/install packages
async fn cmd_sync(
    config: &Config,
    packages: &[String],
    default_forge: Forge,
    noconfirm: bool,
    cli: &Cli,
) -> Result<()> {
    if packages.is_empty() {
        eprintln!("No packages specified. Usage: grel -S owner/repo");
        return Ok(());
    }

    let host_os = Os::host();
    let host_arch = Arch::host();

    println!("{}", "Resolving packages...".bold());

    // Initialize database
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    // Initialize HTTP client
    let client = build_http_client(&config.general)?;

    // Initialize provider registry
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let registry = ProviderRegistry::new(client.clone(), github_token);

    // Build resolver config
    let selection_policy = match config.assets.default_selection_policy {
        grel_config::SelectionPolicy::First => grel_core::SelectionPolicy::First,
        grel_config::SelectionPolicy::Largest => grel_core::SelectionPolicy::Largest,
    };

    let mut exclude_keywords = config.assets.exclude_keywords.clone();
    if let Some(ref extra) = cli.exclude_keywords {
        exclude_keywords.extend(extra.clone());
    }

    let resolver_config = ResolverConfig {
        default_selection_policy: selection_policy,
        exclude_keywords,
        ignore_formats: config.assets.ignore_formats.clone(),
        prefer_formats: config.assets.prefer_formats.clone(),
        prefer_32bit_on_64bit: config.assets.prefer_32bit_on_64bit,
        fallback_to_32bit: config.assets.fallback_to_32bit,
        prefer_musl: config.assets.prefer_musl,
    };

    let allow_keyword = cli.allow_keyword();

    for pkg_str in packages {
        println!("\n{}", format!("→ {pkg_str}").bold());

        // Parse package reference (owner/repo uses --forge, forge/owner/repo explicit)
        let pkg_ref = PackageRef::parse_with_forge(pkg_str, default_forge)
            .with_context(|| format!("Invalid package reference: {pkg_str}"))?;

        // Get the provider
        let provider = registry
            .get_provider(&pkg_ref.forge)
            .with_context(|| format!("Failed to get provider for {}", pkg_ref.forge))?;

        // Fetch release: pinned version or latest
        let release = if let Some(ref tag) = pkg_ref.version {
            println!("  Pinning to version {tag}");
            match provider.get_release(&pkg_ref.owner, &pkg_ref.repo, tag).await {
                Ok(r) => r,
                Err(grel_providers::ProviderError::NotFound(e)) => {
                    eprintln!("{}", format!("Release not found: {e}").red());
                    continue;
                }
                Err(grel_providers::ProviderError::RateLimitExceeded) => {
                    eprintln!("{}", "Rate limit exceeded. Set GREL_GITHUB_TOKEN for higher limits.".red());
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
                    eprintln!("{}", "Rate limit exceeded. Set GREL_GITHUB_TOKEN for higher limits.".red());
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
            if release.name.is_empty() { &release.tag } else { &release.name },
            release.tag
        );

        if release.prerelease {
            println!("  {}", "Warning: This is a pre-release".yellow());
        }

        // Convert provider assets to RemoteAssets
        let remote_assets: Vec<RemoteAsset> = release.assets.clone();

        // Resolve assets with full details (default + alternatives)
        let Some(mut sel) = grel_core::resolve_assets_detailed(
            &remote_assets,
            &host_os,
            &host_arch,
            &resolver_config,
            allow_keyword,
        ) else {
            eprintln!(
                "  {}",
                format!("No compatible assets for {host_os}/{host_arch}").red()
            );
            continue;
        };

        // Determine if default asset is managed
        let is_managed_default = is_asset_managed(&sel.default, &config.assets);
        sel.default_is_managed = is_managed_default;

        // Interactive confirmation / alternative selection
        let chosen_asset = match confirm_asset_selection(
            &sel,
            &config,
            noconfirm,
            allow_keyword,
        ) {
            Some(asset) => asset,
            None => {
                eprintln!("  Skipped");
                continue;
            }
        };

        let is_managed = is_asset_managed(&chosen_asset, &config.assets);

        // Determine paths
        let install_dir = if is_managed {
            config.paths.install_root.join(format!(
                "{}/{}/{}",
                pkg_ref.forge, pkg_ref.owner, pkg_ref.repo
            ))
        } else {
            config.paths.download_dir.clone()
        };
        let archive_path = install_dir.join(&chosen_asset.filename);

        if cli.dry_run {
            println!(
                "  {}",
                format!("(dry-run: would download {})", chosen_asset.filename).yellow()
            );
            continue;
        }

        // Ensure destination exists
        std::fs::create_dir_all(&install_dir).map_err(|e| {
            anyhow::anyhow!("Failed to create directory '{}': {e}", install_dir.display())
        })?;

        println!(
            "  Downloading: {} ({})",
            chosen_asset.filename,
            format_size(chosen_asset.size_bytes.unwrap_or(0))
        );

        // Download the asset
        let checksum = grel_network::download::download_file(
            &client,
            &chosen_asset.url,
            &archive_path,
            None,
        )
        .await
        .with_context(|| format!("Failed to download {}", chosen_asset.filename))?;

        // If managed, extract and install
        let installed_binaries = if is_managed {
            match grel_network::archive::install_asset(
                &archive_path,
                &install_dir,
                &config.paths.bin_dir,
                &chosen_asset.filename,
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
                    eprintln!(
                        "  {}",
                        format!("Warning: Failed to extract: {e}").yellow()
                    );
                    vec![]
                }
            }
        } else {
            vec![]
        };

        // Convert binary paths to filenames for DB tracking
        let bin_filenames: Vec<String> = installed_binaries
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .collect();

        // Optionally clean up the downloaded archive
        if is_managed && !config.general.keep_archives && archive_path.exists() {
            std::fs::remove_file(&archive_path).ok();
            println!(
                "  {}",
                "Cleaned up downloaded archive".dimmed()
            );
        }

        // Update database
        let mut pkg = models::InstalledPackage::new(
            pkg_ref.forge.to_string(),
            pkg_ref.owner.clone(),
            pkg_ref.repo.clone(),
        );
        pkg.version = release.tag.clone();
        pkg.asset_filename = chosen_asset.filename.clone();
        pkg.checksum = Some(checksum);
        pkg.install_path = install_dir.to_string_lossy().to_string();
        pkg.set_binary_list(bin_filenames);
        pkg.is_managed = is_managed;
        pkg.status = models::PackageStatus::Active;

        db.upsert_package(&pkg)
            .await
            .with_context(|| "Failed to update package record")?;

        println!(
            "  {}",
            format!("Installed {} v{}", pkg_ref.to_short_ref(), release.tag).green()
        );
    }

    db.close().await;
    Ok(())
}

/// Present the resolved asset selection to the user, show alternatives,
/// and return the chosen asset (or None if skipped).
fn confirm_asset_selection(
    sel: &grel_core::AssetSelection,
    config: &Config,
    noconfirm: bool,
    _allow_keyword: bool,
) -> Option<RemoteAsset> {
    // In non-interactive mode, always accept the default
    if !noconfirm && !grel_cli::is_interactive() {
        return Some(sel.default.clone());
    }

    // Show the selected asset
    let managed_tag = if sel.default_is_managed { "managed" } else { "unmanaged" };

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

    // Show alternatives
    if !sel.alternatives.is_empty() {
        println!(
            "  {}",
            "Other compatible assets:".dimmed()
        );
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

        // Empty input = accept default
        if input.is_empty() {
            return Some(sel.default.clone());
        }

        // Skip
        if input == "s" || input == "skip" {
            return None;
        }

        // Try to parse as a number
        if let Ok(n) = input.parse::<usize>() {
            if n == 1 {
                return Some(sel.default.clone());
            }
            if n >= 2 && n <= 1 + sel.alternatives.len() {
                let chosen = sel.alternatives[n - 2].clone();
                // Warn if the chosen alternative is unmanaged
                if !is_asset_managed(&chosen, &config.assets) {
                    eprintln!(
                        "  {}",
                        format!(
                            "Warning: This asset requires manual installation.",
                        )
                        .yellow()
                    );
                }
                return Some(chosen);
            }
        }

        // Also accept y/n for single-option case
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

/// Search for packages on the forge
async fn cmd_search(
    config: &Config,
    pattern: String,
    default_forge: Forge,
    max_results: usize,
) -> Result<()> {
    println!("{}", format!("Searching for \"{pattern}\" on {default_forge}...").bold());

    let client = build_http_client(&config.general)?;
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let registry = ProviderRegistry::new(client, github_token);

    let provider = registry
        .get_provider(&default_forge)
        .with_context(|| format!("Provider for {default_forge} not available"))?;

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
async fn cmd_sync_refresh(config: &Config, _default_forge: Forge) -> Result<()> {
    println!("{}", "Synchronizing package database...".bold());

    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

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

    let client = build_http_client(&config.general)?;
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let registry = ProviderRegistry::new(client, github_token);

    let mut up_to_date = 0;
    let mut available_updates = 0;
    let mut orphaned = 0;

    for pkg in &active_packages {
        let forge: Forge = pkg.forge.parse().unwrap_or(Forge::GitHub);

        print!("  {:<35} ", pkg.package_ref());

        let provider = match registry.get_provider(&forge) {
            Ok(p) => p,
            Err(_) => {
                println!("{}", "provider unavailable".red());
                continue;
            }
        };

        match provider.latest_release(&pkg.owner, &pkg.repo).await {
            Ok(release) => {
                if release.tag == pkg.version {
                    println!("{}", "up to date".green());
                    up_to_date += 1;
                } else {
                    println!("{}", format!("{} -> {}", pkg.version, release.tag).yellow());
                    available_updates += 1;
                }
            }
            Err(grel_providers::ProviderError::NotFound(_)) => {
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

    db.close().await;
    Ok(())
}

/// Clean artifact cache
async fn cmd_clean_cache(_config: &Config) -> Result<()> {
    println!("{}", "Cleaning download cache...".bold());
    // TODO: Implement cache cleanup
    println!("  {}", "Cache cleanup not yet implemented".yellow());
    Ok(())
}

// ---------------------------------------------------------------------------
// Upgrade all packages (-Su / -Syu)
// ---------------------------------------------------------------------------

async fn cmd_upgrade(
    config: &Config,
    noconfirm: bool,
    default_forge: Forge,
    refresh: bool,
) -> Result<()> {
    if refresh {
        println!("{}", "Synchronizing package database...".bold());
    }
    println!("{}", "Checking for upgrades...".bold());

    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

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

    let _ = (active_packages.len(), noconfirm, default_forge, refresh, db);
    // TODO: Implement full upgrade pipeline
    println!("  {}", "Upgrade pipeline not yet fully implemented".yellow());
    Ok(())
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

async fn cmd_list(config: &Config, quiet: bool, explicit: bool) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    let packages = db.list_packages().await?;

    if quiet {
        for pkg in &packages {
            if explicit && !pkg.is_managed {
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

    let active_count = packages.iter().filter(|p| matches!(p.status, models::PackageStatus::Active)).count();
    let orphaned_count = packages.iter().filter(|p| matches!(p.status, models::PackageStatus::Orphaned)).count();

    println!("{}", format!("Installed packages ({active_count} active, {orphaned_count} orphaned)").bold());
    println!();

    for pkg in &packages {
        if explicit && !pkg.is_managed {
            continue;
        }

        let status_icon = match pkg.status {
            models::PackageStatus::Active => "green",
            models::PackageStatus::Orphaned => "yellow",
            models::PackageStatus::Migrated => "white",
        };

        let managed_tag = if pkg.is_managed { "managed" } else { "unmanaged" };

        print!(
            "  [{status_icon}] {:<30} {:<12} {:<10} {}",
            pkg.package_ref(),
            format!("v{}", pkg.version),
            managed_tag,
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

async fn cmd_list_orphans(config: &Config, quiet: bool) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

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

    if quiet {
        for pkg in &orphans {
            println!("{}", pkg.package_ref());
        }
    } else {
        println!("{}", format!("Orphaned packages ({})", orphans.len()).bold());
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

async fn cmd_info_local(config: &Config, pkg_ref_str: String) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    let pkg_ref = PackageRef::parse_with_forge(&pkg_ref_str, Forge::GitHub)
        .with_context(|| format!("Invalid package reference: {pkg_ref_str}"))?;

    let pkg = db.get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo).await?;

    let Some(pkg) = pkg else {
        eprintln!("{}", format!("Package not found: {}", pkg_ref.to_short_ref()).red());
        db.close().await;
        return Ok(());
    };

    println!("{}", format!("Package: {}", pkg.package_ref()).bold());
    println!("  Version:       {}", pkg.version);
    println!("  Status:        {}", pkg.status);
    println!("  Managed:       {}", pkg.is_managed);
    println!("  Asset:         {}", pkg.asset_filename);
    println!("  Install path:  {}", pkg.install_path);

    if let Some(checksum) = &pkg.checksum {
        println!("  SHA256:        {checksum}");
    }

    db.close().await;
    Ok(())
}

async fn cmd_info_remote(_config: &Config, pkg_ref_str: String, default_forge: Forge) -> Result<()> {
    let pkg_ref = PackageRef::parse_with_forge(&pkg_ref_str, default_forge)
        .with_context(|| format!("Invalid package reference: {pkg_ref_str}"))?;

    let client = build_http_client(&Config::default().general)?;
    let github_token = std::env::var("GREL_GITHUB_TOKEN").ok();
    let registry = ProviderRegistry::new(client, github_token);

    let provider = registry
        .get_provider(&pkg_ref.forge)
        .with_context(|| format!("Provider for {} not available", pkg_ref.forge))?;

    let release = provider.latest_release(&pkg_ref.owner, &pkg_ref.repo).await?;

    println!("{}", format!("Package: {}/{}", pkg_ref.owner, pkg_ref.repo).bold());
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

async fn cmd_owns(config: &Config, path: String) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    let packages = db.list_packages().await?;

    // Simple check: does any package's install_path contain the query?
    let found: Vec<_> = packages
        .iter()
        .filter(|p| p.install_path.contains(&path) || p.asset_filename.contains(&path))
        .collect();

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

async fn cmd_verify_checksums(config: &Config) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

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

    // TODO: Re-compute checksums and compare
    println!("  {}", "Checksum verification not yet implemented".yellow());

    db.close().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Remove
// ---------------------------------------------------------------------------

async fn cmd_remove(
    config: &Config,
    packages: &[String],
    noconfirm: bool,
    cli: &Cli,
) -> Result<()> {
    if packages.is_empty() {
        eprintln!("No packages specified. Usage: grel -R owner/repo");
        return Ok(());
    }

    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    for pkg_str in packages {
        let pkg_ref = PackageRef::parse_with_forge(pkg_str, Forge::GitHub)
            .with_context(|| format!("Invalid package reference: {pkg_str}"))?;

        let pkg = db.get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo).await?;

        let Some(pkg) = pkg else {
            eprintln!("{}", format!("Package not found: {pkg_str}").red());
            continue;
        };

        if !noconfirm {
            let accepted = grel_cli::ask_confirmation(
                &format!("Remove {} v{}?", pkg.package_ref(), pkg.version),
                false,
            );
            if !accepted {
                continue;
            }
        }

        let id = pkg.id.expect("Package must have an ID");

        if cli.dry_run {
            println!(
                "  {}",
                format!("(dry-run: would remove {})", pkg.package_ref()).yellow()
            );
            println!("    install dir: {}", pkg.install_path);
            for bin in pkg.binary_list() {
                println!("    binary: {}", config.paths.bin_dir.join(&bin).display());
            }
            continue;
        }

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

        db.remove_package(id).await?;

        println!("{}", format!("Removed {}", pkg_ref.to_short_ref()).green());
    }

    db.close().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

async fn cmd_migrate(config: &Config, old_ref: &str, new_ref: &str) -> Result<()> {
    let old_ref = PackageRef::parse_with_forge(old_ref, Forge::GitHub)
        .with_context(|| format!("Invalid package reference: {old_ref}"))?;
    let new_ref = PackageRef::parse_with_forge(new_ref, Forge::GitHub)
        .with_context(|| format!("Invalid package reference: {new_ref}"))?;

    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    let old_pkg = db.get_package(&old_ref.forge.to_string(), &old_ref.owner, &old_ref.repo).await?;

    let Some(old_pkg) = old_pkg else {
        eprintln!("{}", format!("Package not found: {}", old_ref.to_short_ref()).red());
        db.close().await;
        return Ok(());
    };

    let id = old_pkg.id.expect("Package must have an ID");

    let mut new_pkg = models::InstalledPackage::new(
        new_ref.forge.to_string(),
        new_ref.owner.clone(),
        new_ref.repo.clone(),
    );
    new_pkg.version = old_pkg.version.clone();
    new_pkg.asset_filename = old_pkg.asset_filename.clone();
    new_pkg.checksum = old_pkg.checksum.clone();
    new_pkg.install_path = old_pkg.install_path.clone();
    new_pkg.is_managed = old_pkg.is_managed;
    new_pkg.status = models::PackageStatus::Active;

    db.remove_package(id).await?;
    db.upsert_package(&new_pkg).await?;

    println!(
        "{}",
        format!("Migrated {} -> {}", old_ref.to_short_ref(), new_ref.to_short_ref()).green()
    );

    db.close().await;
    Ok(())
}

async fn cmd_db_clean(config: &Config) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    // Remove orphaned packages
    let packages = db.list_packages().await?;
    let mut removed = 0;
    for pkg in &packages {
        if matches!(pkg.status, models::PackageStatus::Orphaned) {
            if let Some(id) = pkg.id {
                db.remove_package(id).await?;
                removed += 1;
            }
        }
    }

    println!("Removed {removed} orphaned package(s)");
    // TODO: Clear stale ETags/IP cache
    println!("  {}", "ETag/IP cache cleanup not yet implemented".yellow());

    db.close().await;
    Ok(())
}

async fn cmd_db_check(config: &Config) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    // Basic integrity check: can we list packages?
    let count = db.list_packages().await?.len();
    println!("Database integrity check: {} package records found", count);
    println!("{}", "Database appears healthy".green());

    db.close().await;
    Ok(())
}

async fn cmd_db_dump(config: &Config) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    let packages = db.list_packages().await?;
    // Simple JSON dump
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

async fn cmd_db_as_explicit(config: &Config, pkg_ref_str: &str) -> Result<()> {
    eprintln!("Marking {} as explicitly installed (stub)", pkg_ref_str);
    let _ = config;
    Ok(())
}

async fn cmd_db_as_deps(config: &Config, pkg_ref_str: &str) -> Result<()> {
    eprintln!("Marking {} as dependency (stub)", pkg_ref_str);
    let _ = config;
    Ok(())
}

// ---------------------------------------------------------------------------
// Upgrade from local file (-U)
// ---------------------------------------------------------------------------

async fn cmd_upgrade_local(
    _config: &Config,
    targets: &[String],
    _noconfirm: bool,
    cli: &Cli,
) -> Result<()> {
    if targets.is_empty() && cli.local_asset.is_none() {
        eprintln!("No file specified. Usage: grel -U ./package.tar.gz");
        return Ok(());
    }
    // TODO: Implement local file install
    println!("  {}", "Local file upgrade not yet implemented".yellow());
    Ok(())
}

// ---------------------------------------------------------------------------
// Files (-F)
// ---------------------------------------------------------------------------

async fn cmd_file_search(config: &Config, pattern: String, quiet: bool) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    let packages = db.list_packages().await?;
    let lower = pattern.to_lowercase();

    for pkg in &packages {
        if pkg.asset_filename.to_lowercase().contains(&lower)
            || pkg.install_path.to_lowercase().contains(&lower)
        {
            if quiet {
                println!("{}", pkg.install_path);
            } else {
                println!("  {}  {}", pkg.package_ref(), pkg.asset_filename);
            }
        }
    }

    db.close().await;
    Ok(())
}

async fn cmd_file_list(config: &Config, pkg_ref_str: String, quiet: bool) -> Result<()> {
    let db_path = config.paths.install_root.join("state.sqlite");
    let db = Database::init(&db_path)
        .await
        .with_context(|| format!("Failed to initialize database at {}", db_path.display()))?;

    let pkg_ref = PackageRef::parse_with_forge(&pkg_ref_str, Forge::GitHub)
        .with_context(|| format!("Invalid package reference: {pkg_ref_str}"))?;

    let pkg = db.get_package(&pkg_ref.forge.to_string(), &pkg_ref.owner, &pkg_ref.repo).await?;

    match pkg {
        Some(p) => {
            if quiet {
                println!("{}", p.install_path);
            } else {
                println!("{}: {}", p.package_ref(), p.install_path);
            }
        }
        None => {
            eprintln!("Package not found: {}", pkg_ref.to_short_ref());
        }
    }

    db.close().await;
    Ok(())
}

async fn cmd_reindex(_config: &Config) -> Result<()> {
    println!("{}", "Rebuilding file index...".bold());
    // TODO: Re-scan bin_dir and download_dir
    println!("  {}", "Reindex not yet implemented".yellow());
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if an asset should be managed (auto-installed)
fn is_asset_managed(asset: &RemoteAsset, asset_config: &grel_config::AssetConfig) -> bool {
    if asset_config.ignore_formats.iter().any(|p| {
        if let Some(ext) = p.strip_prefix("*.") {
            asset.tokens.format == *ext
        } else {
            asset.tokens.format == *p
        }
    }) {
        return false;
    }

    let lower = asset.filename.to_lowercase();
    if asset_config.exclude_keywords.iter().any(|k| lower.contains(&k.to_lowercase())) {
        return false;
    }

    true
}

/// Format byte size for display
fn format_size(bytes: u64) -> String {
    grel_cli::format_size(bytes)
}
