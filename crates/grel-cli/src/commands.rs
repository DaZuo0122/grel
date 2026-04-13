//! CLI argument definitions using pacman-style operation flags.
//!
//! Usage: `grel <OPERATION> [OPTIONS] [TARGETS...]`
//!
//! Operations (mutually exclusive, first one wins):
//!   -S  --sync       Fetch & install from forges
//!   -Q  --query      Inspect local state
//!   -R  --remove     Uninstall packages
//!   -D  --database   Local DB & state management
//!   -U  --upgrade    Install from local file
//!   -F  --files      File index & binary search

use clap::Parser;
use grel_core::Forge;

/// grel - A package manager for pre-built binaries from Git forges
///
/// Operations (first one wins, flags can be stacked):
///   -S   Sync/install packages from forges
///   -Q   Query installed packages
///   -R   Remove packages
///   -D   Database management
///   -U   Upgrade from local file
///   -F   File search & index
///
/// Examples:
///   grel -S foo/bar              Install a package
///   grel -Ss ripgrep             Search for packages
///   grel -Sy                     Refresh metadata
///   grel -Su                     Upgrade installed (cached)
///   grel -Syu                    Refresh + upgrade
///   grel -Ql                     List installed
///   grel -Qi foo/bar             Show local package info
///   grel -R foo/bar              Remove a package
#[derive(Parser, Debug)]
#[command(
    name = "grel",
    version,
    about = "A package manager for pre-built binaries from Git forges",
    long_about = None,
    override_usage = "grel <OPERATION> [OPTIONS] [TARGETS...]"
)]
pub struct Cli {
    // -----------------------------------------------------------------------
    // Operation flags (mutually exclusive, first wins)
    // -----------------------------------------------------------------------

