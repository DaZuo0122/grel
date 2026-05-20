# grel CLI Codebase Review And Test Report

Date: 2026-05-14

Branch: `codex-test-automation-just`

Scope: review the current CLI application against the documented command surface in `docs/COMMANDS_DESIGN.md`, `docs/TECHNICAL_DESIGN.md`, the current `clap` interface, and the shipped automated tests.

Verdict: `FAIL`

The codebase is not functionally complete relative to its documented CLI contract. Core install, query, remove, and local-file flows exist, and the offline automated tests currently pass, but several advertised flags are unwired, some documented commands do not map to the real CLI, and at least one high-risk filesystem bug can delete the entire unmanaged download directory during uninstall.

## 1. Executed Checks

Commands executed during this review:

```powershell
cargo run -- --help
cargo run -- -D --clean
cargo run -- -D --db-clean
just test-offline
```

Observed results:

- `cargo run -- --help`: succeeded and exposed the current real CLI surface.
- `cargo run -- -D --clean`: printed `No database operation specified. Use -Dh for help.`
- `cargo run -- -D --db-clean`: executed `cmd_db_clean` and printed `ETag/IP cache cleanup not yet implemented`.
- `just test-offline`: passed.

Offline automated test result summary:

- Unit tests: passed
- CLI smoke tests: passed
- Root offline integration tests: passed
- `grel-elf` integration tests: passed
- Live provider tests: intentionally ignored by default

## 2. High-Severity Findings

### 2.1 Unmanaged package removal can delete the entire configured download directory

Status: `FAILED`

Evidence:

- `src/commands/sync.rs:739-747` stores unmanaged installs under `ctx.config.paths.download_dir`.
- `src/commands/sync.rs:827` persists `install_path` as that directory path, not the downloaded file path.
- `src/commands/upgrade.rs:156-176` repeats the same model for failed local extraction by marking the package unmanaged while still storing the directory path.
- `src/commands/remove.rs:249-253` deletes `install_path` recursively with `remove_dir_all` when it is a directory.

Impact:

- Removing one unmanaged package can recursively remove the whole shared download directory, including unrelated files and unrelated unmanaged packages.

Expected:

- For unmanaged packages, `install_path` should point to the downloaded file, or removal should target the specific archive path only.

Actual:

- The package record points at the directory, and uninstall removes the directory tree.

### 2.2 `--asdeps` / dependency-installed state is not persisted during install or upgrade

Status: `FAILED`

Evidence:

- `src/commands/sync.rs:364` and `src/commands/sync.rs:832` set `pkg.is_explicit = is_explicit`.
- `crates/grel-cache/src/database.rs:294-306` does not include `is_explicit` in the `INSERT INTO installed` column list.
- `crates/grel-cache/src/database.rs:298-306` also does not update `is_explicit` in the `ON CONFLICT` clause.

Impact:

- `grel -S --asdeps owner/repo` is silently recorded as explicit install.
- `-Qd`, `-Qe`, `-Ru`, and cascade orphan detection can produce incorrect results.

Expected:

- `is_explicit` should be persisted on first install and preserved or updated on later upserts.

Actual:

- The database default `is_explicit = 1` wins for new installs.

### 2.3 `-Qt` / `--unrequired` is wired to repository-orphan status, not dependency orphan detection

Status: `FAILED`

Evidence:

- `docs/COMMANDS_DESIGN.md` defines `-Q -t` as “Show packages not required by any other (orphans)”.
- `src/main.rs:82-83` routes `cli.unrequired || cli.orphans` to `commands::query::cmd_list_orphans`.
- `src/commands/query.rs:176-180` implements `cmd_list_orphans` by filtering `PackageStatus::Orphaned`.

Impact:

- Users asking for dependency orphans get only packages whose upstream repo became unreachable or was manually marked orphaned.

Expected:

- `-Qt` should compute unrequired installed packages from the dependency graph.

Actual:

- `-Qt` shows a different concept entirely.

### 2.4 Documented database commands `-D --clean`, `-D --check`, and `-D --dump` are not actually wired

