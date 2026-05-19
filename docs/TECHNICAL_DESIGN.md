> **Note:** This is a historical design document. Some details may be outdated. For the current architecture overview, see [ARCHITECTURE.md](ARCHITECTURE.md).

# 📘 `grel-rs` Technical Design Document
**Binary:** `grel` | **Repository:** `grel-rs`  
**Target Platforms:** Linux, Windows (macOS optional)  
**Core Philosophy:** Pure CLI, deterministic asset resolution, explicit over implicit, robust upgrade handling, transparent user control.


---

## 1. Overview & Goals
`grel` is a terminal-native, high-performance release downloader and package manager for Git forges. It abstracts provider APIs into a unified pipeline, supports proxies, caches DNS/IPs for CDN routing, downloads in parallel, and delivers a transparent, scriptable, pacman-compatible UX.

**Key Requirements Met:**
- ✅ Pure CLI (no TUI/GUI), standard `std::io` prompts & warnings
- ✅ Deterministic resolution: **strict filters → priority sorting → explicit policy fallback** (zero scoring)
- ✅ `exclude_keywords` config to block installer/setup/bundle artifacts
- ✅ Warning system for unmanaged or extra-step packages (applies to `ignore_formats` overrides & keyword matches)
- ✅ Configurable `download_dir` for unmanaged packages (defaults to OS `Downloads/`)
- ✅ `default_selection_policy` (`first` | `largest`) as explicit fallback, overridden by detailed flags
- ✅ `-Syu` resilience: filename rename detection, orphan tracking, explicit migration
- ✅ Cross-platform PATH integration, XDG-compliant, no `sudo`


---

## 2. Workspace Architecture
```
grel-rs/
├── Cargo.toml                  # Workspace root (resolver = "2")
├── crates/
│   ├── grel-cli/               # CLI parsing, pure-text prompts, progress routing
│   ├── grel-core/              # Resolution, tokenization, filter/sort pipeline, upgrade state
│   ├── grel-providers/         # Forge trait, registry, API implementations (modules)
│   ├── grel-network/           # HTTP client, proxy routing, IP cache resolver, parallel downloader
│   ├── grel-cache/             # SQLite state, IP cache, artifact storage, TTL eviction
│   └── grel-config/            # Layered config, migrations, asset priority matrices
├── tests/                      # Integration, mock servers, e2e fixtures
└── scripts/                    # CI, release, benchmark helpers
```

| Crate | Responsibility |
|-------|----------------|
| `grel-cli` | Subcommand routing, pure CLI prompt loops, `indicatif` progress, `tabled` output |
| `grel-core` | `PackageRef` parsing, `AssetTokens` extraction, deterministic filter/sort, upgrade planning |
| `grel-providers` | `ReleaseProvider` trait, GitHub/GitLab/Gitea/Codeberg modules, self-hosted auto-detection |
| `grel-network` | Proxy chaining, DNS/IP cache resolver, `JoinSet` parallel downloads, streaming extraction |
| `grel-cache` | `state.sqlite` management, ETag/IP cache, LRU artifact eviction, atomic install temp dirs |
| `grel-config` | TOML loading, env/CLI overrides, schema validation, config migrations |

---

## 3. CLI Specification & Warning Flow
All interaction uses standard terminal I/O. Unmanaged packages trigger explicit warnings & confirmation.

### Warning & Confirmation Flow
When an asset matches `exclude_keywords` or falls under an overridden `ignore_formats`:
```
⚠️  Asset "foo-setup-1.0.0.exe" matches excluded keyword "setup".
ℹ️  grel cannot manage installers directly. Package will download to ~/Downloads/.
:: Proceed with download? [y/N]: _
```
- `y`/`Enter` → Downloads to `download_dir`, marks `is_managed = false` in DB
- `N`/`n`/`Esc` → Aborts sync for this package, continues with others
- `--noconfirm` / `-y` → Auto-accepts, prints `ℹ️ Non-interactive: accepted unmanaged asset` to `stderr`
- **Never blocks CI/pipes:** `!std::io::stdin().is_terminal()` → auto-accepts, logs to `stderr`

---

## 4. Asset Resolution Pipeline (Deterministic)
**No scoring. No fuzzy logic.** Strict filtering → transparent priority → policy fallback.

