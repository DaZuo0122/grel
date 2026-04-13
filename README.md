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
git clone https://github.com/grel-rs/grel-rs.git
cd grel-rs
cargo install --path .
```

### Pre-built Binaries

Download from [Releases](https://github.com/grel-rs/grel-rs/releases):

```bash
# Linux
grel sync github/grel-rs/grel-rs

# Windows (PowerShell)
grel.exe sync github/grel-rs/grel-rs
```

## Quick Start

```bash
# Install your first package
grel sync github/BurntSushi/ripgrep

# Upgrade all packages
grel -Syu

# List installed packages
grel list

# Remove a package
grel remove github/BurntSushi/ripgrep
```

## Configuration

Configuration is stored in `~/.config/grel/config.toml` (Linux) or `%APPDATA%\grel\config.toml` (Windows).

```toml
[general]
max_concurrent = 4
proxy = ""

[assets]
default_selection_policy = "first"
exclude_keywords = ["setup", "installer", "bundle", "nupkg"]
ignore_formats = ["*.deb", "*.rpm", "*.msi", "*.dmg"]
prefer_formats = ["*.tar.gz", "*.tar.xz", "*.zip", "*.exe"]
fallback_to_32bit = true

[paths]
install_root = "~/.local/share/grel"
bin_dir = "~/.local/share/grel/bin"
download_dir = "~/Downloads"

[upgrade]
check_interval_hours = 6
max_parallel_checks = 10
```

## Usage

### Sync Packages

```bash
# Install a package
grel sync github/cli/cli

# Install with custom output directory
grel sync github/BurntSushi/ripgrep -O /usr/local/bin

# Allow keyword-matching assets
grel sync github/foo/bar --allow-keyword
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
grel list

# Shows active and orphaned packages
```

### Remove Packages

```bash
grel remove github/cli/cli
```

### PATH Setup

```bash
# Show PATH configuration snippets
grel path add
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
