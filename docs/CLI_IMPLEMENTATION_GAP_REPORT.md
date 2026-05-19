> **Note:** This is a historical gap report from 2026-05-09. Many gaps listed here have since been closed. For current status, see [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

# CLI Implementation Gap Report

> Generated: 2026-05-09  
> Scope: All CLI flags, their routing in `src/main.rs`, and their actual implementation in `src/commands/`

---

## Executive Summary

| Category | Count |
|----------|-------|
| Fully implemented | 31 |
| Partially implemented | 7 |
| Stub (accepted by CLI, no logic) | 3 |
| Broken (has logic but fails) | 1 |
| Missing (not in CLI) | 2 |
| **Total flags reviewed** | **44** |

The core install/remove/query pipeline is functional. The biggest architectural gap is the **absence of a persistent file index** (`package_files` DB table), which limits `-Ql`, `-Fl`, `-Fy`, and `-Qo` to slow, potentially inaccurate filesystem walks. There is also **one panic path** (`-Fl` without an argument).

---

## 1. How to Read This Report

Each entry follows this classification:

- **✅ Fully implemented** — Working logic exists and is wired end-to-end.
- **⚠️ Partially implemented** — Has working logic but missing pieces (e.g., only handles one of two cleanup locations).
- **🚧 Stub** — The CLI accepts the flag and passes it into the program, but no handler code reads or acts on it.
- **❌ Broken** — Code exists but has a bug that causes incorrect behavior or a crash.
- **⛔ Missing** — The flag is not declared in `commands.rs` at all.

---

## 2. Master Flag Matrix

### Sync (`-S`)

| # | Flag | Classification | Evidence |
|---|------|----------------|----------|
| 1 | `-S <pkg>...` | ✅ Fully implemented | `cmd_sync`: full pipeline — manifest resolution → asset resolution → download → extract → DB upsert → dep install → ELF check |
| 2 | `-Ss <pattern>` | ✅ Fully implemented | `cmd_search`: registry + forge provider search |
| 3 | `-Sy` | ✅ Fully implemented | `cmd_sync_refresh`: checks all packages against remote, marks orphaned |
| 4 | `-Su` / `-Syu` | ✅ Fully implemented | `cmd_upgrade`: full upgrade loop with atomic replace |
| 5 | `-Si <pkg>` | ✅ Fully implemented | `cmd_info_remote`: fetches latest release from forge |
| 6 | `-Sc` | ⚠️ Partially implemented | `cmd_clean_cache`: only cleans `download_dir`; does **not** clean old archives under `install_root` |
| 7 | `-Sw` | ⛔ Missing | Not declared in `commands.rs` |
| 8 | `-S --asdeps` | ✅ Fully implemented | Wired in `cmd_sync`: sets `is_explicit = false` before DB upsert |
| 9 | `-S --asexplicit` | ✅ Fully implemented | Wired in `cmd_sync`: sets `is_explicit = true` before DB upsert |
| 10 | `-S --needed` | ⛔ Missing | Not declared in `commands.rs` |
| 11 | `-S --overwrite` | 🚧 Stub | Declared in CLI (`overwrite: bool`), but **never read** in `cmd_sync` or `cmd_upgrade` |
| 12 | `-S --asset <name>` | ✅ Fully implemented | Overrides asset auto-selection in resolver pipeline |
| 13 | `-S --platform <os/arch>` | ✅ Fully implemented | Overrides host OS/arch detection in resolver |
| 14 | `-S --allow-format <fmt>` | 🚧 Stub | Declared in CLI (`allow_format: Option<String>`), but **never passed** to the resolver |
| 15 | `-S --exclude-keywords <kw>...` | ✅ Fully implemented | Passed to resolver; blocks keyword-matched assets |
| 16 | `-S --allow-keyword` | ✅ Fully implemented | Bypasses `exclude_keywords` filter in resolver |
| 17 | `-S --dry-run` | ✅ Fully implemented | Respected by download, extract, and DB write paths |
| 18 | `-S --noconfirm` | ✅ Fully implemented | Auto-accepts interactive prompts |

### Query (`-Q`)

| # | Flag | Classification | Evidence |
|---|------|----------------|----------|
| 19 | `-Q` | ✅ Fully implemented | `cmd_list`: lists all installed packages with status/color |
| 20 | `-Qq` | ✅ Fully implemented | `quiet` mode: prints only package refs |
| 21 | `-Qi <pkg>` | ✅ Fully implemented | `cmd_info_local`: version, status, managed, explicit, asset, path, checksum, **dependencies** |
| 22 | `-Qo <path>` | ⚠️ Partially implemented | `cmd_owns`: only does substring match on `install_path` and `asset_filename`. Does **not** walk the file tree or query a file index, so many legitimate ownership queries return "No package owns" |
| 23 | `-Qe` / `-Qeq` | ✅ Fully implemented | Filters list to `is_explicit = true` packages |
| 24 | `-Qd` / `-Qdq` | ✅ Fully implemented | Filters list to `is_explicit = false` packages |
| 25 | `-Ql [pkg]` | ⚠️ Partially implemented | `cmd_list_files`: walks `install_path` on the filesystem. No persistent file index, so it's slow and may be inaccurate if files were moved after install. `-Ql` without arg works (lists all packages). `-Ql foo/bar` works. |
| 26 | `-Qs <pattern>` | ✅ Fully implemented | `cmd_local_search`: substring match on package ref/owner/repo |
| 27 | `-Qt` / `--orphans` | ✅ Fully implemented | `cmd_list_orphans`: filters by `PackageStatus::Orphaned` |
| 28 | `-Qk` | ✅ Fully implemented | `cmd_verify_checksums`: computes SHA256 of stored archive and compares to DB. Note: pacman verifies *installed files*; grel verifies the *download archive*. |

### Remove (`-R`)

| # | Flag | Classification | Evidence |
|---|------|----------------|----------|
| 29 | `-R <pkg>...` | ✅ Fully implemented | `cmd_remove`: parses ref, checks dependents, removes files + DB record. Supports `--dry-run`, `--noconfirm`. |
| 30 | `-Rc` | ✅ Fully implemented | `clean` flag: after removing the target, rebuilds the dependency graph and removes orphaned implicit packages |
| 31 | `-Rs` / `--recursive` | ✅ Fully implemented | `recursive` flag: removes dependent packages first before removing the target |
| 32 | `-Rns` | ⚠️ Partially implemented | `-n` / `--nosave` is wired through the call chain but is a **no-op** in `remove_package_files`. Code comment: "nosave is currently a no-op because grel does not yet implement config file preservation (.grelnew)" |
| 33 | `-Ru` | ✅ Fully implemented | `cmd_remove_unneeded`: finds implicit packages with zero dependents, topologically sorts, prompts, removes |
| 34 | `-R --dry-run` | ✅ Fully implemented | Prints "would remove" messages without deleting files |
| 35 | `-R --noconfirm` | ✅ Fully implemented | Auto-accepts removal confirmation |

### Database (`-D`)

| # | Flag | Classification | Evidence |
|---|------|----------------|----------|
| 36 | `-D --asexplicit <pkg>...` | ✅ Fully implemented | `cmd_db_as_explicit`: updates `is_explicit = true` in DB |
| 37 | `-D --asdeps <pkg>...` | ✅ Fully implemented | `cmd_db_as_deps`: updates `is_explicit = false` in DB |
| 38 | `-D --migrate <OLD> <NEW>` | ✅ Fully implemented | `cmd_migrate`: removes old record, inserts new with copied fields |
| 39 | `-D --clean` | ⚠️ Partially implemented | `cmd_db_clean`: removes orphaned DB records. Prints "ETag/IP cache cleanup not yet implemented" — actual cache cleanup is a cosmetic stub |
| 40 | `-D --check` | ✅ Fully implemented | `cmd_db_check`: counts package records, prints "healthy" |
| 41 | `-D --dump` | ✅ Fully implemented | `cmd_db_dump`: prints JSON array of packages to stdout |

### Upgrade (`-U`)

| # | Flag | Classification | Evidence |
|---|------|----------------|----------|
| 42 | `-U <file>` | ✅ Fully implemented | `cmd_upgrade_local`: validates file, computes SHA256, copies to install dir, extracts, links binaries, handles reinstall (cleans old dir + binaries), upserts DB. Supports `--dry-run`, `--noconfirm`, `--local-asset`. |

### Files (`-F`)

| # | Flag | Classification | Evidence |
|---|------|----------------|----------|
| 43 | `-Fs <pattern>` | ✅ Fully implemented | `cmd_file_search`: walks every package's `install_path` looking for filename substring match. No DB index, so O(n) filesystem walk. |
| 44 | `-Fl <pkg>` | ✅ Fully implemented | `cmd_file_list`: walks specific package's `install_path`. |
| 45 | `-Fl` (no pkg arg) | ❌ Broken | `src/main.rs:141` uses `.unwrap()` on `cli.list.first().or_else(|| cli.targets.first())`. If neither is set, the program **panics** at runtime. |
| 46 | `-Fy` | ⚠️ Partially implemented | `cmd_reindex`: walks all packages, counts files, prints summary. Does **not** persist to a `package_files` DB table (no persistent index is built). |

### Global / Misc

| # | Flag | Classification | Evidence |
|---|------|----------------|----------|
| 47 | `--dry-run` | ✅ Fully implemented | Respected by `-S`, `-R`, `-U` handlers |
| 48 | `--noconfirm` | ✅ Fully implemented | Respected globally via `CommandContext` |
| 49 | `--config <path>` | ✅ Fully implemented | Loaded before dispatch |
| 50 | `-f / --forge <forge>` | ✅ Fully implemented | Default forge for 2-part package refs |
| 51 | `--proxy <url>` | ✅ Fully implemented | Passed to HTTP client builder |
| 52 | `--verify-signatures` / `--no-verify-signatures` | ✅ Fully implemented | Overrides `config.security.verify_signatures` |
| 53 | `--registry <url>` / `--no-registry` | ✅ Fully implemented | Overrides registry URL or disables registry lookup |
| 54 | `--auto-resolve-deps` / `--no-auto-resolve-deps` | ✅ Fully implemented | Overrides `config.elf_deps.auto_resolve_system_deps` |
| 55 | `--show-parsed-deps` / `--no-show-parsed-deps` | ✅ Fully implemented | Overrides `config.elf_deps.show_parsed_deps` |

---

## 3. Detailed Gap Analysis

### Gap 1: `-Fl` without argument panics (❌)

**Location:** `src/main.rs:141`

```rust
let pkg = cli.list.first().or_else(|| cli.targets.first()).unwrap();
```

**Impact:** Medium — crashes the program instead of printing a helpful error.

**Fix:** Replace `.unwrap()` with a proper error message:
```rust
let Some(pkg) = cli.list.first().or_else(|| cli.targets.first()) else {
    eprintln!("Usage: grel -Fl <package>");
    return Ok(());
};
```

---

### Gap 2: No persistent file index (⚠️ affects `-Ql`, `-Fl`, `-Fy`, `-Qo`)

**Root cause:** There is no `package_files` table in the database. Every file-related operation performs a live filesystem walk of `install_path`.

**Affected commands:**
- `-Ql [pkg]` — walks `install_path` recursively. Slow for large packages. Misses files if the user moved them.
- `-Fl <pkg>` — same limitation.
- `-Fy` — walks all packages, counts files, prints summary. The count is ephemeral; nothing is persisted.
- `-Qo <path>` — only does substring matching on `install_path` and `asset_filename`. It cannot tell you which package owns `/home/user/.local/share/grel/bin/my-tool` because it doesn't know the actual extracted file paths.

**What pacman does:** `pacman -Qo` and `pacman -Ql` use the `package_files` table (or equivalent mtree data) for O(1) lookups.

**Fix complexity:** Medium. Requires:
1. New `package_files` table: `(package_id, file_path PRIMARY KEY)`
2. During extraction in `sync.rs`, record every extracted file path
3. Update `cmd_reindex` to populate the table
4. Update `cmd_owns`, `cmd_list_files`, `cmd_file_list`, `cmd_file_search` to query the table

---

### Gap 3: `--overwrite` is a stub (🚧)

**Location:** `crates/grel-cli/src/commands.rs` declares `overwrite: bool`, but it is **never read** in any handler.

**Expected behavior:** When installing a package whose binaries already exist in `bin_dir/`, `--overwrite` should replace them without error. Currently, the link/copy fallback chain in `grel-network/src/archive.rs` may silently succeed or fail depending on OS behavior.

**Fix complexity:** Low. Add an overwrite check in the binary linking logic.

---

### Gap 4: `--allow-format` is a stub (🚧)

**Location:** `crates/grel-cli/src/commands.rs` declares `allow_format: Option<String>`, but it is **never passed** to the resolver.

**Expected behavior:** Temporarily allow a normally ignored format (e.g., `--allow-format *.deb`) for a single install.

**Fix complexity:** Low. Pass the value into the resolver config as a one-shot override for `ignore_formats`.

---

### Gap 5: `-Sc` only cleans `download_dir` (⚠️)

**Location:** `src/commands/sync.rs` (`cmd_clean_cache`)

**Current behavior:** Removes orphaned files from `download_dir` only.

**Expected behavior:** Should also implement a retention policy for archives under `install_root` (e.g., keep only the current archive, or keep N versions).

**Fix complexity:** Low-Medium. Add `install_root` archive scan + retention rule.

---

### Gap 6: `--nosave` (`-R -n`) is a no-op (⚠️)

**Location:** `src/commands/remove.rs:240-266`

**Current behavior:** The `nosave` parameter is accepted but intentionally ignored. A code comment explains: "grel does not yet implement config file preservation (.grelnew)".

**Expected behavior:** With `--nosave`, remove all files including config. Without it, preserve config files (e.g., files matching `*.conf`, `*.toml` in the install tree).

**Fix complexity:** Medium. Requires defining what counts as a "config file" in the grel context.

---

### Gap 7: `-Sw` (download only) is missing (⛔)

**Location:** Not in `commands.rs`

**Expected behavior:** Download the asset to `download_dir` but do not extract or link.

**Fix complexity:** Low. Add the flag and skip the extract/link phase in `cmd_sync`.

---

### Gap 8: `--needed` is missing (⛔)

**Location:** Not in `commands.rs`

**Expected behavior:** Skip reinstall if the package is already installed at the latest version.

**Fix complexity:** Low. In `cmd_sync`, check the DB before installing.

---

## 4. Command Router Reference

For cross-referencing, here is the exact dispatch logic from `src/main.rs`:

```
Operation::Sync
  ├── -u          → sync::cmd_upgrade
  ├── -s <pat>    → sync::cmd_search
  ├── -i <pkg>    → query::cmd_info_remote
  ├── -y          → sync::cmd_sync_refresh
  ├── -c          → sync::cmd_clean_cache
  └── else        → sync::cmd_sync (targets)

Operation::Query
  ├── -t / --orphans → query::cmd_list_orphans
  ├── -i <pkg>    → query::cmd_info_local
  ├── -o <path>   → query::cmd_owns
  ├── -k          → query::cmd_verify_checksums
  ├── -s <pat>    → query::cmd_local_search
  ├── -l [pkg]    → query::cmd_list_files(pkg?)
  └── else        → query::cmd_list

Operation::Remove
  ├── -u (no targets) → remove::cmd_remove_unneeded
  └── else        → remove::cmd_remove (targets)

Operation::Database
  ├── --db-clean  → database::cmd_db_clean
  ├── --db-check  → database::cmd_db_check
  ├── --db-dump   → database::cmd_db_dump
  ├── --asexplicit→ database::cmd_db_as_explicit
  ├── --asdeps    → database::cmd_db_as_deps
  ├── --migrate   → database::cmd_migrate
  └── else        → error

Operation::Upgrade
  └── (any)       → upgrade::cmd_upgrade_local

Operation::Files
  ├── -s <pat>    → files::cmd_file_search
  ├── -l <pkg>    → files::cmd_file_list   [PANIC if no pkg]
  ├── -y          → files::cmd_reindex
  └── else        → error
```

---

## 5. Stubs and Cosmetic TODOs

| Location | Text | Severity |
|----------|------|----------|
| `src/commands/database.rs:78` | `"ETag/IP cache cleanup not yet implemented"` | Cosmetic — harmless |
| `src/commands/remove.rs:263-265` | `nosave` intentionally ignored with explanatory comment | Functional gap — user expectation mismatch |
| `crates/grel-network/src/dns_cache.rs:46` | `"TODO: Probe IPs for fastest RTT"` | Outside commands scope |

No `unimplemented!()`, `todo!()`, or `panic!()` macros exist in `src/commands/`.

---

## 6. Recommendations (Priority Order)

| Priority | Gap | Effort | Impact |
|----------|-----|--------|--------|
| **P0** | Fix `-Fl` panic (`.unwrap()` → error message) | 5 min | Prevents crashes |
| **P1** | Implement `--overwrite` | 30 min | Common user need |
| **P1** | Implement `--allow-format` | 30 min | Common user need |
| **P1** | Implement `--needed` | 30 min | Avoids redundant reinstalls |
| **P2** | Implement `-Sw` (download only) | 1 hour | Nice-to-have parity |
| **P2** | Extend `-Sc` to `install_root` | 1-2 hours | Completes cache cleanup |
| **P3** | Build `package_files` table + index | 4-6 hours | Enables robust `-Qo`, `-Ql`, `-Fl` |
| **P3** | Implement `--nosave` config preservation | 2-3 hours | Minor user expectation gap |