### Precedence Rules (Strict → Override → Fallback)
| Priority | Mechanism | Override Capability |
|----------|-----------|---------------------|
| 1️⃣ | OS/Arch exact match or `Unknown` | None |
| 2️⃣ | `exclude_keywords` filter | None (hard block unless CLI `--allow-keyword`) |
| 3️⃣ | `ignore_formats` filter | Overridable via CLI/config |
| 4️⃣ | `arch_priority` index | Overrides `default_selection_policy` |
| 5️⃣ | `prefer_formats` index | Overrides `default_selection_policy` |
| 6️⃣ | `default_selection_policy` | **Only applies to remaining ties** |

### Step 1: Strict Filtering
```rust
assets.iter()
    .map(|a| AssetTokens::from_filename(&a.filename))
    .filter(|t| t.os == target.os || t.os == Os::Unknown)
    .filter(|t| !keyword_excluded(t, &config.exclude_keywords))
    .filter(|t| arch_matches_priority(t, &config.arch_priority, config.fallback_to_32bit))
    .filter(|t| !format_ignored(t, &config.ignore_formats))
    .collect()
```

### Step 2: Deterministic Sorting
Sorted by explicit cascade:
1. `arch_priority` index
2. `prefer_formats` index
3. Lexicographic filename
4. Size descending

### Step 3: Policy Fallback (`default_selection_policy`)
Only applied if `>1` asset survives sorting and remains tied.
- `first` → picks top of sorted list
- `largest` → picks by `size_bytes` descending
- **Never overrides** arch/format priority or keyword filters.

### Step 4: Selection & Warning
- `0` → `❌ No compatible assets`
- `1` → Check if unmanaged → warn/confirm → auto-select or prompt
- `>1` → Pure CLI numbered prompt → warn/confirm if unmanaged

---

## 5. Platform Enums & Alias Mapping
Strongly-typed, exhaustive, infallible parsing.
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Os { Linux, Windows, MacOS, FreeBSD, Android, iOS, Unknown(String) }

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Arch { X86_64, Aarch64, I686, ArmV7, ArmV6, Riscv64, S390x, PowerPC64, Unknown(String) }
```
- `FromStr` implemented with `to_lowercase()` + alias table
- `serde` ready for TOML config
- `Display` outputs canonical names for DB/CLI consistency

---

## 6. Configuration Structure (Updated)
Strict separation of human-editable TOML and machine-managed SQLite.

```toml
# grel configuration schema v1
# All values shown are defaults. Omitted keys fall back to these.

[general]
version = 1                      # Schema version (do not modify)
max_concurrent = 4               # Parallel downloads (0 = auto CPU/2, min 2)
proxy = ""                       # Empty = auto-detect $http_proxy/$all_proxy

[assets]
default_selection_policy = "first"  # "first" | "largest" (only breaks ties after strict filtering)
exclude_keywords = ["setup", "installer", "bundle", "nupkg"]
ignore_formats = ["*.deb", "*.rpm", "*.msi", "*.dmg", "*.pkg", "*.AppImage"]
prefer_formats = ["*.tar.gz", "*.tar.xz", "*.zip", "*.exe"]
prefer_32bit_on_64bit = false    # Prefers 32-bit assets ONLY when running on 64-bit OS
fallback_to_32bit = true         # Allows 32-bit install if NO 64-bit asset exists
prefer_musl = false              # Linux-only: prefers musl over gnu builds

[paths]
install_root = "~/.local/share/grel"  # Managed package storage
bin_dir = "~/.local/share/grel/bin"   # Extracted binaries (added to $PATH)
download_dir = "~/Downloads"          # Unmanaged/extra-op packages (falls back to system Downloads/)

[upgrade]
check_interval_hours = 6         # Remote check cooldown per package
max_parallel_checks = 10         # Concurrent API requests during `-Syu`

[auth]
github_token = ""                # Use env $GREL_GITHUB_TOKEN instead
gitlab_token = ""                # Use env $GREL_GITLAB_TOKEN instead
gitea_token = ""                 # Use env $GREL_GITEA_TOKEN instead

