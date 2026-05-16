# CLI Regression Review Report
Date: 2026-05-16
Baseline Report: `/testresult.md` (2026-05-14)
Branch: `codex-test-automation-just`

## Conclusion
Following this codebase update, several critical issues listed in the previous `testresult.md` have been resolved. However, the overall verdict still cannot be upgraded from `FAIL` to `PASS`.
The current status is more accurately described as: `Significantly improved, but still incomplete`.

The main reasons fall into three categories:
1. Several legacy issues remain unfixed, particularly the `-D` command flags being inconsistent with documentation, `-Qo` still not performing true file ownership queries, and the `-U` CLI form still mismatching the docs.
2. Some legacy issues are only partially fixed. For example, the `install_path` semantics for unmanaged packages were corrected in certain paths, but the `-Syu` upgrade path still retains inconsistencies.
3. A new regression appeared in the automated test entry point: `just test-offline` currently fails because the test code has not been updated to match the new parameter signature of `archive::install_asset`.

## Actual Execution
The following commands were executed during this review:

```bash
cargo run -- --help
cargo run -- -Sh
cargo run -- -Qh
cargo run -- -D --clean
cargo run -- -D --db-clean
just test-offline
```


### Execution Summary
| Command | Result | Notes |
| --- | --- | --- |
| `cargo run -- --help` | Success | The exposed CLI surface still differs from `docs/COMMANDS_DESIGN.md` |
| `cargo run -- -Sh` | Success | Output is still global help, not `-S` scoped help |
| `cargo run -- -Qh` | Success | Also returns global help |
| `cargo run -- -D --clean` | Exits successfully, but does not trigger DB cleanup | Still outputs `No database operation specified. Use -Dh for help.` |
| `cargo run -- -D --db-clean` | Success | Now cleans orphan packages, ETag cache, and DNS cache |
| `just test-offline` | Failed | Test calls in `crates/grel-network/src/archive.rs` are missing the `overwrite: bool` parameter |

## Issue Review Against `testresult.md`

### 1. Uninstalling unmanaged packages deletes the entire download directory
- Old Verdict: `FAILED`
- Current Verdict: `PARTIAL`
- Status:
  - `src/commands/sync.rs:361-364` now changes the `install_path` for unmanaged packages in the standard install path to the specific file path `archive_path`.
  - `src/commands/upgrade.rs:175-178` applies the same fix to local file install paths.
  - However, `upgrade_single_package()` in `src/commands/sync.rs` still uses the old semantics, setting `updated_pkg.install_path` to a directory instead of a specific file during unmanaged upgrades.
- Evidence:
  - Standard install path: `src/commands/sync.rs:361-364`
  - Local file install path: `src/commands/upgrade.rs:175-178`
  - Upgrade path not fully corrected: `upgrade_single_package()` logic starting at `src/commands/sync.rs:1464`
  - Deletion logic still decides `remove_dir_all` based on whether `install_path` is a directory: `src/commands/remove.rs:223-235`
- Judgment: This issue is no longer as dangerous as "all unmanaged paths are risky", but it cannot be considered fully resolved.

### 2. `--asdeps` / non-explicit install state not persisted
- Old Verdict: `FAILED`
- Current Verdict: `FIXED`
- Evidence:
  - `crates/grel-cache/src/database.rs:294-307` now includes `is_explicit` in `INSERT INTO installed`
  - The `ON CONFLICT` branch at `crates/grel-cache/src/database.rs:298-307` also updates `is_explicit`
- Judgment: Substantively fixed.

### 3. `-Qt` incorrectly implemented as "repo unreachable orphan" instead of "dependency orphan"
- Old Verdict: `FAILED`
- Current Verdict: `FIXED`
- Evidence:
  - `src/main.rs:85-88` now routes `cli.unrequired` and `cli.orphans` separately
  - `src/commands/query.rs:217-249` adds `cmd_list_unrequired()`, which calculates unrequired packages based on the dependency graph
  - `src/commands/query.rs:152-214` retains `cmd_list_orphans()` for repo-unreachable status queries
- Judgment: Semantically split and corrected. Clearly fixed.

### 4. `-D --clean/--check/--dump` in docs not actually wired up
- Old Verdict: `FAILED`
- Current Verdict: `NOT FIXED`
- Evidence:
  - Running `cargo run -- -D --clean` still outputs: `No database operation specified. Use -Dh for help.`
  - Clap fields remain `db_clean/db_check/db_dump`: `crates/grel-cli/src/commands.rs:251-257`
  - Working commands are still `--db-clean/--db-check/--db-dump`
- Judgment: Completely unfixed. Implementation was enhanced, but the external CLI names remain incorrect.

### 5. `--verify-signatures` only has a toggle, no real signature verification
- Old Verdict: `FAILED`
- Current Verdict: `PARTIAL`
- Status:
  - `src/commands/sync.rs:749-760` now at least checks for a matching `.sig` or `.asc` file in the release when verification is enabled, skipping installation if absent.
  - This is still not "signature verification", merely "sidecar file existence detection".
  - No cryptographic verification of signature content, chain of trust, or checksum file body was found.
- Judgment: Improved, but still fails to meet the `grel-config` promise of "refusing to install assets without valid signatures/checksums".

### 6. `-Qo` / `--owns` is not an actual file ownership query
- Old Verdict: `FAILED`
- Current Verdict: `NOT FIXED`
- Evidence:
  - `src/commands/query.rs:352-359` still only performs:
    - `install_path.contains(path)`
    - `asset_filename.contains(path)`
- Judgment: Still not an ownership lookup based on actual extracted files / linked binaries.

