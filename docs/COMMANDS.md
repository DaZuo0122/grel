# Command Reference

This document describes the actual CLI surface of `grel` as of the latest codebase. For design history, see `COMMANDS_DESIGN.md`.

**Syntax:**
```
grel <OPERATION> [OPTIONS] [TARGETS...]
```

Operations are mutually exclusive; the first one wins. Flags can be stacked (e.g., `-Syu` = `-S -y -u`).

Use `grel -Sh`, `grel -Qh`, etc. for operation-scoped help.

---

## Global Options

These are available with any operation:

| Flag | Description |
|------|-------------|
| `-C, --config <PATH>` | Load a specific TOML config file |
| `-f, --forge <FORGE>` | Default forge for 2-part refs (`github`, `gitlab`, `gitea`, `codeberg`) |
| `--proxy <URL>` | HTTP proxy override |
| `--verify-signatures` / `--no-verify-signatures` | Toggle signature sidecar checking |
| `--registry <URL>` / `--no-registry` | Toggle central registry lookup |
| `--auto-resolve-deps` / `--no-auto-resolve-deps` | Toggle ELF system-dependency resolution (Linux) |
| `--show-parsed-deps` / `--no-show-parsed-deps` | Toggle printing discovered ELF dependencies |
| `--enable-hooks` / `--no-enable-hooks` | Allow / suppress manifest install/removal hooks |
| `--exclude-keywords <K1,K2>` | Temporarily add keywords to the exclusion list |
| `--dry-run` | Simulate without writing files |
| `--noconfirm` | Skip all interactive prompts |
| `-h, --help` | Show help (global or operation-scoped) |

---

## `-S, --sync` — Fetch & Install from Forges

**Usage:** `grel -S [OPTIONS] [TARGETS...]`

| Flag | Description |
|------|-------------|
| `-s, --search <PATTERN>` | Search forges for packages |
| `-y, --refresh` | Refresh forge metadata and caches |
| `-u, --sysupgrade` | Upgrade all installed packages |
| `-i, --info <PKG>` | Show remote release metadata before installing |
| `-c, --clean` | Purge downloaded artifacts from cache |
| `-w, --download-only` | Download archive only, skip extraction |
| `--overwrite` | Replace existing binaries if they conflict |
| `--needed` | Skip install if package is already up-to-date |
| `--asset <NAME>` | Download exact asset filename (bypass auto-selection) |
| `--platform <OS/ARCH>` | Override host platform detection (e.g., `linux/x86_64`) |
| `--allow-format <FMT>` | Temporarily allow a normally ignored format |
| `--allow-keyword` | Allow keyword-matching assets |
| `--asdeps` | Install packages as dependencies (`is_explicit = false`) |
| `--asexplicit` | Install packages as explicitly installed (default) |

**Examples:**
```bash
grel -S foo/bar              # Install latest release
grel -S github/foo/bar       # Explicit forge
grel -S foo/bar@v1.2.3       # Pin version
grel -Ss ripgrep             # Search for ripgrep
grel -Sy                     # Refresh metadata only
grel -Su                     # Upgrade installed (cached metadata)
grel -Syu                    # Refresh + upgrade
grel -S --asdeps foo/bar     # Install as dependency
grel -S --dry-run foo/bar    # Simulate install
```

**Notes:**
- Package references can be `owner/repo`, `forge/owner/repo`, or `owner/repo@version`.
- The default forge is `github` unless overridden by `-f`.
- `-Sc` cleans `download_dir` and removes stale archives under `install_root` (keeps the current `asset_filename`).
- `--needed` compares the installed DB version against the remote release tag; skips if they match.
- `-Sw` sets `is_managed = false`; the archive is left in `download_dir`.
- Manifest `post_install` hooks run after extraction only when `enable_hooks = true` (disabled by default).

---

## `-Q, --query` — Inspect Local State

**Usage:** `grel -Q [OPTIONS] [TARGETS...]`

| Flag | Description |
|------|-------------|
| `-l, --list [<PKG>]` | List files owned by a package (no arg = all packages) |
| `-i, --info <PKG>` | Show detailed info of an installed package |
| `-o, --owns <PATH>` | Find which package owns a file/binary |
| `-q, --quiet` | Output minimal data (package names only) |
| `-e, --explicit` | Filter to manually installed packages |
| `-d, --deps-filter` | Filter to packages installed as dependencies |
| `-t, --unrequired` | List packages not required by any other (dependency orphans) |
| `-k, --check` | Verify SHA256 checksums of installed archives |
| `-s, --search <PATTERN>` | Search locally installed packages by name |

**Examples:**
```bash
grel -Q                      # List all installed packages
grel -Ql                     # List files for all packages
grel -Ql foo/bar             # List files owned by foo/bar
grel -Qi foo/bar             # Show local install info
grel -Qo rg                  # Which package provides rg?
grel -Qeq                    # Explicitly installed, quiet mode
grel -Qd                     # Dependency-installed packages
grel -Qt                     # Orphan packages
grel -Qk                     # Verify checksums
grel -Qs ripgrep             # Search installed packages
```