[migrations]
# "old/owner/repo" = "new/owner/repo"
```

---

## 7. State Database Schema (`state.sqlite`)
```sql
CREATE TABLE installed (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    
    forge TEXT NOT NULL,
    owner TEXT NOT NULL,
    repo TEXT NOT NULL,
    version TEXT NOT NULL,
    asset_filename TEXT NOT NULL,
    checksum TEXT,
    install_path TEXT NOT NULL,       -- Actual path (bin/ or download_dir)
    installed_binaries TEXT NOT NULL DEFAULT '',
    is_managed BOOLEAN NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'orphaned', 'migrated')),
    orphaned_at INTEGER,
    last_checked INTEGER,
    installed_at INTEGER DEFAULT (strftime('%s', 'now')),
    manifest_source TEXT NOT NULL DEFAULT 'heuristic' CHECK (manifest_source IN ('registry', 'in_repo', 'heuristic')),
    is_explicit BOOLEAN NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX idx_pkg_unique ON installed(forge, owner, repo);
CREATE INDEX idx_status ON installed(status);

CREATE TABLE dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL,
    dep_target TEXT NOT NULL,         -- "forge/owner/repo" or "system:libname"
    dep_type TEXT NOT NULL DEFAULT 'grel' CHECK (dep_type IN ('grel', 'grel_opt', 'system')),
    FOREIGN KEY (package_id) REFERENCES installed(id) ON DELETE CASCADE
);
CREATE INDEX idx_deps_package ON dependencies(package_id);
CREATE INDEX idx_deps_target ON dependencies(dep_target);

CREATE TABLE etag_cache (
    url TEXT PRIMARY KEY,
    etag TEXT NOT NULL,
    last_modified INTEGER NOT NULL
);

CREATE TABLE dns_cache (
    hostname TEXT NOT NULL,
    ip_address TEXT NOT NULL,
    rtt_ms INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (hostname, ip_address)
);

CREATE TABLE system_dep_cache (
    library_name TEXT NOT NULL,
    distro_id TEXT NOT NULL,
    package_name TEXT NOT NULL,
    discovered_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (library_name, distro_id)
);
```
- `is_managed`: `true` = auto-extracted/linked to `bin/`; `false` = left in `download_dir`
- `install_path`: Stores actual destination for accurate cleanup/migration
- `dependencies.dep_target`: Unified target string. Grel packages use `forge/owner/repo`; system libraries use `system:libname` (e.g., `system:libssl.so.3`)

---

## 8. Download Path & Installation Behavior
| Asset Type | Destination | Management |
|------------|-------------|------------|
| Standard binary/archive (`.tar.gz`, `.zip`, `.exe`) | `bin_dir/` (Linux) / `bin\` (Win) | ✅ Managed (extracted, checksummed, linked) |
| Unmanaged/Extra-op (`.msi`, `.deb`, matched keywords) | `download_dir` (default: `~/Downloads`) | ⚠️ Download-only, no extraction/execution |
| CLI Override (`--output-dir ~/tmp`) | User-specified path | ✅ Respected regardless of type |

**Cross-Platform Default Resolution:**
```rust
fn default_download_dir() -> PathBuf {
    directories::UserDirs::new()
        .map(|d| d.download_dir().clone())
        .unwrap_or_else(|| std::env::temp_dir().join("grel-downloads"))
}
```
---

## 9. `-Syu` Upgrade Logic & Edge Cases
### 9.1 Filename Change Detection
Authors often rename assets (`x86_64` → `amd64`, `.tar.gz` → `.tar.xz`).
1. Load stored `asset_filename` from DB
2. Resolve new release → filter → sort → get `new_best`
3. If `new_best.filename != stored.filename`:
   ```
   ⚠️ Remote asset renamed: old-name.tar.gz → new-name.tar.xz
   ℹ️ Proceeding with update...
   ```
4. Download → verify → extract → **update DB record** with new filename. Proceeds safely.

### 9.2 Repository Rename / 404 Handling
1. Provider fetch → `404` or unreachable
2. Mark `status = 'orphaned'`, set `orphaned_at = now()`
3. Skip in future `-Syu` runs
4. Warn user:
   ```
   ⚠️ Package unreachable: foo/bar (HTTP 404)
   ℹ️ Skipped. Run `grel migrate foo/bar new/path` or `grel remove foo/bar`
   ```
5. **No auto-migration.** Explicit user action required.

### 9.3 Orphaned Visibility
```bash
$ grel list
:: Installed packages (3 active, 1 orphaned)
  🟢 bar/fuzz          1.2.3  linux/x86_64  tar.gz
  🟢 foo/tool          0.9.1  windows/amd64 exe
  🟡 legacy/old-proj   2.0.0  linux/x86_64  tar.gz  [orphaned since 2024-05-01]
