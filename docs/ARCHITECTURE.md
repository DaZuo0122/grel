# Architecture Overview

This document is for contributors and advanced users who want to understand how `grel` works under the hood.

---

## Workspace Layout

```
grel-rs/
├── Cargo.toml                  # Workspace root (resolver = "2")
├── src/
│   ├── main.rs                 # CLI dispatch, config loading, help rendering
│   └── commands/               # Command handlers
│       ├── sync.rs
│       ├── query.rs
│       ├── remove.rs
│       ├── database.rs
│       ├── upgrade.rs
│       └── files.rs
├── crates/
│   ├── grel-cli/               # Clap definitions, operation router, prompts
│   ├── grel-core/              # Asset resolution, tokenization, manifests, deps
│   ├── grel-providers/         # Git forge adapters (GitHub, GitLab, Gitea, Codeberg)
│   ├── grel-network/           # HTTP client, proxy, DNS cache, downloads, extraction
│   ├── grel-cache/             # SQLite schema, migrations, CRUD
│   ├── grel-config/            # Figment-based TOML + env config loading
│   └── grel-elf/               # Linux ELF parsing & system dependency resolution
├── tests/                      # Integration tests
└── docs/                       # Documentation
```

| Crate | Responsibility |
|-------|----------------|
| `grel-cli` | Clap argument definitions (`Cli`), `Operation` enum, interactive prompt loops |
| `grel-core` | `PackageRef` parsing, `AssetTokens` extraction, deterministic resolver, `Manifest`, dependency graph |
| `grel-providers` | `ReleaseProvider` trait, GitHub/GitLab/Gitea/Codeberg modules, registry client |
| `grel-network` | `reqwest` client builder, proxy routing, parallel downloads, archive extraction (zip/tar) |
| `grel-cache` | `state.sqlite` management, package CRUD, ETag/DNS/system-dep caches |
| `grel-config` | TOML loading, env var merging (`GREL_*`), schema validation, path resolution |
| `grel-elf` | `goblin`-based ELF `DT_NEEDED` parsing, distro detection, package-manager command building |

---

## Data Flows

### Install (`-S`)

```
CLI (-S foo/bar)
  → main.rs dispatches to commands::sync::cmd_sync
    → grel-core: PackageRef::parse("foo/bar")
    → grel-providers: ReleaseProvider::latest_release("foo/bar")
    → grel-core: resolve_assets() — strict filter → sort → policy fallback
    → grel-network: download asset + compute SHA256
    → grel-network: extract archive (zip/tar.gz/tar.xz) + link binaries
    → grel-cache: upsert package record + installed_binaries
    → [Linux] grel-elf: resolve_elf_deps() → suggest system packages
```

### Upgrade (`-Syu`)

```
CLI (-Syu)
  → main.rs dispatches to commands::sync::cmd_upgrade
    → grel-cache: list installed packages
    → grel-providers: check remote latest (respecting check_interval_hours)
    → grel-core: diff versions, build upgrade plan
    → grel-network: parallel download to temp `.part` files
    → grel-network: atomic rename + extract + link
    → grel-cache: update DB records (version, asset_filename, checksum)
    → stale binary cleanup
```

### Query (`-Q`)

```
CLI (-Q / -Qi / -Ql / -Qo)
  → main.rs dispatches to commands::query
    → grel-cache: SQLite read-only queries
    → package_files index lookups (with filesystem fallback for unindexed packages)
```

---

## Asset Resolution Pipeline

`grel` uses a **deterministic, non-scoring** pipeline:

1. **Strict Filtering**
   - OS/Arch match (or `Unknown`)
   - `exclude_keywords` rejection (unless `--allow-keyword`)
   - Arch priority + 32-bit fallback rules
   - `ignore_formats` rejection (unless `--allow-format`)

2. **Deterministic Sorting**
   - `prefer_formats` priority
   - Lexicographic filename
   - Size descending

3. **Policy Fallback**
   - Only if `>1` asset remains tied after sorting
   - `first` → top of sorted list
   - `largest` → biggest `size_bytes`

4. **Selection & Warning**
   - `0` → error (no compatible assets)
   - `1` → auto-select (warn if unmanaged)
   - `>1` → interactive numbered prompt

---

## State Database Schema (`state.sqlite`)

Located in the platform-specific data directory (e.g., `~/.local/share/grel/state.sqlite`).

### `installed`

