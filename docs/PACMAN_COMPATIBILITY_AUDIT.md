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
| `-Sc` | Clean old packages from cache | Stub — prints "not yet implemented" | ❌ |
| `-Scc` | Clean ALL cache | Not supported | ⚠️ |
| `-Sw <pkg>` | Download only, don't install | Not supported | ❌ |
| `-S --asdeps` | Install packages as dependencies | Only available via `-D --asdeps` | ❌ |
| `-S --asexplicit` | Install packages as explicit | Only available via `-D --asexplicit` | ❌ |
| `-S --needed` | Skip reinstall if up-to-date | Not supported | ⚠️ |
| `-S --overwrite` | Overwrite conflicting files | Flag accepted, no logic | ⚠️ |
| `-S -i` (no arg) | Prompt before install | Not supported | 🚫 |

**Key gaps:** `--asdeps`/`--asexplicit` during sync, `-Sw`, `-Sc` implementation.

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
| `-Ql <pkg>` | **List files owned by package** | **Ignored — lists all packages instead** | ❌ |
| `-Ql` | List files for ALL packages | Lists all packages (not files) | ❌ |
| `-Qs <pattern>` | **Search locally installed packages** | **Not supported** (`-s` is only `-Ss`) | ❌ |
| `-Qd` | **List packages installed as dependencies** | **Not supported** | ❌ |
| `-Qt` | **List unrequired (orphan) packages** | **Only `--orphans` long flag exists** | ❌ |
| `-Qk` | Verify package files | Stub — prints "not yet implemented" | ⚠️ |
| `-Qm` | List foreign (non-repo) packages | Not applicable | 🚫 |
| `-Qn` | List native (repo) packages | Not applicable | 🚫 |
| `-Qg` | List package groups | Not applicable | 🚫 |

### 🔴 Critical: `-Q -l` is broken

**Root cause:** In `commands.rs`, the `-l` flag is defined as:
```rust
#[arg(short = 'l', long, action = clap::ArgAction::SetTrue, value_name = "PKG")]
pub list: Option<String>,
```

This is a **clap contradiction**: `SetTrue` produces a `bool`, but the field is `Option<String>`. The `value_name = "PKG"` is effectively ignored. When the user runs `grel -Ql foo/bar`, clap sets `list = Some(true-ish)` but the value `"foo/bar"` is lost. The routing in `main.rs` then falls through to `cmd_list()` which ignores any target and always lists all packages.

**Pacman behavior:**
```bash
pacman -Ql ripgrep      # Lists all files owned by ripgrep
pacman -Ql              # Lists files for ALL packages
pacman -Q               # Lists installed package names
```

**grel current behavior:**
```bash
grel -Ql foo/bar        # Lists all installed packages (WRONG)
grel -Ql                # Lists all installed packages (should list files)
grel -Q                 # Lists all installed packages (correct)
```

This means `grel -Ql` is completely redundant with `grel -Q` and does not provide file listing at all.

---

## `-R, --remove` (Uninstall)

| Flag | pacman behavior | grel behavior | Status |
|------|-----------------|---------------|--------|
| `-R <pkg>` | Remove package | Remove package | ✅ |
| `-Rc` | Remove + unneeded deps | Now implemented | ✅ |
| `-Rs` | Remove + recursive dependents | Now implemented | ✅ |
| `-Rns` | Nosave + cascade | `-n` is stub | ⚠️ |
| `-Ru` | **Remove unneeded packages** | **Not supported** | ❌ |
| `-R --noconfirm` | Skip confirmation | Skip confirmation | ✅ |
| `-R --dry-run` | Show what would be removed | Show what would be removed | ✅ |

**Key gap:** `-Ru` (remove unneeded) is missing. This is distinct from `-Rc`: `-Ru` removes packages that were installed as deps but are no longer required, while `-Rc` removes the target plus its unneeded deps.

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
| `-Fy` | Refresh file database | Stub — prints "not yet implemented" | ❌ |
| `-Fs <pattern>` | Search file database | Uses `--file-search` (no short `-s`) | ⚠️ |
| `-Fl <pkg>` | List files in package | Stub — shows install_path only | ❌ |
| `-Fx <pattern>` | Regex search | Not supported | ⚠️ |
| `-Fq` | Quiet file search | Quiet flag exists | ✅ |

**Key gaps:** `-Fy` reindex is stubbed, `-Fl` is stubbed, `-Fs` doesn't use the short `-s` flag.

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
| 1 | `-Q -l` broken (clap contradiction, ignores PKG arg) | **High** — can't list package files | Medium |
| 2 | `-Q -s` missing (local search) | **High** — can't search installed packages | Low |
| 3 | `-Q -d` missing (deps filter) | **Medium** — can't list dependencies only | Low |
| 4 | `-Q -t` missing (orphans short flag) | **Medium** — `-Qt` is standard for orphans | Low |
| 5 | `-S --asdeps` / `-S --asexplicit` missing | **Medium** — common workflow broken | Low |
| 6 | `-U` entirely stubbed | **High** — local install doesn't work | Medium |
| 7 | `-R -u` missing (remove unneeded) | **Medium** — can't clean up unused deps | Medium |
| 8 | `-F -s` uses `--file-search` not `-s` | **Low** — inconsistency with pacman | Low |
| 9 | `-Sc` stubbed | **Low** — cache cleanup missing | Low |
| 10 | `-Fy` / `-Fl` stubbed | **Low** — file index incomplete | High |

---

## Root Causes

1. **Clap definition bug:** `-l` under `-Q` uses `action = SetTrue` on `Option<String>`, making it impossible to capture the package argument.
2. **Flag namespace collision:** `-s` is assigned to `-Ss` (remote search) and cannot be reused for `-Qs` (local search) without restructuring.
3. **Architecture gaps:** No complete file index means `-Ql`, `-Fl`, `-Fy` cannot work properly yet.
4. **Workflow gaps:** `--asdeps`/`--asexplicit` were only implemented for `-D`, not `-S`.