Status: `FAILED`

Evidence:

- `docs/COMMANDS_DESIGN.md` documents `grel -D --clean`, `grel -D --check`, and `grel -D --dump`.
- `crates/grel-cli/src/commands.rs:251-257` exposes `db_clean`, `db_check`, and `db_dump`, which clap maps to `--db-clean`, `--db-check`, and `--db-dump`.
- `src/main.rs:114-119` only dispatches to database handlers via `cli.db_clean`, `cli.db_check`, and `cli.db_dump`.
- Runtime confirmation:
  - `cargo run -- -D --clean` did not trigger the database clean command.
  - `cargo run -- -D --db-clean` did trigger it.

Impact:

- The published database command syntax does not work as documented.

Expected:

- The documented `--clean`, `--check`, and `--dump` flags under `-D` should execute the database handlers.

Actual:

- Only `--db-clean`, `--db-check`, and `--db-dump` work.

### 2.5 `--verify-signatures` is advertised as an enforcement control, but no verification is implemented

Status: `FAILED`

Evidence:

- `crates/grel-config/src/config.rs:300-302` states that when enabled, grel “will refuse to install assets that lack a valid detached signature or checksum file”.
- `src/main.rs:33-37` only toggles the boolean in config.
- `src/commands/sync.rs:556-561` and `src/commands/sync.rs:1423-1428` only print a warning when verification is disabled.
- No signature validation or refusal path exists in download/install flows.

Impact:

- Users can reasonably believe they are protected by signature enforcement when they are not.

Expected:

- Enabling verification should validate detached signatures or checksums and block installation on failure or absence.

Actual:

- The flag changes messaging only.

## 3. Medium-Severity Findings

### 3.1 `-Qo` / `--owns` is not implemented as file ownership lookup

Status: `FAILED`

Evidence:

- `src/commands/query.rs:297-304` matches `install_path.contains(path)` or `asset_filename.contains(path)`.

Impact:

- `grel -Qo rg` will not reliably tell the user which package owns the installed `rg` binary unless the query happens to appear in the archive filename or install path string.

Expected:

- Search actual extracted file paths and linked binaries.

Actual:

- Only string containment against package metadata is used.

### 3.2 `--proxy` CLI flag is exposed but not applied

Status: `FAILED`

Evidence:

- `crates/grel-cli/src/commands.rs:312-314` defines `cli.proxy`.
- `src/main.rs` never copies `cli.proxy` into `config.general.proxy`.
- `crates/grel-network/src/client.rs:29-36` only reads `config.proxy` and environment variables.

Impact:

- `--proxy` on the command line has no effect.

### 3.3 `--allow-format` CLI flag is exposed but unused

Status: `FAILED`

Evidence:

- `crates/grel-cli/src/commands.rs:301-302` defines `allow_format`.
- There is no consumer of `ctx.cli.allow_format` in `src/` or `crates/`.

Impact:

- Users cannot temporarily allow ignored formats as documented.

### 3.4 `--overwrite` is exposed but unused

Status: `FAILED`

Evidence:

- `crates/grel-cli/src/commands.rs:280-282` defines `overwrite`.
- There is no usage of `overwrite` anywhere in `src/`, `crates/`, or tests.

Impact:

- Conflict resolution behavior does not match the advertised CLI contract.

### 3.5 `-R --nosave` is currently a no-op

Status: `FAILED`

Evidence:

- `src/commands/remove.rs:257-259` explicitly states `nosave` is currently a no-op.

Impact:

- The flag is user-visible but has no behavior behind it.

### 3.6 Local file install flag names do not match the documented CLI

Status: `FAILED`

Evidence:

- `docs/COMMANDS_DESIGN.md` documents `-U --asset <path>`.
- `crates/grel-cli/src/commands.rs:262-263` exposes `--local-asset`.
- `src/commands/upgrade.rs:14-16` consumes `ctx.cli.local_asset`.

Impact:

- The documented `-U --asset` path form is not the real interface.

## 4. Low-Severity And Documentation Drift

### 4.1 Recursive remove short flag differs from the documented CLI

