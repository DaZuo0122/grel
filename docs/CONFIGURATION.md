# Configuration Guide

`grel` uses a layered configuration system:

1. **Compiled defaults** — hardcoded in `grel-config`
2. **Explicit TOML file** — passed via `-C, --config <PATH>`
3. **Environment variables** — `GREL_*` overrides

> **Important:** `grel` does **not** auto-discover a config file from `~/.config/grel/` or `%APPDATA%\grel\`. You must pass `-C <path>` or rely on defaults + environment variables.

---

## Quick Config Example

```toml
# grel.toml
[general]
version = 1                      # Schema version (do not modify)
max_concurrent = 4               # Parallel downloads (0 = auto CPU/2, min 2)
proxy = ""                       # Empty = auto-detect $http_proxy/$all_proxy
keep_archives = true             # Keep downloaded archives after extraction

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
bin_dir = "~/.local/share/grel/bin"   # Extracted binaries (add to $PATH)
download_dir = "~/Downloads"          # Unmanaged/extra-op packages

[upgrade]
check_interval_hours = 6         # Remote check cooldown per package
max_parallel_checks = 10         # Concurrent API requests during `-Syu`

[auth]
github_token = ""                # Prefer env $GREL_AUTH_GITHUB_TOKEN
gitlab_token = ""                # Prefer env $GREL_AUTH_GITLAB_TOKEN
gitea_token = ""                 # Prefer env $GREL_AUTH_GITEA_TOKEN

[security]
verify_signatures = false        # Check for .sig/.asc sidecar files (not full crypto verify)
enable_hooks = false             # Allow manifest post_install/pre_remove hooks to run (disabled by default for security)

[registry]
url = "https://github.com/grel-registry/packages"
auto_update = true

[elf_deps]
auto_resolve_system_deps = true  # Linux: try to resolve missing shared libs
show_parsed_deps = true          # Print discovered DT_NEEDED libraries
distro_override = ""             # Override distro detection (e.g., "debian")
install_cmd_template = ""        # Override package manager command template
```

Run with:
```bash
grel -C ./grel.toml -S foo/bar
```

---

## Environment Variables

Any config value can be overridden via `GREL_<SECTION>_<KEY>` using Figment's env merging.

| Variable | Example | Maps to |
|----------|---------|---------|
| `GREL_GENERAL_PROXY` | `http://proxy:8080` | `[general] proxy` |
| `GREL_GENERAL_MAX_CONCURRENT` | `8` | `[general] max_concurrent` |
| `GREL_AUTH_GITHUB_TOKEN` | `ghp_xxx` | `[auth] github_token` |
| `GREL_SECURITY_VERIFY_SIGNATURES` | `true` | `[security] verify_signatures` |
| `GREL_REGISTRY_URL` | `https://...` | `[registry] url` |
| `GREL_ELF_AUTO_RESOLVE_SYSTEM_DEPS` | `false` | `[elf_deps] auto_resolve_system_deps` |
| `GREL_ELF_SHOW_PARSED_DEPS` | `false` | `[elf_deps] show_parsed_deps` |

Lists and maps are supported via Figment's env syntax (consult Figment docs for complex types).

---

## Sections Reference

### `[general]`

| Key | Default | Description |
|-----|---------|-------------|
| `version` | `1` | Config schema version. Must match the compiled version. |
| `max_concurrent` | `4` | Parallel downloads. `0` = auto (`num_cpus / 2`, min `2`). |
| `proxy` | `""` | HTTP/HTTPS proxy. Empty string triggers auto-detection from `http_proxy` / `all_proxy`. |
| `keep_archives` | `true` | Retain downloaded archives in `install_root` after extraction. |

### `[assets]`

| Key | Default | Description |
|-----|---------|-------------|
| `default_selection_policy` | `"first"` | Tie-breaker after filtering: `"first"` or `"largest"`. |
| `exclude_keywords` | `["setup", "installer", "bundle", "nupkg", ...]` | Reject assets whose filenames contain these keywords. |
| `ignore_formats` | `["*.deb", "*.rpm", "*.msi", ...]` | Reject assets matching these globs. |
| `prefer_formats` | `["*.tar.gz", "*.tar.xz", "*.zip", "*.exe"]` | Sort priority for these formats. |
| `prefer_32bit_on_64bit` | `false` | Prefer 32-bit assets when running on 64-bit OS. |
| `fallback_to_32bit` | `true` | Allow 32-bit install if no 64-bit asset exists. |
| `prefer_musl` | `false` | Prefer musl-linked binaries on Linux. |

### `[paths]`

| Key | Default | Description |
|-----|---------|-------------|
| `install_root` | `~/.local/share/grel` (Linux) / `%LOCALAPPDATA%\grel` (Windows) | Root for managed package storage. |
| `bin_dir` | `<install_root>/bin` | Symlinks/copies of extracted binaries. Add this to `PATH`. |
| `download_dir` | `~/Downloads` or system default | Where unmanaged packages (installers, etc.) are saved. |

### `[upgrade]`

| Key | Default | Description |
|-----|---------|-------------|
| `check_interval_hours` | `6` | Minimum hours between remote version checks for a given package. |
| `max_parallel_checks` | `10` | Concurrent API requests during `grel -Syu`. |

### `[auth]`

| Key | Default | Description |
|-----|---------|-------------|
| `github_token` | `""` | GitHub personal access token (increases rate limit to 5000/hr). |
| `gitlab_token` | `""` | GitLab token. |
| `gitea_token` | `""` | Gitea token. |

### `[security]`

| Key | Default | Description |
|-----|---------|-------------|
| `verify_signatures` | `false` | If `true`, requires a `.sig` or `.asc` sidecar file to exist in the release. **Note:** This checks for file presence only; it does not cryptographically verify signatures. |
| `enable_hooks` | `false` | If `true`, allows manifest `post_install` and `pre_remove` hooks to execute. **Disabled by default** for security — only enable if you trust the packages you install. |

### `[registry]`

| Key | Default | Description |
|-----|---------|-------------|
| `url` | `https://github.com/grel-registry/packages` | Central registry URL for manifest lookups. |
| `auto_update` | `true` | Auto-fetch registry updates. |

### `[elf_deps]` (Linux only)

| Key | Default | Description |
|-----|---------|-------------|
| `auto_resolve_system_deps` | `true` | After installing a Linux binary, parse ELF `DT_NEEDED` and suggest system packages to install. |
| `show_parsed_deps` | `true` | Print discovered libraries to the terminal. |
| `distro_override` | `null` | Force a specific distro ID (e.g., `"debian"`, `"fedora"`). |
| `install_cmd_template` | `null` | Custom install command template. |
| `library_map` | `{}` | Global `lib_name → package_name` overrides. |
| `distro_library_map` | `{}` | Per-distro `lib_name → package_name` overrides. |

---

## Platform Paths

`grel` uses `directories::ProjectDirs` to resolve default paths:

| OS | `install_root` | `bin_dir` |
|----|----------------|-----------|
| Linux | `~/.local/share/grel` | `~/.local/share/grel/bin` |
| macOS | `~/Library/Application Support/grel` | `~/Library/Application Support/grel/bin` |
| Windows | `%LOCALAPPDATA%\grel` | `%LOCALAPPDATA%\grel\bin` |

---

## Schema Version

The config file must declare `version = 1` under `[general]`. If the version does not match the compiled schema version, `grel` will refuse to start with an error. This ensures forward compatibility as the config format evolves.