**Notes:**
- `-Qo` uses the `package_files` DB index first (exact path, canonical path, filename, stem match), with a filesystem walk fallback for unindexed packages.
- `-Ql` uses the `package_files` DB index first, with a filesystem walk fallback for unindexed packages.
- `-Qk` verifies the **downloaded archive** checksum, not individual extracted files.
- `--orphans` is deprecated; use `-t` instead.

---

## `-R, --remove` — Uninstall

**Usage:** `grel -R [OPTIONS] [TARGETS...]`

| Flag | Description |
|------|-------------|
| `-c, --clean` | Cascade: remove package + unneeded dependencies |
| `-n, --nosave` | Do not preserve config file backups (`.grelnew`) |
| `-r, --recursive` | Remove packages that depend on the target first |
| `-u, --sysupgrade` | Remove packages that are no longer required |
| `--noconfirm` | Skip removal confirmation |
| `--dry-run` | Show what would be removed |

**Examples:**
```bash
grel -R foo/bar              # Uninstall
grel -Rc foo/bar             # Cascade removal
grel -Rr foo/bar             # Recursive removal
grel -Ru                     # Remove all unneeded packages
grel -Rcn foo/bar            # Cascade + nosave
```

**Notes:**
- `--nosave` skips preserving files that look like configs (by extension). Config preservation is heuristic-based, not metadata-based.
- `pre_remove` hooks from the saved `.grel.toml` manifest run before deletion when `enable_hooks = true`.

---

## `-D, --database` — Local DB & State Management

**Usage:** `grel -D [OPTIONS] [TARGETS...]`

| Flag | Description |
|------|-------------|
| `--asexplicit` | Mark target(s) as explicitly installed |
| `--asdeps` | Mark target(s) as dependencies |
| `--migrate <OLD> <NEW>` | Update `owner/repo` path for renamed projects |
| `--clean` | Prune orphaned records and stale ETag/DNS caches |
| `--check` | Verify SQLite DB integrity (`PRAGMA integrity_check`) |
| `--dump` | Export state as JSON to stdout |

**Examples:**
```bash
grel -D --clean
grel -D --check
grel -D --asexplicit foo/bar
grel -D --asdeps foo/bar
grel -D --migrate old-org/tool new-org/tool
```

**Notes:**
- `--clean` removes orphaned DB records and clears stale ETag/DNS cache entries.
- `--check` runs SQLite integrity checks and reports package counts.

---

## `-U, --upgrade` — Install from Local File

**Usage:** `grel -U [OPTIONS] [TARGETS...]`

| Flag | Description |
|------|-------------|
| `--overwrite` | Replace existing binaries if they conflict |
| `--noconfirm` | Skip prompts & warnings |
| `--asset <PATH>` | Treat file as direct download (bypass extraction) |

**Examples:**
```bash
grel -U ./ripgrep-14.1.1-x86_64-linux.tar.gz
grel -U ./tool.exe --noconfirm
grel -U ./mytool --asset ./mytool    # Direct copy, no extraction
```

**Notes:**
- Local install computes SHA256, extracts/archives, links binaries, and upserts the DB.
- `--asset` is useful for plain binaries that should not be treated as archives.

---

## `-F, --files` — File Index & Binary Search

**Usage:** `grel -F [OPTIONS] [TARGETS...]`

| Flag | Description |
|------|-------------|
| `-s, --search <PATTERN>` | Search installed packages for a filename/binary |
| `-l, --list <PKG>` | List all files extracted by a package |
| `-y, --refresh` | Rebuild file index by walking all packages |
| `-q, --quiet` | Output only matching paths |

**Examples:**
```bash
grel -Fs rg                  # Find which package provides rg
grel -Fl foo/bar             # List all files from foo/bar
grel -Fy                     # Reindex all packages
```

**Notes:**
- `-Fs` and `-Fl` query the persistent `package_files` DB index first, with a filesystem fallback for unindexed packages.
- `-Fy` rebuilds the index by walking all managed packages and persisting the file list to `package_files`.
- `-Fl` without a package argument will report an error (do not run bare `grel -Fl`).

---

## Option Combining & Precedence

1. **Stacking:** `-Syu` is equivalent to `-S -y -u`.
2. **Operation First:** The first `-S/-Q/-R/-D/-U/-F` determines the operation.
3. **Override Hierarchy:** CLI flags > Environment variables (`GREL_*`) > compiled defaults.
4. **Conflicts:** `--dry-run` + `--noconfirm` → `--dry-run` wins (no prompts, no writes).

---

## Internal Routing

| Operation | Routes To |
|-----------|-----------|
| `-S` | `grel-cli` → `grel-core/resolver` → `grel-network/download` → `grel-cache/install` |
| `-Syu` | `grel-core/upgrade` → parallel download → atomic replace |
| `-Q/-F` | `grel-cache/sqlite` (read-only queries, `package_files` index, filesystem fallback) |
| `-R/-D` | `grel-cache/state` (DB updates, orphan marking, migration, cleanup) |
| `-U` | `grel-network/archive` → `grel-cache/install` |