### 7. `--proxy` CLI flag not wired up
- Old Verdict: `FAILED`
- Current Verdict: `FIXED`
- Evidence:
  - `src/main.rs:52-54` now writes `cli.proxy` back to `config.general.proxy`
  - HTTP client construction in `crates/grel-network/src/client.rs:29-36` consumes this value

### 8. `--allow-format` exposed but ineffective
- Old Verdict: `FAILED`
- Current Verdict: `FIXED`
- Evidence:
  - `src/commands/sync.rs:549-552` removes the specified format from `ignore_formats` before constructing `ResolverConfig`
- Limitation: Current implementation is "exact string removal from config ignore list", not a stronger pattern extension, but it is no longer unwired.

### 9. `--overwrite` exposed but ineffective
- Old Verdict: `FAILED`
- Current Verdict: `FIXED`
- Evidence:
  - `src/commands/sync.rs:311-316`, `809-814`, `1511-1516`
  - `src/commands/upgrade.rs:129-134`
  - `crates/grel-network/src/archive.rs:363-365`
- Judgment: Overwrite logic is now integrated into the archive/link installation paths.

### 10. `-R --nosave` is a no-op
- Old Verdict: `FAILED`
- Current Verdict: `PARTIAL`
- Status:
  - `src/commands/remove.rs:223-235` now preserves config files when `nosave = false`.
  - `src/commands/remove.rs:241-278` adds `preserve_config_files()`, which copies files to a `.grelnew` directory based on extension.
- Limitation: Simplified implementation that guesses config files by extension, not the precise metadata-based preservation used by pacman.

### 11. `-U` CLI form inconsistent with documentation
- Old Verdict: `FAILED`
- Current Verdict: `NOT FIXED`
- Evidence:
  - Docs `docs/COMMANDS_DESIGN.md` still specify `-U --asset <path>`
  - Actual CLI field remains `local_asset`: `crates/grel-cli/src/commands.rs:262-263`
  - Implementation still reads `ctx.cli.local_asset`: `src/commands/upgrade.rs:14`, `40`

### 12. Recursive removal short flag inconsistent with documentation
- Old Verdict: `MISMATCH`
- Current Verdict: `NOT FIXED`
- Evidence:
  - Docs specify `-R -s`
  - Implementation remains `-r, --recursive`: `crates/grel-cli/src/commands.rs:223-228`
  - Worse, `long_help` still says `-s (recursive)`, contradicting the actual flag: `crates/grel-cli/src/commands.rs:73-75`

### 13. README outdated
- Old Verdict: `MISMATCH`
- Current Verdict: `PARTIAL`
- Status:
  - README updated from `grel sync / grel list / grel remove` to pacman-style `-S / -Q / -R`
  - However, README now examples `-D --db-clean / --db-check`, continuing to diverge from `docs/COMMANDS_DESIGN.md`
- Judgment: README is updated, but intra-repo "documentation consistency" is still not achieved.

### 14. `cmd_db_clean` only does partial work
- Old Verdict: `PARTIAL`
- Current Verdict: `FIXED`
- Evidence:
  - `src/commands/database.rs:82-90` now calls:
    - `db.clean_etag_cache()`
    - `db.clean_dns_cache()`
  - `crates/grel-cache/src/database.rs` has corresponding implementations

### 15. `cmd_db_check` only prints package count, not a real integrity check
- Old Verdict: `PARTIAL`
- Current Verdict: `FIXED`
- Evidence:
  - `src/commands/database.rs:98-117` now calls `db.check_integrity()`
  - `crates/grel-cache/src/database.rs` implements `PRAGMA integrity_check`

## Newly Discovered Issues

### A. `just test-offline` currently fails, test baseline regression
- Severity: `HIGH`
- Phenomenon: `just test-offline` fails in this run
- Error located at `crates/grel-network/src/archive.rs:707`
- Root cause: Test code still calls `link_binaries(&install_dir, &bin_dir, "mytool.tar.gz")` using the old signature. `link_binaries()` now requires a 4th parameter: `overwrite: bool`
- Judgment: Not a product logic bug, but a test infrastructure regression. It directly blocks offline automated verification for this branch and must be fixed before merging.

### B. `-Qk` checksum verification path semantics still inconsistent for unmanaged packages
- Severity: `MEDIUM`
- Evidence: `src/commands/query.rs:395` hardcodes archive path calculation as `Path::new(&pkg.install_path).join(&pkg.asset_filename)`
- However, in the new version, some unmanaged install paths now set `install_path` directly to a file path.
- Impact: Some unmanaged packages may be falsely flagged as `MISSING` during `-Qk`.

### C. `-Sh` / `-Qh` do not actually provide scoped help
- Severity: `LOW`
- Evidence: Running `cargo run -- -Sh` and `cargo run -- -Qh` both return the same global help instead of operation-specific help.
- Impact: Breaks pacman user muscle memory and contradicts the program's own prompt: "Use `grel -Sh`, `grel -Qh`..."

## Final Verdict
Compared to `testresult.md`, the progress in this codebase update is clearly visible, especially in:
- `--proxy` wired up
- `--allow-format` wired up
- `--overwrite` wired up
- `is_explicit` persisted
- `-Qt` corrected to dependency orphan semantics
- `cmd_db_clean` and `cmd_db_check` are no longer stubs

However, the following issues still prevent a `PASS` verdict:
- `-D --clean/--check/--dump` external CLI still mismatches documentation
- `-Qo` still not truly implemented
- `--verify-signatures` still only checks for sidecar file existence, no real cryptographic verification
- Unmanaged package path semantics remain inconsistent in the `-Syu` upgrade path
- `just test-offline` currently fails

**Comprehensive Verdict: `FAIL` (Significantly improved from the previous version, but still not in a complete, merge-ready state)**