```

---

## 10. Networking, Proxy & DNS/IP Caching
- **Proxy Priority:** `--proxy` > `GREL_PROXY` env > `http_proxy`/`all_proxy` > `config.toml`
- **DNS/IP Cache:** 
  1. Resolve `A/AAAA` via `hickory-resolver`
  2. Parallel `TcpStream::connect` probes on `443`
  3. Store fastest IP + RTT in SQLite with `300s` TTL
  4. `reqwest::ClientBuilder::resolve(host, ip)` forces routing
- **Background Refresh:** Idle task pre-warms hot domains, evicts stale entries

---

## 11. Rate Limiting & ETag Optimization
GitHub: 60/hr unauth, 5000/hr auth. `grel` handles gracefully:
1. **ETag Caching:** `If-None-Match` → `304 Not Modified` **free**. Cached in DB.
2. **Smart Polling:** Only checks packages where `last_checked < now() - check_interval_hours`
3. **Rate Limit Parsing:** Reads `X-RateLimit-Remaining`/`Reset`. If `< 50` → sleep/queue.
4. **Auth Prompt:** First run suggests `GREL_GITHUB_TOKEN` for 5000/hr.
5. **`--no-api` Fallback:** Skips remote checks, uses local timestamps only.

---

## 12. PATH Integration & Post-Install
- Installs to `~/.local/share/grel/bin/` (Linux) / `%LOCALAPPDATA%\grel\bin\` (Windows)
- `grel path add` prints exact shell/registry snippets:
  ```bash
  export PATH="$HOME/.local/share/grel/bin:$PATH"
  ```
  ```powershell
  [Environment]::SetEnvironmentVariable("Path", "$env:LOCALAPPDATA\grel\bin;$env:Path", "User")
  ```
- First run: `💡 Run: grel path add to enable global command access`
- XDG-compliant, no `sudo`, no system conflicts.

---

## 13. Security & Reliability
- ✅ TLS: `rustls` only
- ✅ Checksums: `sha256` before extraction (managed packages only)
- ✅ Archive safety: Reject `..`, absolute paths, external symlinks
- ✅ Atomic installs: Temp dir → verify → `rename` → DB update
- ✅ Unmanaged warnings: Explicit user consent required (unless `--noconfirm`)
- ✅ Graceful degradation: Network errors → retry/backoff, rate limits → sleep/queue, missing assets → clear error + suggestions

---

## 14. Dependency Matrix
```toml
# Core & Async
tokio = { version = "1", features = ["full"] }
futures = "0.3"
async-trait = "0.1"

# CLI & UX (Pure CLI, no TUI)
clap = { version = "4", features = ["derive", "wrap_help", "string"] }
indicatif = { version = "0.17", features = ["tokio"] }
tracing-indicatif = "0.3"
tabled = "0.16"
owo-colors = "4"

# Network & Proxy
reqwest = { version = "0.12", features = ["rustls-tls", "json", "socks", "stream"] }
hickory-resolver = "0.24"
dashmap = "6"

# Cache & Storage
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "sqlite"] }
sha2 = "0.10"
directories = "5"
tar = "0.4"
zip = "2"
zstd = "0.13"
xz2 = "0.1"
bzip2 = "0.5"
sanitize-filename = "0.5"

# Config & Utils
figment = { version = "0.10", features = ["toml", "env"] }
semver = "1"
url = "2"
chrono = { version = "0.4", features = ["serde"] }
regex = "1"
serde = { version = "1", features = ["derive"] }
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
```

---

## 15. Testing & CI Strategy
- **Unit:** Keyword exclusion, policy fallback precedence, filter/sort determinism, config parsing
- **Mocked Providers:** `wiremock` for 200/304/403/429/500, rate limit headers
- **Integration:** Unmanaged package download → `download_dir` verification, `grel migrate` state changes
- **Non-Interactive:** `echo "" | grel sync foo/bar` must auto-accept with stderr warning
- **Cross-Platform CI:** `ubuntu-latest`, `windows-latest`
- **Performance:** `criterion` for DNS/IP cache + parallel download throughput
- **Fuzzing:** `cargo-fuzz` on `AssetTokens::from_filename()` + archive path validation

---