Status: `MISMATCH`

Evidence:

- `docs/COMMANDS_DESIGN.md` documents `-R -s` for recursive remove.
- `crates/grel-cli/src/commands.rs:223-228` exposes `-r, --recursive`.

### 4.2 README command examples are stale relative to the actual pacman-style CLI

Status: `MISMATCH`

Evidence:

- `README.md` still shows commands such as `grel sync`, `grel list`, `grel remove`, `grel path add`, and a custom output flag `-O`.
- The current CLI help exposes only the pacman-style operation flags and does not provide `path add` or `-O`.

### 4.3 `cmd_db_clean` is only partially implemented

Status: `PARTIAL`

Evidence:

- Runtime execution of `cargo run -- -D --db-clean` prints `ETag/IP cache cleanup not yet implemented`.
- `src/commands/database.rs:58-71` removes orphaned package records only.

### 4.4 `cmd_db_check` is only a record-count health message, not a real integrity check

Status: `PARTIAL`

Evidence:

- `src/commands/database.rs:76-85` only lists package count and prints “Database appears healthy”.

## 5. Expected CLI Coverage Matrix

| Area | Expected capability | Current status | Notes |
|---|---|---|---|
| `-S` install/search/refresh/upgrade/info | Core flow present | `PARTIAL` | Main package flow exists, but `--proxy`, `--allow-format`, `--overwrite`, and `--verify-signatures` are incomplete or unwired. |
| `-S --asdeps` / dependency install metadata | Persist dependency-installed state | `FAILED` | `is_explicit` not written during upsert. |
| `-S --asset`, `-S --platform` | Manual asset selection and platform override | `PASS` | Implemented in `src/commands/sync.rs`. |
| `-Q` list/info/search/check | Basic queries | `PARTIAL` | `-Qo` and `-Qt` do not match expected behavior. |
| `-R` remove/unneeded/cascade | Basic removal works | `PARTIAL` | `--nosave` is a no-op; unmanaged uninstall path is unsafe. |
| `-D` clean/check/dump/migrate | Database maintenance | `PARTIAL` | Real flags differ from docs; clean/check are partial implementations. |
| `-U` local archive install | Local file install | `PARTIAL` | Core install works, but documented flag naming differs and unmanaged-path semantics are unsafe. |
| `-F` file search/list/reindex | File queries | `PASS/PARTIAL` | Works by walking install paths, but no persistent index exists despite the “reindex” wording. |

## 6. Automated Test Coverage Review

Existing automated coverage is useful but incomplete relative to the documented CLI.

Covered well:

- Asset parsing and deterministic resolver behavior
- Dependency graph utilities
- ELF distro/package resolution logic
- Archive safety helpers
- Database system dependency cache
- Some upgrade-related DB state behavior
- Basic CLI help/version/invalid-forge smoke cases

Not adequately covered:

- `-D` flag wiring versus documented syntax
- `--asdeps` persistence in the database
- `-Qt` semantic correctness
- `-Qo` ownership lookup correctness
- Unmanaged package uninstall safety
- `--proxy`, `--allow-format`, `--overwrite`, and `--verify-signatures`
- README and command-design drift against actual clap surface

## 7. Recommended Fix Order

1. Fix unmanaged package path semantics and uninstall behavior before any release.
2. Fix `Database::upsert_package` to persist `is_explicit`.
3. Rewire `-Qt` to dependency-orphan detection and keep repo-unreachable status as a separate query concept.
4. Align `-D` flag names with the documented interface or update the docs and tests immediately.
5. Either implement signature verification and format/proxy/overwrite behavior, or remove those flags from the public CLI until they are real.
6. Add regression tests for every issue above before merging further CLI work.

## 8. Final Conclusion

The current codebase demonstrates a substantial amount of implementation work and a passing offline test suite, but the CLI feature set is not complete relative to its own documented contract. Several public flags are placeholders, multiple command/documentation pairings are mismatched, and at least one uninstall path has data-loss risk. The project should not be considered feature-complete for the documented CLI until the high-severity findings are resolved and covered by regression tests.
