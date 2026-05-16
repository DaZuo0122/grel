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
            if cli.db_clean {
                commands::database::cmd_db_clean(&ctx).await?;
            } else if cli.db_check {
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
