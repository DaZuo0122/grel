> **Note:** This is a historical design document. The feature described here has been largely implemented in `grel-elf`. For current behavior, see [ARCHITECTURE.md](ARCHITECTURE.md) and [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

# ELF System Dependency Auto-Resolution Design

> **Date:** 2026-05-09  
> **Scope:** Linux-only feature to parse installed ELF binaries, discover missing shared-library dependencies, and optionally install them via the host distro's package manager.  
> **Status:** Design document for review

---

## 1. Goals & Non-Goals

### Goals
- Parse `DT_NEEDED` entries from ELF binaries after extraction/installation.
- Cross-reference discovered libraries against the live system (`ldconfig -p`).
- Map missing libraries to distro-specific package names.
- Generate and (optionally) execute install commands via the native package manager.
- Make every step configurable, overridable, and disable-able.

### Non-Goals
- Not a replacement for `ldd` or static analysis tools.
- Does not resolve *grel* dependencies (those stay in `.grel.toml` manifests).
- Does not attempt to install on non-Linux platforms (graceful no-op).
- Does not build a universal package-name database (relies on distro tooling + user overrides).

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│  grel -S foo/bar                                                    │
│     └── install_asset() extracts binaries                           │
│            └── grel_elf::resolve_elf_deps(&bin_paths, ctx)          │
│                     ├── ElfParser: read DT_NEEDED                   │
│                     ├── DistroDetector: read /etc/os-release        │
│                     ├── SysdepChecker: extends grel-network's       │
│                     │       check_system_deps() (ldconfig -p)       │
│                     ├── PkgManager::resolve(lib) → package name     │
│                     │       (apt-file, dnf repoquery, pkgfile, …)   │
│                     └── PkgManager::install(pkgs) or print cmd      │
└─────────────────────────────────────────────────────────────────────┘
```

A new workspace crate **`grel-elf`** is introduced so that ELF parsing, distro detection, and package-manager logic are isolated, testable, and compile-gated on Linux. The existing `grel-network::system_deps` module already handles `ldconfig -p` discovery and is reused here for the availability check step.

---

## 3. New Crate: `grel-elf`

### 3.1 Dependencies
| Crate | Purpose |
|-------|---------|
| `goblin` | Pure-Rust ELF parsing (no native deps, no `libelf`). |
| `grel-config` | Read `ElfDepConfig`. |
| `grel-cache` | Cache resolved `lib → package` mappings in SQLite. |
| `grel-network` | Reuse `check_system_deps()` for `ldconfig -p` availability check. |
| `serde` + `toml` | Parse user-supplied library-to-package override file. |

`grel-elf` is a **regular workspace dependency** (not platform-conditional at the workspace level). Platform gating is applied internally via `#[cfg(target_os = "linux")]` so that non-Linux builds still compile against the no-op public API (see §8).

### 3.2 Module Layout
```
crates/grel-elf/src/
  lib.rs          # Public API: resolve_elf_deps()
  parser.rs       # ElfParser: DT_NEEDED extraction
  distro.rs       # DistroDetector: /etc/os-release parsing
  pkgmgr.rs       # PkgManager trait + built-in backends
  resolver.rs     # SystemDepResolver: orchestration + caching
```

### 3.3 `ElfParser` (`parser.rs`)
- Uses `goblin::elf::Elf` to parse the `.dynamic` section.
- Extracts `DT_NEEDED` strings (e.g., `libssl.so.3`, `libc.so.6`).
- Skips non-ELF files gracefully.
- Returns `Vec<String>` of required library names (basename only).
- File reads are blocking (`std::fs::read`); callers in async context must wrap with `tokio::task::spawn_blocking`.

```rust
pub struct ElfParser;
impl ElfParser {
    pub fn needed_libs(path: &Path) -> Result<Vec<String>, ElfParseError>;
}
```

### 3.4 `DistroDetector` (`distro.rs`)
- Reads `/etc/os-release` into `OsRelease { id, id_like, name }`.
- Normalizes distro families:
  - `debian`, `ubuntu`, `linuxmint` → **DebianFamily**
  - `fedora`, `rhel`, `centos`, `almalinux`, `rocky` → **RedHatFamily**
  - `arch`, `manjaro` → **ArchFamily**
  - `alpine` → **AlpineFamily**
  - `opensuse`, `suse` → **SuseFamily**

```rust
pub struct DistroInfo {
    pub id: String,
    pub id_like: Vec<String>,
    pub family: DistroFamily,
}
```

### 3.5 `PkgManager` Trait (`pkgmgr.rs`)
Each backend implements:
```rust
pub trait PkgManager: Send + Sync {
    /// Human-readable name
    fn name(&self) -> &str;

    /// Is this manager available on the current system?
    /// Implementations must check both the package manager binary
    /// AND any auxiliary resolution tool (e.g., apt-file for Debian).
    fn detect(&self) -> bool;

    /// Resolve a library basename to a package name.
    /// Returns `None` if the manager cannot resolve it.
    /// Implementations invoke external processes via `std::process::Command`.
    /// Callers in async context must wrap with `tokio::task::spawn_blocking`.
    fn resolve_lib(&self, lib: &str) -> Result<Option<String>, PkgMgrError>;

    /// Build the install command for a list of packages.
    fn install_cmd(&self, pkgs: &[String]) -> Vec<String>;
}
```

Built-in backends:

| Family | Manager | Detection | Resolution Command | Install Command |
|--------|---------|-----------|-------------------|-----------------|
| DebianFamily | `AptPkgManager` | `apt-file` exists | `apt-file search -l {lib}` | `apt-get install -y {pkgs}` |
| RedHatFamily | `DnfPkgManager` | `dnf` exists | `dnf repoquery --whatprovides '*{lib}*'` | `dnf install -y {pkgs}` |
| ArchFamily | `PacmanPkgManager` | `pacman` exists | `pkgfile -r '{lib}'` (fallback: `pacman -Fy && pacman -F '{lib}'`) | `pacman -S --noconfirm {pkgs}` |
| AlpineFamily | `ApkPkgManager` | `apk` exists | `apk info --who-owns '{lib}'` | `apk add {pkgs}` |
| SuseFamily | `ZypperPkgManager` | `zypper` exists | `zypper search --provides '{lib}'` | `zypper install -y {pkgs}` |

**Note on Debian detection:** `apt-file` is a separate package not installed by default on Debian/Ubuntu. `detect()` must check for `apt-file`, not `apt-get`. If `apt-file` is absent, the manager falls back to the heuristics described below.

**Fallback chain when the resolution command is unavailable or fails:**
1. The DB cache (`system_dep_cache` table).
2. The user's override config (`library_map` / `distro_library_map`).
3. A heuristic guess (strip `.so.X` suffix → package name).

### 3.6 `SystemDepResolver` (`resolver.rs`)
Orchestrates the full flow:
```rust
pub struct SystemDepResolver<'a> {
    config: &'a ElfDepConfig,
    db: Option<&'a Database>,
    pkg_manager: Box<dyn PkgManager>,
}

impl<'a> SystemDepResolver<'a> {
    pub fn new(
        config: &'a ElfDepConfig,
        db: Option<&'a Database>,
        pkg_manager: Box<dyn PkgManager>,
    ) -> Self;

    pub async fn resolve(&self, elf_paths: &[PathBuf]) -> Result<ResolutionReport, Error>;
}
```

**Flow:**
1. Collect all `DT_NEEDED` from all ELF paths via `spawn_blocking(|| ElfParser::needed_libs(...))` (deduplicate).
2. Filter out libraries already present using `grel_network::system_deps::check_system_deps()`.
3. For each missing library:
   a. Check DB cache (`system_dep_cache`).
   b. Check user override config (`library_map` / `distro_library_map`).
   c. Call `spawn_blocking(|| pkg_manager.resolve_lib(lib))`.
   d. Store successful discovery in DB cache.
4. Build `ResolutionReport`:
   ```rust
   pub struct ResolvedPackage {
       pub lib: String,
       pub package: String,
   }

   #[derive(Default)]
   pub struct ResolutionReport {
       pub missing_libs: Vec<String>,
       pub resolved_pkgs: Vec<ResolvedPackage>,
       pub unresolved_libs: Vec<String>,
       pub install_cmd: Option<Vec<String>>,
   }
   ```
5. If `auto_resolve_system_deps` is enabled and `resolved_pkgs` is non-empty:
   - Print the command via `tracing::info!`.
   - Prompt for confirmation (unless `--noconfirm` is set).
   - Execute via `tokio::process::Command`. Prepend `sudo` if the current process is not running as root (`nix::unistd::getuid() != 0`).

---

## 4. Configuration & Flags

### 4.1 New Config Section: `ElfDepConfig`
Added to `grel-config/src/config.rs` and included as `pub elf_deps: ElfDepConfig` in the top-level `Config` struct:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElfDepConfig {
    /// Master switch. Default: true.
    #[serde(default = "default_true")]
    pub auto_resolve_system_deps: bool,

    /// Print discovered libraries before resolving. Default: true.
    #[serde(default = "default_true")]
    pub show_parsed_deps: bool,

    /// Override distro detection (e.g., "debian", "fedora", "arch").
    #[serde(default)]
    pub distro_override: Option<String>,

    /// Override the package manager command template.
    /// e.g., "sudo apt-get install -y {packages}"
    #[serde(default)]
    pub install_cmd_template: Option<String>,

    /// Global user-defined library → package mappings (applies to all distros).
    #[serde(default)]
    pub library_map: HashMap<String, String>,

    /// Per-distro user-defined library → package mappings.
    /// Keys are distro IDs (e.g., "fedora", "debian"). Takes precedence
    /// over `library_map` when the active distro matches.
    #[serde(default)]
    pub distro_library_map: HashMap<String, HashMap<String, String>>,
}
```

Environment variable equivalents:
- `GREL_ELF_AUTO_RESOLVE_SYSTEM_DEPS=true|false`
- `GREL_ELF_SHOW_PARSED_DEPS=true|false`
- `GREL_ELF_DISTRO_OVERRIDE=debian`

### 4.2 New CLI Flags
Added to `grel-cli/src/commands.rs` under global options:
```rust
/// --auto-resolve-deps
#[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_auto_resolve_deps")]
pub auto_resolve_deps: bool,

/// --no-auto-resolve-deps
#[arg(long, action = ArgAction::SetTrue, conflicts_with = "auto_resolve_deps")]
pub no_auto_resolve_deps: bool,

/// --show-parsed-deps
#[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_show_parsed_deps")]
pub show_parsed_deps: bool,

/// --no-show-parsed-deps
#[arg(long, action = ArgAction::SetTrue, conflicts_with = "show_parsed_deps")]
pub no_show_parsed_deps: bool,
```

CLI overrides take precedence over config file values.

### 4.3 Optional User Override File
Users may create `~/.config/grel/system-dep-map.toml`:
```toml
[library_map]
"libssl.so.3" = "libssl3"
"libcrypto.so.3" = "libssl3"
"libcurl.so.4" = "libcurl4"

[distro_library_map.fedora]
"libssl.so.3" = "openssl-libs"
"libcrypto.so.3" = "openssl-libs"
```
This file is merged into `ElfDepConfig` at load time. `distro_library_map` entries override `library_map` when the active distro matches.

---

## 5. Database Schema Addition

A new table caches discovered `library → package` mappings per distro to avoid slow package-manager queries on subsequent installs.

```sql
CREATE TABLE IF NOT EXISTS system_dep_cache (
    library_name TEXT NOT NULL,
    distro_id TEXT NOT NULL,
    package_name TEXT NOT NULL,
    discovered_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (library_name, distro_id)
);
```

DB methods added to `grel-cache/src/database.rs`:
```rust
pub async fn get_cached_system_dep(&self, lib: &str, distro: &str) -> Result<Option<String>, DatabaseError>;
pub async fn set_cached_system_dep(&self, lib: &str, distro: &str, pkg: &str) -> Result<(), DatabaseError>;
```

---

## 6. Integration Points

### 6.1 Sync Install (`commands/sync.rs`)
After `install_asset()` succeeds and binaries are linked:
```rust
#[cfg(target_os = "linux")]
{
    if ctx.config.elf_deps.auto_resolve_system_deps {
        let bin_paths = /* collect all installed binary paths */;
        match grel_elf::resolve_elf_deps(&bin_paths, &ctx.config.elf_deps, Some(&db)).await {
            Ok(report) => {
                if ctx.config.elf_deps.show_parsed_deps {
                    tracing::info!("Discovered system dependencies: {:?}", report.missing_libs);
                }
                if !report.resolved_pkgs.is_empty() {
                    // prompt / execute
                }
            }
            Err(e) => tracing::warn!("ELF dep resolution failed: {}", e),
        }
    }
}
```

### 6.2 Upgrade (`commands/sync.rs` `cmd_upgrade`)
When upgrading an existing package, re-run ELF dep resolution in case the new release links against newer system libraries. Same flow as sync install after extraction.

---

## 7. Security & Safety

1. **No silent privilege escalation.** Install commands are printed via `tracing::info!` before execution. `sudo` is prepended to the install command only when the current process is not running as root. Auto-execution only proceeds when `--noconfirm` is passed; otherwise the user is prompted.
2. **No network calls in resolver.** All resolution uses local package-manager indexes (`apt-file`, `dnf` metadata, etc.).
3. **ELF parsing is read-only.** `goblin` does not execute the binary; it only parses headers.
4. **Path sanitization.** Only files inside `install_dir/extracted/` and `bin_dir/` are scanned.
5. **Graceful degradation.** If `goblin` fails, `ldconfig` fails, or the package manager is unavailable, the install continues with a warning.

---

## 8. Platform Gating

`grel-elf` is added as a **regular workspace dependency** in all crates that use it. Platform gating is handled internally within the crate:

- All Linux-specific implementation code is annotated `#[cfg(target_os = "linux")]`.
- `lib.rs` exposes a no-op stub on non-Linux:
  ```rust
  #[cfg(not(target_os = "linux"))]
  pub async fn resolve_elf_deps(
      _paths: &[PathBuf],
      _config: &ElfDepConfig,
      _db: Option<&Database>,
  ) -> Result<ResolutionReport, Error> {
      Ok(ResolutionReport::default())
  }
  ```

This ensures the crate compiles on all platforms and callers require no `#[cfg]` annotations at integration sites.

---

## 9. Implementation Plan

| Phase | Task | Est. Effort |
|-------|------|-------------|
| 1 | Create `crates/grel-elf` crate, add to workspace | 30 min |
| 2 | Implement `ElfParser` with `goblin` + unit tests | 1.5 hrs |
| 3 | Implement `DistroDetector` | 1 hr |
| 4 | Implement `PkgManager` trait + 5 backends | 2.5 hrs |
| 5 | Add `system_dep_cache` DB table + methods to `grel-cache` | 1 hr |
| 6 | Implement `SystemDepResolver` (orchestration + DB cache integration) | 2 hrs |
| 7 | Add `ElfDepConfig` to `grel-config` | 1 hr |
| 8 | Add CLI flags to `grel-cli` | 30 min |
| 9 | Integrate into `sync.rs` (`cmd_sync` and `cmd_upgrade`) | 1 hr |
| 10 | End-to-end test on Debian/Ubuntu and Fedora containers | 2 hrs |
| **Total** | | **~13 hrs** |

---

## 10. Open Questions

1. **Built-in heuristic mapping as fallback:** Should we ship a small built-in table (e.g., `libssl.so.*` → common package names) for the case where distro tooling (`apt-file`, `pkgfile`, etc.) is unavailable? This would complement the DB cache; the cache stores dynamically discovered mappings while the built-in table covers the most common libraries out of the box.

2. **Recursive system dependency resolution:** Should the resolver recursively resolve dependencies of system packages (e.g., if `libssl3` depends on `libc6`, install `libc6` too)? Most package managers handle transitive deps natively during install, so this is likely unnecessary.

3. **`patchelf` / rpath modification:** Should we support modifying extracted binaries so they find libraries in non-standard paths? Out of scope for this feature, but worth noting as a future extension.
