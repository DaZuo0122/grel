# grel - A Package Manager for Pre-built Binaries

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://github.com/grel-rs/grel-rs/actions/workflows/rust.yml/badge.svg)](https://github.com/grel-rs/grel-rs/actions)

**grel** is a terminal-native, high-performance release downloader and package manager for Git forges (GitHub, GitLab, Gitea, Codeberg). It abstracts provider APIs into a unified pipeline, supports proxies, caches DNS/IPs for CDN routing, downloads in parallel, and delivers a transparent, scriptable, pacman-compatible UX.

## Features

- **Deterministic Resolution** - Strict filters → priority sorting → explicit policy fallback
- **Keyword Exclusion** - Configurable `exclude_keywords` to block installer/setup/bundle artifacts
- **Warning System** - Explicit warnings for unmanaged or extra-step packages
- **Cross-Platform** - Linux, Windows (macOS optional)
- **Proxy Support** - Full proxy chain with DNS/IP caching
- **SQLite State** - Robust package tracking with ETag caching

## Installation

### From Source

```bash
git clone https://github.com/DaZuo0122/grel.git
cd grel
cargo install --path .
```

### Pre-built Binaries

Download from [Releases](https://github.com/DaZuo0122/grel/releases):

```bash
# Linux
grel -S github/DaZuo0122/grel

# Windows (PowerShell)
grel.exe -S github/DaZuo0122/grel
```

## Quick Start

```bash
# Install your first package
grel -S github/BurntSushi/ripgrep

# Upgrade all packages
grel -Syu

# List installed packages
grel -Q

# Remove a package
grel -R github/BurntSushi/ripgrep
```

## Configuration

Configuration is stored in `~/.config/grel/config.toml` (Linux) or `%APPDATA%\grel\config.toml` (Windows).

```toml
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
bin_dir = "~/.local/share/grel/bin"   # Extracted binaries (added to $PATH)
download_dir = "~/Downloads"          # Unmanaged/extra-op packages (falls back to system Downloads/)

[upgrade]
check_interval_hours = 6         # Remote check cooldown per package
max_parallel_checks = 10         # Concurrent API requests during `-Syu`

[auth]
github_token = ""                # Use env $GREL_GITHUB_TOKEN instead
gitlab_token = ""                # Use env $GREL_GITLAB_TOKEN instead
gitea_token = ""                 # Use env $GREL_GITEA_TOKEN instead
```

## Usage

### Sync Packages

```bash
# Install a package
grel -S github/cli/cli

# Allow keyword-matching assets
grel -S github/foo/bar --allow-keyword

# Install without confirming asset selection
grel -S github/foo/bar --noconfirm
```

### Upgrade All Packages

```bash
# Check and upgrade all packages
grel -Syu

# Non-interactive mode (auto-accept)
grel -Syu --noconfirm
```

### List Packages

```bash
# Show all installed packages
grel -Q

# Show only explicitly installed packages
grel -Qe

# Show unrequired (orphan) packages
grel -Qt
```

### Remove Packages

```bash
# Remove a package
grel -R github/cli/cli

# Remove and delete config files (--nosave)
grel -Rn github/cli/cli

# Remove with cascade (orphaned deps)
grel -Rc github/cli/cli
```

### Database Operations

```bash
# Mark package as explicitly installed
grel -D --asexplicit github/cli/cli

# Mark package as dependency
grel -D --asdeps github/cli/cli

# Clean stale caches
grel -D --clean

# Check database integrity
grel -D --check
```

## Architecture

The project is organized as a Cargo workspace with 6 crates:

- **`grel-cli`** - CLI parsing and user interaction
- **`grel-core`** - Core resolution logic and asset tokenization
- **`grel-providers`** - Git forge provider adapters
- **`grel-network`** - HTTP client, proxy management, parallel downloads
- **`grel-cache`** - SQLite state management and caching
- **`grel-config`** - Configuration management

## Development

### Building

```bash
cargo build
cargo build --release
```

### Testing

```bash
cargo test
cargo test --workspace
```

### Linting

```bash
cargo clippy --workspace
cargo fmt --all -- --check
```

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please open issues and pull requests.