```sql
CREATE TABLE installed (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    forge TEXT NOT NULL,
    owner TEXT NOT NULL,
    repo TEXT NOT NULL,
    version TEXT NOT NULL,
    asset_filename TEXT NOT NULL,
    checksum TEXT,
    install_path TEXT NOT NULL,
    installed_binaries TEXT NOT NULL DEFAULT '',
    is_managed BOOLEAN NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active',
    orphaned_at INTEGER,
    last_checked INTEGER,
    installed_at INTEGER DEFAULT (strftime('%s', 'now')),
    manifest_source TEXT NOT NULL DEFAULT 'heuristic',
    is_explicit BOOLEAN NOT NULL DEFAULT 1
);
```

- `status`: `active`, `orphaned`, or `migrated`
- `is_managed`: `true` = extracted/linked to `bin_dir/`; `false` = left in `download_dir/`
- `is_explicit`: `true` = manually installed; `false` = installed as a dependency
- `manifest_source`: `registry`, `in_repo`, or `heuristic`

### `dependencies`

```sql
CREATE TABLE dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    dep_target TEXT NOT NULL,         -- "forge/owner/repo" or "system:libname"
    dep_type TEXT NOT NULL DEFAULT 'grel',
    FOREIGN KEY (package_id) REFERENCES installed(id) ON DELETE CASCADE
);
```

### `etag_cache`

```sql
CREATE TABLE etag_cache (
    url TEXT PRIMARY KEY,
    etag TEXT NOT NULL,
    last_modified INTEGER NOT NULL
);
```

> **Note:** The `etag_cache` table exists but is not yet used for `If-None-Match` optimization.

### `dns_cache`

```sql
CREATE TABLE dns_cache (
    hostname TEXT NOT NULL,
    ip_address TEXT NOT NULL,
    rtt_ms INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (hostname, ip_address)
);
```

> **Note:** DNS caching exists but RTT probing and forced routing are not yet fully wired.

### `system_dep_cache`

```sql
CREATE TABLE system_dep_cache (
    library_name TEXT NOT NULL,
    distro_id TEXT NOT NULL,
    package_name TEXT NOT NULL,
    discovered_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (library_name, distro_id)
);
```

Caches `lib_name → package_name` mappings discovered by `grel-elf` to avoid slow package-manager queries.

### `package_files`

```sql
CREATE TABLE package_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    file_type TEXT NOT NULL DEFAULT 'data',
    FOREIGN KEY (package_id) REFERENCES installed(id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX idx_pkg_file_unique ON package_files(package_id, file_path);
CREATE INDEX idx_pkg_file_path ON package_files(file_path);
```

Tracks every file extracted by a managed package. Used for fast `-Ql`, `-Fl`, `-Fs`, `-Qo` lookups and conflict detection during install. Rebuilt by `-Fy`.

---

## Manifest Resolution (3-Tier)

When installing a package, `grel` tries three sources for metadata:

1. **Registry** — Central registry cache (git repo of TOML files)
2. **In-repo** — `.grel.toml` fetched from the repo's default branch
3. **Heuristic** — Filename tokenization (`AssetTokens::from_filename`)

A manifest can declare:
- Exact asset patterns per platform
- Grel dependencies (`dependencies.grel`)
- Optional dependencies
- Hook scripts (`post_install`, `pre_remove`) — executed when `enable_hooks = true` (disabled by default)

---

## Dependency Graph

`grel-core` maintains an in-memory dependency graph:

- **Nodes:** installed packages
- **Edges:** `dependencies` table entries where `dep_type = 'grel'`

Used for:
- `-Rc` (cascade removal): remove orphaned implicit packages after target removal
- `-Qt` (unrequired): list packages with zero reverse dependencies
- `-Ru` (remove unneeded): topological sort and remove implicit leaf packages

Cycle detection is performed during graph construction.

---

## Platform Enums

```rust
pub enum Os { Linux, Windows, MacOS, FreeBSD, Android, iOS, Unknown(String) }
pub enum Arch { X86_64, Aarch64, I686, ArmV7, ArmV6, Riscv64, S390x, PowerPC64, Unknown(String) }
```

Parsing uses an exhaustive alias table (`x86_64` → `Arch::X86_64`, `amd64` → `Arch::X86_64`, etc.).

---

## Build & Test

```bash
# Build
cargo build
cargo build --release

# Test
cargo test --workspace

# Lint
cargo clippy --workspace
cargo fmt --all -- --check
```

---

## Key Design Decisions

- **User-space only:** No `sudo`, no system directories. Everything lives under `~/.local/share/grel` (or platform equivalent).
- **Atomic installs:** Downloads go to temp dirs → verify → rename → DB update.
- **Pure CLI:** No TUI. Standard `std::io` prompts with TTY detection for CI auto-accept.
- **Deterministic resolution:** No fuzzy scoring. Strict filters → explicit sort → policy fallback.
- **Cross-platform:** Linux and Windows are first-class. macOS is supported but optional.