    /// -S, --sync: Fetch & install from forges
    #[arg(
        short = 'S', long = "sync", action = clap::ArgAction::SetTrue,
        help = "Sync/install packages from forges",
        long_help = "Fetch and install packages from Git forges.\n\
                     Sub-options: -s (search), -y (refresh), -u (sysupgrade),\n\
                     -i (info), -c (clean), --dry-run, --noconfirm,\n\
                     --overwrite, --asset, --platform"
    )]
    pub op_sync: bool,

    /// -Q, --query: Inspect local state
    #[arg(
        short = 'Q', long = "query", action = clap::ArgAction::SetTrue,
        help = "Query installed packages",
        long_help = "Inspect local package state and installed files.\n\
                     Sub-options: -l (list), -i (info), -o (owns),\n\
                     -q (quiet), -e (explicit), -k (check), --orphans"
    )]
    pub op_query: bool,

    /// -R, --remove: Uninstall
    #[arg(
        short = 'R', long = "remove", action = clap::ArgAction::SetTrue,
        help = "Remove packages",
        long_help = "Uninstall packages from the system.\n\
                     Sub-options: -c (cascade), -n (nosave),\n\
                     -s (recursive), --noconfirm, --dry-run"
    )]
    pub op_remove: bool,

    /// -D, --database: DB management
    #[arg(
        short = 'D', long = "database", action = clap::ArgAction::SetTrue,
        help = "Database management",
        long_help = "Manage local SQLite database and state.\n\
                     Sub-options: --asexplicit, --asdeps, --migrate,\n\
                     --clean, --check, --dump"
    )]
    pub op_database: bool,

    /// -U, --upgrade: Install from local file
    #[arg(
        short = 'U', long = "upgrade", action = clap::ArgAction::SetTrue,
        help = "Upgrade from local file",
        long_help = "Install a package from a local archive file.\n\
                     Sub-options: --overwrite, --noconfirm, --asset"
    )]
    pub op_upgrade: bool,

    /// -F, --files: File search
    #[arg(
        short = 'F', long = "files", action = clap::ArgAction::SetTrue,
        help = "File search & index",
        long_help = "Search and list files from installed packages.\n\
                     Sub-options: -s (search), -l (list), -y (refresh), -q (quiet)"
    )]
    pub op_files: bool,

    // -----------------------------------------------------------------------
    // -S (sync) sub-options
    // -----------------------------------------------------------------------

    /// -s: Search forges for packages
    #[arg(
        short = 's', long = "search",
        help = "Search forges for packages (use with -S)",
        value_name = "PATTERN"
    )]
    pub sync_search: Option<String>,

    /// -y: Refresh metadata / rebuild index
    #[arg(
        short = 'y', long, action = clap::ArgAction::SetTrue,
        help = "Refresh forge metadata (with -S) or rebuild index (with -F)"
    )]
    pub refresh: bool,

    /// -u: Sysupgrade
    #[arg(
        short = 'u', long, action = clap::ArgAction::SetTrue,
        help = "Upgrade all installed packages (with -S)"
    )]
    pub sysupgrade: bool,

    /// -i: Show info (used with -S or -Q)
    #[arg(
        short = 'i', long,
        help = "Show package info (with -S: remote, with -Q: local)",
        value_name = "PKG"
    )]
    pub info: Option<String>,

    /// -c: Clean (used with -S or -R)
    #[arg(
        short = 'c', long, action = clap::ArgAction::SetTrue,
        help = "Purge artifact cache (with -S) or cascade (with -R)"
    )]
    pub clean: bool,

    // -----------------------------------------------------------------------
    // -Q (query) sub-options
    // -----------------------------------------------------------------------

    /// -l: List (used with -Q or -F)
    #[arg(
        short = 'l', long, action = clap::ArgAction::SetTrue,
        help = "List packages (with -Q) or files (with -F <pkg>)",
        value_name = "PKG"
    )]
    pub list: Option<String>,

    /// -o: Owns (used with -Q)
    #[arg(
        short = 'o', long,
        help = "Find which package owns a file",
        value_name = "PATH"
    )]
    pub owns: Option<String>,

    /// -q: Quiet mode
    #[arg(
        short = 'q', long, action = clap::ArgAction::SetTrue,
        help = "Quiet/minimal output (with -Q or -F)"
    )]
    pub quiet: bool,

    /// -e: Explicit only
    #[arg(
        short = 'e', long, action = clap::ArgAction::SetTrue,
        help = "Filter to manually installed packages (with -Q)"
    )]
    pub explicit: bool,

    /// -k: Check checksums
    #[arg(
        short = 'k', long, action = clap::ArgAction::SetTrue,
        help = "Verify checksums of installed files (with -Q)"
    )]
    pub check: bool,

    /// --orphans
    #[arg(
        long, action = clap::ArgAction::SetTrue,
        help = "Show orphaned packages (with -Q)"
    )]
    pub orphans: bool,

    // -----------------------------------------------------------------------
    // -F (files) sub-options
    // -----------------------------------------------------------------------

    /// -s for files search
    #[arg(
        long = "file-search",
        help = "Search installed packages for a filename (with -F)",
        value_name = "PATTERN"
    )]
    pub file_search: Option<String>,

    /// -l for files list (with -F <pkg>)
    #[arg(
        long = "file-list",
        help = "List all files from a package (with -F)",
        value_name = "PKG"
    )]
    pub file_list: Option<String>,

    // -----------------------------------------------------------------------
    // -R (remove) sub-options
    // -----------------------------------------------------------------------

    /// -n: nosave
    #[arg(
        short = 'n', long, action = clap::ArgAction::SetTrue,
        help = "Do not preserve extracted files (with -R)"
    )]
    pub nosave: bool,

    /// -r: recursive removal
    #[arg(
        short = 'r', long = "recursive", action = clap::ArgAction::SetTrue,
        help = "Remove packages that depend on target (with -R)"
    )]
    pub recursive: bool,

    // -----------------------------------------------------------------------
    // -D (database) sub-options
    // -----------------------------------------------------------------------

    #[arg(long, value_name = "PKG")]
    pub asexplicit: Option<String>,

    #[arg(long, value_name = "PKG")]
    pub asdeps: Option<String>,

    #[arg(long, value_names = &["OLD", "NEW"], num_args = 2)]
    pub migrate: Option<Vec<String>>,

    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub db_clean: bool,

    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub db_check: bool,

    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub db_dump: bool,

    // -----------------------------------------------------------------------
    // -U (upgrade local) sub-options
    // -----------------------------------------------------------------------

    #[arg(long, value_name = "PATH")]
    pub local_asset: Option<String>,

    // -----------------------------------------------------------------------
    // Common/global options
    // -----------------------------------------------------------------------

    /// Target packages (for -S, -R) or file path (for -U)
    #[arg(value_name = "TARGETS")]
    pub targets: Vec<String>,

    /// --dry-run
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub dry_run: bool,

    /// --noconfirm
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub noconfirm: bool,

    /// --overwrite
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub overwrite: bool,

    /// --asset <name>
    #[arg(long, value_name = "NAME")]
    pub asset: Option<String>,

    /// --platform <os/arch>
    #[arg(long, value_name = "OS/ARCH")]
    pub platform: Option<String>,

    /// --exclude-keywords
    #[arg(long, value_name = "K1,K2", value_delimiter = ',')]
    pub exclude_keywords: Option<Vec<String>>,

    /// --allow-keyword
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub allow_keyword: bool,

    /// --allow-format
    #[arg(long, value_name = "FMT")]
    pub allow_format: Option<String>,

    /// -C, --config
    #[arg(short = 'C', long)]
    pub config: Option<String>,

    /// -f, --forge
    #[arg(short = 'f', long = "forge", default_value = "github")]
    pub default_forge: ForgeArg,

    /// --proxy
    #[arg(long)]
    pub proxy: Option<String>,
}

/// Forge argument wrapper for clap integration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForgeArg(pub Forge);

impl clap::ValueEnum for ForgeArg {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            ForgeArg(Forge::GitHub),
            ForgeArg(Forge::GitLab),
            ForgeArg(Forge::Gitea),
            ForgeArg(Forge::Codeberg),
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self.0 {
            Forge::GitHub => Some(clap::builder::PossibleValue::new("github")),
            Forge::GitLab => Some(clap::builder::PossibleValue::new("gitlab")),
            Forge::Gitea => Some(clap::builder::PossibleValue::new("gitea")),
            Forge::Codeberg => Some(clap::builder::PossibleValue::new("codeberg")),
        }
    }
}

/// Determine which operation was requested (first wins)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Sync,
    Query,
    Remove,
    Database,
    Upgrade,
    Files,
    Help,
}

impl Cli {
    /// Determine the operation from parsed flags (first operation flag wins)
    pub fn operation(&self) -> Operation {
        if self.op_sync {
            return Operation::Sync;
        }
        if self.op_query {
            return Operation::Query;
        }
        if self.op_remove {
            return Operation::Remove;
        }
        if self.op_database {
            return Operation::Database;
        }
        if self.op_upgrade {
            return Operation::Upgrade;
        }
        if self.op_files {
            return Operation::Files;
        }
        Operation::Help
    }

    pub fn allow_keyword(&self) -> bool {
        self.allow_keyword
    }
}
