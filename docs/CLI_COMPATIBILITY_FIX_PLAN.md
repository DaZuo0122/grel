> **Note:** This is a historical fix plan from 2026-05-09. Many phases described here have been completed. For current status, see [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

# CLI Compatibility Fix Plan

> **Date:** 2026-05-09  
> **Goal:** Resolve all critical pacman CLI compatibility gaps identified in `PACMAN_COMPATIBILITY_AUDIT.md`

---

## Phase 1: Critical `-Q` Fixes

### 1.1 Fix `-Q -l` (list files owned by package)
**Root cause:** `commands.rs` defines `list: Option<String>` with `action = clap::ArgAction::SetTrue`, creating a clap contradiction that ignores the package argument.

**Fix:**
- Redefine `-l` for `-Q` as a separate flag that takes an optional package name: `list_pkg: Option<String>` with `action = ArgAction::Set` (not `SetTrue`)
- Keep the existing `list: bool` with `SetTrue` for the "list all" behavior
- In `main.rs` Query routing:
  - `grel -Q` → `cmd_list()` (list packages)
  - `grel -Ql` → `cmd_list_files(None)` (list files for all packages)
  - `grel -Ql foo/bar` → `cmd_list_files(Some("foo/bar"))` (list files for specific package)
- Add `cmd_list_files()` that reads all tracked files from the package's install directory and DB

**Files:** `crates/grel-cli/src/commands.rs`, `src/main.rs`

### 1.2 Add `-Q -s` (local search)
**Fix:**
- Add `local_search: Option<String>` to `Cli` with `short = 's'` under Query context
- In `main.rs`, route `-Qs <pattern>` to a new `cmd_local_search()`
- `cmd_local_search()` queries DB `list_packages()` and filters by name/description match

**Note:** `-s` currently only exists for `-S`. Need to handle the namespace carefully — the flag is context-dependent by operation, so clap should allow both.

**Files:** `crates/grel-cli/src/commands.rs`, `src/main.rs`

### 1.3 Add `-Q -d` (dependency filter)
**Fix:**
- Add `deps_filter: bool` to `Cli` with `short = 'd'`
- Modify `cmd_list()` to accept a `deps: bool` parameter
- When `deps = true`, filter to `is_explicit = false`

**Files:** `crates/grel-cli/src/commands.rs`, `src/main.rs`

### 1.4 Add `-Q -t` (orphans / unrequired)
**Fix:**
- Add `unrequired: bool` to `Cli` with `short = 't'`
- Route `-Qt` to `cmd_list_orphans()` (same as `--orphans`)
- Update help text to mention `-t` as the short form

**Files:** `crates/grel-cli/src/commands.rs`, `src/main.rs`

---

## Phase 2: `-S` Workflow Fixes

### 2.1 Add `-S --asdeps` / `-S --asexplicit`
**Fix:**
- Add `--asdeps` and `--asexplicit` as boolean flags to `Cli` (not just under `-D`)
- In `cmd_sync`, after installing each package, set `is_explicit` based on the flag:
  - `--asdeps` → `is_explicit = false`
  - `--asexplicit` → `is_explicit = true` (default)
- Mutually exclusive: error if both are passed

**Files:** `crates/grel-cli/src/commands.rs`, `src/main.rs`

### 2.2 Add `-S -w` (download only)
**Fix:**
- Add `download_only: bool` to `Cli` with `short = 'w'`
- In `cmd_sync`, if `download_only = true`, download the asset to `download_dir/` but do not extract or update `bin_dir/`
- Still record in DB with `is_managed = false`

**Files:** `crates/grel-cli/src/commands.rs`, `src/main.rs`

---

## Phase 3: `-R` Fixes

### 3.1 Add `-R -u` (remove unneeded)
**Fix:**
- Add `unneeded: bool` to `Cli` with `short = 'u'` under Remove
- In `cmd_remove`, if `unneeded = true` (and no targets specified), find all packages where:
  - `is_explicit = false`
  - No other package depends on them (`get_dependents` returns empty)
  - Status is `active`
- Remove them in dependency-order (reverse topological sort)
- Prompt for confirmation unless `--noconfirm`

**Files:** `crates/grel-cli/src/commands.rs`, `src/main.rs`

### 3.2 Wire `-R -n` (nosave)
**Fix:**
- Currently stubbed. Implement by skipping the "preserve config" logic.
- Since grel doesn't have a config backup system yet, the implementation is minimal:
  - If `nosave = true`, do not retain any extracted files that might be configs
  - Document that full config preservation requires future `.grelnew` implementation

**Files:** `src/main.rs`

---

## Phase 4: `-U` Fix

### 4.1 Implement `-U <file>` (local file install)
**Fix:**
- `cmd_upgrade_local` is currently a stub
- Implement the full flow:
  1. Validate file exists
  2. Compute SHA256
  3. Extract archive (if managed format) or copy (if plain binary)
  4. Link binaries to `bin_dir/`
  5. Update DB with `is_managed = true`
- Support `--asset` override to treat file as direct download (no extraction)

**Files:** `src/main.rs`

---

## Phase 5: `-F` Fixes

### 5.1 Add `-F -s` short flag
**Fix:**
- Add `file_search_short: Option<String>` with `short = 's'` that maps to the same behavior as `--file-search`
- Or rename the existing `--file-search` to use `short = 's'` directly

**Files:** `crates/grel-cli/src/commands.rs`, `src/main.rs`

### 5.2 Implement `-F -l` (list files)
**Fix:**
- `cmd_file_list` is a stub
- Implement by walking the package's `install_path` directory and listing all files
- If the package is managed, walk `install_path/extracted/`
- Store results in a new `package_files` DB table for faster future lookups

**Note:** This requires file tracking infrastructure (see Phase 6).

**Files:** `src/main.rs`, `grel-cache/src/database.rs`

### 5.3 Implement `-F -y` (reindex)
**Fix:**
- `cmd_reindex` is a stub
- Walk all managed packages' `install_path/extracted/` directories
- Record all files in `package_files` table
- Clear and rebuild the index

**Files:** `src/main.rs`, `grel-cache/src/database.rs`

---

## Phase 6: File Tracking Infrastructure

To support `-Ql`, `-Fl`, `-Fs`, `-Fy` properly, grel needs to track all extracted files, not just binaries.

### DB Schema Addition
```sql
CREATE TABLE package_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    FOREIGN KEY (package_id) REFERENCES installed(id) ON DELETE CASCADE
);
CREATE INDEX idx_pkg_files_package ON package_files(package_id);
CREATE INDEX idx_pkg_files_path ON package_files(file_path);
```

### Changes
- During `install_asset()`, after extraction, walk the extracted directory and record all files (not just binaries) in `package_files`
- During `remove_package_files()`, delete the DB records
- Add `db.get_package_files(package_id)` method

**Files:** `grel-cache/src/database.rs`, `grel-cache/src/models.rs`, `grel-network/src/archive.rs`, `src/main.rs`

---

## Phase 7: Stubs & Polish

### 7.1 Wire `-S --overwrite`
- During install, if a binary already exists in `bin_dir/` and checksum differs, replace it instead of erroring

### 7.2 Wire `-S --asset`, `-S --platform`, `-S --allow-format`
- Route these flags into the asset resolution logic

### 7.3 Wire `-Sc` (cache cleanup)
- Remove old archives from `install_root` based on retention policy

---

## Implementation Order

1. **Phase 1** (Critical `-Q` fixes) — highest impact on muscle memory
2. **Phase 2** (`-S` workflow fixes) — common operations
3. **Phase 3** (`-R` fixes) — cleanup operations
4. **Phase 6** (File tracking) — enables Phase 5
5. **Phase 5** (`-F` fixes) — depends on Phase 6
6. **Phase 4** (`-U` fix) — standalone
7. **Phase 7** (Stubs) — polish

---

## Estimated Effort

| Phase | Complexity | Estimated Time |
|-------|-----------|----------------|
| Phase 1 | Medium | ~2 hours |
| Phase 2 | Low | ~1 hour |
| Phase 3 | Medium | ~1.5 hours |
| Phase 4 | Medium | ~2 hours |
| Phase 5 | High | ~3 hours |
| Phase 6 | High | ~2 hours |
| Phase 7 | Low | ~1 hour |
