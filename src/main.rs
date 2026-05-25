//! grel - A package manager for pre-built binaries from Git forges

use anyhow::Result;
use clap::Parser;
use grel_cli::{Cli, Operation};
use grel_config::load_config;
use owo_colors::OwoColorize;
use tracing_indicatif::IndicatifLayer;

mod commands;

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

    // Crash recovery: clean up partial state from previous interrupted runs
    if let Err(e) = commands::journal::check_and_recover() {
        eprintln!("Warning: crash recovery check failed: {e}");
    }

    // Load configuration
    let config_path = cli.config.clone().map(std::path::PathBuf::from);
    let config = load_config(config_path.as_deref())?;

    // Apply CLI overrides to config
    let mut config = config;
    if cli.verify_signatures {
        config.security.verify_signatures = true;
    }
    if cli.no_verify_signatures {
        config.security.verify_signatures = false;
    }
    if cli.enable_hooks {
        config.security.enable_hooks = true;
    }
    if cli.no_enable_hooks {
        config.security.enable_hooks = false;
    }
    if let Some(ref registry_url) = cli.registry {
        config.registry.url = registry_url.clone();
    }
    if cli.no_registry {
        config.registry.url.clear();
        config.registry.auto_update = false;
    }
    if cli.auto_resolve_deps {
        config.elf_deps.auto_resolve_system_deps = true;
    }
    if cli.no_auto_resolve_deps {
        config.elf_deps.auto_resolve_system_deps = false;
    }
    if cli.show_parsed_deps {
        config.elf_deps.show_parsed_deps = true;
    }
    if cli.no_show_parsed_deps {
        config.elf_deps.show_parsed_deps = false;
    }
    if let Some(ref proxy) = cli.proxy {
        config.general.proxy = proxy.clone();
    }

    let ctx = commands::CommandContext {
        config: &config,
        cli: &cli,
    };

    // Handle help requests (scoped or global)
    if cli.help_flag {
        match cli.operation() {
            Operation::Sync => {
                println!("{}", "grel -S, --sync — Fetch & install from forges".bold());
                println!();
                println!("Usage: grel -S [OPTIONS] [TARGETS...]");
                println!();
                println!("Options:");
                println!("  -s, --search <PATTERN>   Search forges for packages");
                println!("  -y, --refresh            Refresh forge metadata and caches");
                println!("  -u, --sysupgrade         Upgrade all installed packages");
                println!("  -i, --info <PKG>         Show release metadata before installing");
                println!("  -c, --clean              Purge downloaded artifacts from cache");
                println!("      --dry-run            Simulate without writing files");
                println!("      --noconfirm          Skip interactive prompts");
                println!("      --overwrite          Replace existing binaries if they conflict");
                println!("      --asset <NAME>       Download exact asset filename");
                println!("      --platform <OS/ARCH> Override host platform detection");
                println!("      --allow-format <FMT> Temporarily allow normally ignored formats");
                println!("      --asdeps             Install packages as dependencies");
                println!("      --asexplicit         Install packages as explicitly installed");
                println!("      --needed             Skip reinstall if already up-to-date");
                println!("  -w, --download-only      Download without installing");
                println!();
                println!("Examples:");
                println!("  grel -S foo/bar          Install latest release");
                println!("  grel -Ss ripgrep         Search for ripgrep");
                println!("  grel -Syu                Refresh + upgrade all");
            }
            Operation::Query => {
                println!(
                    "{}",
                    "grel -Q, --query — Inspect local package state".bold()
                );
                println!();
                println!("Usage: grel -Q [OPTIONS] [TARGETS...]");
                println!();
                println!("Options:");
                println!("  -l, --list [<PKG>]       List files owned by a package");
                println!("  -i, --info <PKG>         Show detailed local package info");
                println!("  -o, --owns <PATH>        Find which package owns a file");
                println!("  -q, --quiet              Minimal output (names only)");
                println!("  -e, --explicit           Filter to manually installed packages");
                println!("  -d, --deps-filter        Filter to dependency-installed packages");
                println!("  -t, --unrequired         List packages not required by any other");
                println!("  -k, --check              Verify checksums of installed archives");
                println!("  -s, --search <PATTERN>   Search locally installed packages");
                println!();
                println!("Examples:");
                println!("  grel -Q                  List all installed packages");
                println!("  grel -Ql foo/bar         List files for foo/bar");
                println!("  grel -Qo rg              Which package provides rg?");
                println!("  grel -Qk                 Verify all checksums");
            }
            Operation::Remove => {
                println!("{}", "grel -R, --remove — Uninstall packages".bold());
                println!();
                println!("Usage: grel -R [OPTIONS] [TARGETS...]");
                println!();
                println!("Options:");
                println!("  -c, --clean              Cascade: remove unneeded dependencies");
                println!("  -n, --nosave             Do not preserve config file backups");
                println!("  -r, --recursive          Remove packages that depend on the target");
                println!("  -u, --sysupgrade         Remove packages that are no longer required");
                println!("      --noconfirm          Skip removal confirmation");
                println!("      --dry-run            Show what would be removed");
                println!();
                println!("Examples:");
                println!("  grel -R foo/bar          Remove a package");
                println!("  grel -Rc foo/bar         Cascade removal");
                println!("  grel -Ru                 Remove all unneeded packages");
            }
            Operation::Database => {
                println!(
                    "{}",
                    "grel -D, --database — Local DB & state management".bold()
                );
                println!();
                println!("Usage: grel -D [OPTIONS] [TARGETS...]");
                println!();
                println!("Options:");
                println!("      --asexplicit         Mark target(s) as explicitly installed");
                println!("      --asdeps             Mark target(s) as dependencies");
                println!(
                    "      --migrate <OLD> <NEW>  Update owner/repo path for renamed projects"
                );
                println!("      --clean              Prune orphaned records and stale caches");
                println!("      --check              Verify SQLite DB integrity");
                println!("      --dump               Export state as JSON");
                println!();
                println!("Examples:");
                println!("  grel -D --clean          Clean stale caches and orphans");
                println!("  grel -D --check          Verify database integrity");
                println!("  grel -D --asexplicit foo/bar  Mark foo/bar as explicit");
            }
            Operation::Upgrade => {
                println!(
                    "{}",
                    "grel -U, --upgrade — Install from a local archive file".bold()
                );
                println!();
                println!("Usage: grel -U [OPTIONS] [TARGETS...]");
                println!();
                println!("Options:");
                println!("      --overwrite          Replace existing binaries if they conflict");
                println!("      --noconfirm          Skip prompts & warnings");
                println!("      --asset <PATH>       Treat file as direct download");
                println!();
                println!("Examples:");
                println!("  grel -U ./ripgrep.tar.gz      Install from local file");
                println!("  grel -U ./tool.exe --noconfirm");
            }
            Operation::Files => {
                println!("{}", "grel -F, --files — File index & binary search".bold());
                println!();
                println!("Usage: grel -F [OPTIONS] [TARGETS...]");
                println!();
                println!("Options:");
                println!("  -s, --search <PATTERN>   Search installed packages for a filename");
                println!("  -l, --list <PKG>         List all files extracted by a package");
                println!("  -y, --refresh            Rebuild file index from installed packages");
                println!("  -q, --quiet              Output only matching paths");
                println!();
                println!("Examples:");
                println!("  grel -Fs rg              Find which package provides rg");
                println!("  grel -Fl foo/bar         List all files from foo/bar");
            }
            Operation::Help => {
                println!(
                    "{}",
                    "grel - A package manager for pre-built binaries from Git forges".bold()
                );
                println!();
                println!("Usage: grel <OPERATION> [OPTIONS] [TARGETS...]");
                println!();
                println!("Operations:");
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
                println!("  grel -Ql               List installed files");
                println!("  grel -Qi foo/bar       Show local package info");
                println!("  grel -R foo/bar        Remove a package");
                println!();
                println!("Use `grel -Sh`, `grel -Qh`, etc. for operation-specific help.");
                println!("Use `grel --help` for full flag listing.");
            }
        }
        return Ok(());
    }

    // Route by operation (first wins)
    match cli.operation() {
        Operation::Sync => {
            if cli.sysupgrade {
                commands::sync::cmd_upgrade(&ctx).await?;
            } else if let Some(ref pattern) = cli.search {
                commands::sync::cmd_search(&ctx, pattern.clone(), 20).await?;
            } else if let Some(ref pkg) = cli.info {
                commands::query::cmd_info_remote(&ctx, pkg.clone()).await?;
            } else if cli.refresh {
                commands::sync::cmd_sync_refresh(&ctx).await?;
            } else if cli.clean {
                commands::sync::cmd_clean_cache(&ctx).await?;
            } else {
                commands::sync::cmd_sync(&ctx, &cli.targets).await?;
            }
        }
        Operation::Query => {
            if cli.unrequired {
                commands::query::cmd_list_unrequired(&ctx).await?;
            } else if cli.orphans {
                commands::query::cmd_list_orphans(&ctx).await?;
            } else if let Some(ref pkg) = cli.info {
                commands::query::cmd_info_local(&ctx, pkg.clone()).await?;
            } else if let Some(ref path) = cli.owns {
                commands::query::cmd_owns(&ctx, path.clone()).await?;
            } else if cli.check {
                commands::query::cmd_verify_checksums(&ctx).await?;
            } else if let Some(ref pattern) = cli.search {
                commands::query::cmd_local_search(&ctx, pattern.clone()).await?;
            } else if !cli.list.is_empty() || !cli.targets.is_empty() {
                let pkg = cli
                    .list
                    .first()
                    .or_else(|| cli.targets.first())
                    .map(|s| s.as_str());
                commands::query::cmd_list_files(&ctx, pkg).await?;
            } else {
                commands::query::cmd_list(&ctx).await?;
            }
        }
        Operation::Remove => {
            if cli.sysupgrade {
                if !cli.targets.is_empty() {
                    eprintln!("Warning: targets ignored with -u");
                }
                commands::remove::cmd_remove_unneeded(&ctx).await?;
            } else {
                commands::remove::cmd_remove(&ctx, &cli.targets).await?;
            }
        }
        Operation::Database => {
            if cli.db_clean || cli.clean {
                commands::database::cmd_db_clean(&ctx).await?;
            } else if cli.db_check || cli.check {
                commands::database::cmd_db_check(&ctx).await?;
            } else if cli.db_dump {
                commands::database::cmd_db_dump(&ctx).await?;
            } else if cli.asexplicit {
                commands::database::cmd_db_as_explicit(&ctx, &cli.targets).await?;
            } else if cli.asdeps {
                commands::database::cmd_db_as_deps(&ctx, &cli.targets).await?;
            } else if let Some(ref args) = cli.migrate {
                if args.len() == 2 {
                    commands::database::cmd_migrate(&ctx, &args[0], &args[1]).await?;
                } else {
                    eprintln!("Usage: grel -D --migrate <OLD> <NEW>");
                }
            } else {
                eprintln!("No database operation specified. Use -Dh for help.");
            }
        }
        Operation::Upgrade => {
            commands::upgrade::cmd_upgrade_local(&ctx).await?;
        }
        Operation::Files => {
            if let Some(ref pattern) = cli.search {
                commands::files::cmd_file_search(&ctx, pattern.clone()).await?;
            } else if !cli.list.is_empty() || !cli.targets.is_empty() {
                let Some(pkg) = cli.list.first().or_else(|| cli.targets.first()) else {
                    eprintln!("Usage: grel -Fl <package>");
                    return Ok(());
                };
                commands::files::cmd_file_list(&ctx, pkg.clone()).await?;
            } else if cli.refresh {
                commands::files::cmd_reindex(&ctx).await?;
            } else {
                eprintln!("No files operation specified. Use -Fh for help.");
            }
        }
        Operation::Help => {
            println!(
                "{}",
                "grel - A package manager for pre-built binaries from Git forges".bold()
            );
            println!();
            println!("Usage: grel <OPERATION> [OPTIONS] [TARGETS...]");
            println!();
            println!("Operations:");
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
            println!("  grel -Ql               List installed files");
            println!("  grel -Qi foo/bar       Show local package info");
            println!("  grel -R foo/bar        Remove a package");
            println!();
            println!("Use `grel -Sh`, `grel -Qh`, etc. for operation-specific help.");
            println!("Use `grel --help` for full flag listing.");
        }
    }

    Ok(())
}
