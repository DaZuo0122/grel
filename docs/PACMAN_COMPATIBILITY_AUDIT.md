> **Note:** This is a historical audit from 2026-05-09. Many issues listed here have since been fixed. For current CLI behavior, see [COMMANDS.md](COMMANDS.md) and [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

# grel ↔ pacman CLI Compatibility Audit

> **Date:** 2026-05-09  
> **Reference:** Arch Linux `pacman` v6+  
> **Scope:** Compare every flag and operation against pacman conventions. Mark breaks in muscle memory.

---

## Legend

| Icon | Meaning |
|------|---------|
| ✅ | Matches pacman behavior |
| ⚠️ | Partial match / works but differently |
| ❌ | Missing or broken — breaks muscle memory |
| 🚫 | Intentionally different (architectural) |

---

## `-S, --sync` (Fetch & Install)

| Flag | pacman behavior | grel behavior | Status |
|------|-----------------|---------------|--------|
| `-S <pkg>` | Install package | Install package | ✅ |
| `-Ss <pattern>` | Search remote repos | Search remote forges | ✅ |
| `-Sy` | Refresh package database | Refresh metadata | ✅ |
| `-Su` | Upgrade installed packages | Upgrade installed packages | ✅ |
| `-Syu` | Refresh + upgrade | Refresh + upgrade | ✅ |
| `-Si <pkg>` | Show remote package info | Show remote release info | ✅ |
| `-Sc` | Clean old packages from cache | Cleans `download_dir` + stale archives under `install_root` | ⚠️ |
| `-Scc` | Clean ALL cache | Not supported | ⚠️ |
| `-Sw <pkg>` | Download only, don't install | Implemented — downloads to `download_dir`, skips extraction | ✅ |
| `-S --asdeps` | Install packages as dependencies | Implemented during sync | ✅ |
| `-S --asexplicit` | Install packages as explicit | Implemented during sync | ✅ |
| `-S --needed` | Skip reinstall if up-to-date | Implemented — compares DB version to remote tag | ✅ |
| `-S --overwrite` | Overwrite conflicting files | Flag accepted, no logic | ⚠️ |
| `-S -i` (no arg) | Prompt before install | Not supported | 🚫 |

**Key gaps:** `-Scc` (deep cache clean) not implemented.

---

## `-Q, --query` (Inspect Local State) — ⚠️ HAS BREAKAGES

| Flag | pacman behavior | grel behavior | Status |
|------|-----------------|---------------|--------|
| `-Q` (no flags) | List all installed packages | List all installed packages | ✅ |
| `-Qq` | Quiet list (names only) | Quiet list (names only) | ✅ |
| `-Qi <pkg>` | Show installed package info | Show installed package info | ✅ |
| `-Qo <path>` | Find package owning file | Find package owning file | ✅ |
| `-Qe` | List explicitly installed | List explicitly installed | ✅ |
| `-Qeq` | Explicit + quiet | Explicit + quiet | ✅ |
| `-Ql <pkg>` | **List files owned by package** | **Lists files** (DB index + filesystem fallback) | ✅ |
| `-Ql` | List files for ALL packages | Lists files for all packages | ✅ |
| `-Qs <pattern>` | **Search locally installed packages** | **Implemented** | ✅ |
| `-Qd` | **List packages installed as dependencies** | **Implemented** | ✅ |
| `-Qt` | **List unrequired (orphan) packages** | **Implemented** | ✅ |
| `-Qk` | Verify package files | Stub — prints "not yet implemented" | ⚠️ |
| `-Qm` | List foreign (non-repo) packages | Not applicable | 🚫 |
| `-Qn` | List native (repo) packages | Not applicable | 🚫 |
| `-Qg` | List package groups | Not applicable | 🚫 |

### ✅ `-Q -l` is now working

**Fix:** The `-l` flag was restructured to properly accept an optional package argument. `-Ql [pkg]` now lists files using the `package_files` DB index, with a filesystem walk fallback for packages installed before the index existed.

**Current behavior:**
```bash
grel -Ql foo/bar        # Lists files owned by foo/bar
grel -Ql                # Lists files for ALL packages
grel -Q                 # Lists installed package names
```

---

## `-R, --remove` (Uninstall)

| Flag | pacman behavior | grel behavior | Status |
|------|-----------------|---------------|--------|
| `-R <pkg>` | Remove package | Remove package | ✅ |
| `-Rc` | Remove + unneeded deps | Implemented | ✅ |
| `-Rs` | Remove + recursive dependents | Implemented | ✅ |
| `-Rns` | Nosave + cascade | Implemented | ✅ |
| `-Ru` | **Remove unneeded packages** | **Implemented** | ✅ |
| `-R --noconfirm` | Skip confirmation | Skip confirmation | ✅ |
| `-R --dry-run` | Show what would be removed | Show what would be removed | ✅ |



---

## `-D, --database` (Database Management)

| Flag | pacman behavior | grel behavior | Status |
|------|-----------------|---------------|--------|
| `-D --asexplicit <pkg>` | Mark as explicit | Now implemented | ✅ |
| `-D --asdeps <pkg>` | Mark as dependency | Now implemented | ✅ |
| `-D --check` | Check database integrity | Check DB integrity | ✅ |
| `-D -k` / `--check` | Test database | Test DB | ✅ |
| `-D --clean` | Clean database | Clean orphaned records | ✅ |

**Note:** In pacman, `--asdeps` and `--asexplicit` can also be used with `-S`. grel only supports them via `-D`.

---

## `-U, --upgrade` (Install Local File)

| Flag | pacman behavior | grel behavior | Status |
|------|-----------------|---------------|--------|
| `-U <file>` | Install local package file | Stub — prints "not yet implemented" | ❌ |
| `-U --noconfirm` | Skip prompts | Flag exists | ⚠️ |

**Key gap:** `-U` is entirely stubbed despite being a core pacman operation.

---

## `-F, --files` (File Database)

| Flag | pacman behavior | grel behavior | Status |
|------|-----------------|---------------|--------|
| `-Fy` | Refresh file database | Rebuilds and persists `package_files` index | ✅ |
| `-Fs <pattern>` | Search file database | DB `package_files` LIKE search | ✅ |
| `-Fl <pkg>` | List files in package | DB `package_files` index + filesystem fallback | ✅ |
| `-Fx <pattern>` | Regex search | Not supported | ⚠️ |
| `-Fq` | Quiet file search | Quiet flag exists | ✅ |

**Key gap:** `-Fx` (regex search) not supported.

---

## Global / Common Flags

| Flag | pacman behavior | grel behavior | Status |
|------|-----------------|---------------|--------|
| `--noconfirm` | Skip all prompts | Skip all prompts | ✅ |
| `--dry-run` | Simulate only | Simulate only | ✅ |
| `--config <path>` | Use alternate config | Use alternate config | ✅ |
| `--debug` | Verbose debug output | Not supported | ⚠️ |
| `--disable-download-timeout` | Disable timeout | Not supported | 🚫 |

---

## Summary: Muscle Memory Breaks

| Rank | Issue | Impact | Fix Complexity |
|------|-------|--------|----------------|
| 1 | `-U` entirely stubbed | **High** — local install doesn't work | Medium |
| 2 | `-Sc` partial (no `-Scc`) | **Low** — deep cache clean missing | Low |
| 3 | `-Fx` regex search missing | **Low** — advanced file search | Low |

---

## Root Causes

1. **Flag namespace collision:** `-s` is assigned to `-Ss` (remote search) and cannot be reused for `-Qs` (local search) without restructuring. (Resolved: `-Qs` now works via clap subcommand routing.)
2. **Architecture gaps:** File index is now implemented (`package_files` table). `-Ql`, `-Fl`, `-Fy` work with DB lookups + filesystem fallback.
