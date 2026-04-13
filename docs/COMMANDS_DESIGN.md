## 📐 Command Syntax
```
grel <OPERATION> [OPTIONS] [TARGETS...]
```
Operations are mutually exclusive. Options can be combined without spaces (e.g., `-Syu`).

---

## 🧩 Operation & Option Reference

### `-S, --sync` (Fetch & Install from Forges)
| Flag | Long | Description |
|------|------|-------------|
| `-s` | `--search <pattern>` | Search registered forges for packages |
| `-y` | `--refresh` | Force-refresh forge metadata, IP cache, and ETags |
| `-u` | `--sysupgrade` | Upgrade all installed packages to latest matching releases |
| `-i` | `--info <pkg>` | Show release metadata before installing |
| `-c` | `--clean` | Purge downloaded artifacts from cache |
| `--dry-run` | | Simulate resolution/download without writing files |
| `--noconfirm` | | Skip all interactive prompts & warnings |
| `--overwrite` | | Replace existing binaries if files conflict |
| `--asset <name>` | | Bypass auto-selection, download exact filename |
| `--platform <os/arch>` | | Override host platform detection (e.g., `linux/aarch64`) |
| `--exclude-keywords <k1,k2>` | | Temporarily add keywords to exclusion list |
| `--allow-format <fmt>` | | Temporarily allow normally ignored formats |

**Common Combos:**
- `grel -S foo/bar` → Install latest
- `grel -Ss ripgrep` → Search for `ripgrep`
- `grel -Sy` → Refresh metadata only
- `grel -Su` → Upgrade installed (uses cached metadata)
- `grel -Syu` → Refresh + Upgrade (standard update)
- `grel -Ssc` → Search + Clean cache
- `grel -Si foo/bar` → Show remote release info

---

### `-Q, --query` (Inspect Local State)
| Flag | Long | Description |
|------|------|-------------|
| `-l` | `--list` | List all installed packages |
| `-i` | `--info <pkg>` | Show detailed info of an installed package |
| `-o` | `--owns <path>` | Find which package owns a binary/file |
| `-q` | `--quiet` | Output minimal data (package names only) |
| `-e` | `--explicit` | Filter to manually installed packages |
| `-k` | `--check` | Verify checksums of installed binaries |
| `--orphans` | | Show packages marked `orphaned` in DB |

**Common Combos:**
- `grel -Ql` → List installed
- `grel -Qi foo/bar` → Show local install info
- `grel -Qo rg` → Which package provides `rg`?
- `grel -Qeq` → List explicitly installed, quiet mode
- `grel -Qk` → Verify checksums of all installed packages

---

### `-R, --remove` (Uninstall)
| Flag | Long | Description |
|------|------|-------------|
| `-c` | `--cascade` | Remove package + unneeded dependencies |
| `-n` | `--nosave` | Do not preserve extracted files/config backups |
| `-s` | `--recursive` | Remove packages that depend on the target |
| `--noconfirm` | | Skip removal confirmation |
| `--dry-run` | | Show what would be removed |

**Common Combos:**
- `grel -R foo/bar` → Uninstall
- `grel -Rcns foo/bar` → Cascade + Recursive + Nosave
- `grel -Rn foo/bar` → Remove, keep extracted binaries

---

### `-D, --database` (Local DB & State Management)
| Flag | Long | Description |
|------|------|-------------|
| `--asexplicit <pkg>` | | Mark package as manually installed |
| `--asdeps <pkg>` | | Mark package as a dependency |
| `--migrate <old> <new>` | | Update `owner/repo` path for renamed projects |
| `--clean` | | Prune orphaned records, clear stale ETags/IP cache |
| `--check` | | Verify SQLite DB integrity |
| `--dump` | | Export state as JSON (debug/scripting) |

**Common Combos:**
- `grel -D --migrate old/org/tool new/org/tool`
- `grel -D --clean`
- `grel -D --check`

---

### `-U, --upgrade` (Install from Local File)
| Flag | Long | Description |
|------|------|-------------|
| `--overwrite` | | Overwrite conflicting binaries |
| `--noconfirm` | | Skip prompts & warnings |
| `--asset <path>` | | Treat file as direct download (bypass extraction) |

**Common Combos:**
- `grel -U ./ripgrep-14.1.1-x86_64-linux.tar.gz`
- `grel -U ./tool.exe --noconfirm`

---

### `-F, --files` (File Index & Binary Search)
| Flag | Long | Description |
|------|------|-------------|
| `-s` | `--search <pattern>` | Search installed packages for a filename/binary |
| `-l` | `--list <pkg>` | List all files extracted by a package |
| `-y` | `--refresh` | Rebuild file index from installed packages |
| `-q` | `--quiet` | Output only matching paths |

**Common Combos:**
- `grel -Fs rg` → Find which package provides `rg`
- `grel -Fl foo/bar` → List all files from `foo/bar`
- `grel -Fy` → Reindex installed packages

---

## 🔀 Option Combining & Precedence Rules
1. **Stacking:** `-Syu` ≡ `-S -y -u`. Flags can be merged without spaces.
2. **Operation First:** The first `-D/-Q/-R/-S/-U/-F` determines the operation. Subsequent operation flags are ignored.
3. **Override Hierarchy:**
   ```
   CLI flags > Environment vars > config.toml > compiled defaults
   ```
4. **Conflict Handling:**
   - `--dry-run` + `--noconfirm` → `--dry-run` wins, no prompts shown
   - `-c` (clean) in `-S` vs `-D` → `-S -c` cleans artifacts, `-D --clean` prunes DB/cache
   - `--overwrite` required if destination file exists and checksum differs

---

## 🧠 Internal Mapping to Crates
| CLI Layer | Routes To |
|-----------|-----------|
| `-S/-U` | `grel-cli` → `grel-core/resolver` → `grel-network/download` → `grel-cache/install` |
| `-Syu` | `grel-core/upgrade` (ETag check → diff → parallel download → atomic replace) |
| `-Q/-F` | `grel-cache/sqlite` (read-only queries, index lookups) |
| `-R/-D` | `grel-cache/state` (DB updates, orphan marking, migration, cleanup) |
| `--asset/--platform` | `grel-core/asset_resolver` (bypass filters, force selection) |
| `--noconfirm` | `grel-cli/prompts.rs` (TTY detection bypass, auto-accept warnings) |

---

## 📝 Example Workflows
```bash
# Standard update cycle
grel -Syu

# Search & install with manual asset selection
grel -Ss "fd find"
grel -S sharkdp/fd --asset fd-v10.1.0-x86_64-unknown-linux-musl.tar.gz

# Force 32-bit on 64-bit host, skip prompts
grel -Su --prefer-32bit --noconfirm

# Clean everything, refresh metadata, reinstall
grel -D --clean
grel -Sc
grel -Syu

# Migrate renamed repo, verify checksums
grel -D --migrate old-org/tool new-org/tool
grel -Qk
```

This design preserves pacman's muscle memory, strictly separates operations, supports flag combining, and maps cleanly to `grel`'s deterministic, forge-agnostic architecture. Ready for `clap` derivation and command routing.
