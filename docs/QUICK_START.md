# Quick Start Guide

Get `grel` installed and managing packages in under five minutes.

---

## Installation

### From Source

Requires Rust **1.85+**.

```bash
git clone https://github.com/DaZuo0122/grel.git
cd grel
cargo install --path .
```

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/DaZuo0122/grel/releases) and place the binary somewhere on your `PATH`.

---

## First-Time Setup

### 1. Add `grel` binaries to your PATH

`grel` installs packages to a user-local directory. You need to add its `bin` folder to your `PATH`.

**Linux / macOS:**

```bash
export PATH="$HOME/.local/share/grel/bin:$PATH"
```

Add the line to `~/.bashrc` (or `~/.zshrc`) to make it permanent.

**Windows (PowerShell):**

```powershell
$bin = "$env:LOCALAPPDATA\grel\bin"
[Environment]::SetEnvironmentVariable("Path", "$bin;$env:Path", "User")
```

### 2. Create a configuration file (optional)

`grel` works out of the box with compiled-in defaults. If you want to customize behavior, create a config file and pass it with `-C`:

```toml
# ~/.config/grel/config.toml
[general]
version = 1
max_concurrent = 4

[assets]
exclude_keywords = ["setup", "installer", "bundle"]
ignore_formats = ["*.deb", "*.rpm", "*.msi"]

[auth]
github_token = "ghp_xxxxxxxx"
```

> **Note:** `grel` does **not** auto-discover a config file. You must pass it explicitly:
> ```bash
> grel -C ~/.config/grel/config.toml -S foo/bar
> ```
> Alternatively, use `GREL_*` environment variables (e.g., `GREL_GENERAL_PROXY=http://proxy:8080`).

---

## Basic Workflow

### Install a package

```bash
# Install latest release from GitHub (default forge)
grel -S sharkdp/fd

# Install from a specific forge
grel -S github/sharkdp/fd

# Pin to a version
grel -S sharkdp/fd@v10.1.0
```

`grel` will:
1. Fetch the release metadata
2. Pick the best asset for your OS/arch
3. Download and extract it
4. Link binaries into `~/.local/share/grel/bin/`
5. Record the installation in its local SQLite database

### List installed packages

```bash
grel -Q
```

### Upgrade everything

```bash
# Refresh metadata, then upgrade
grel -Syu

# Non-interactive (CI-friendly)
grel -Syu --noconfirm
```

### Remove a package

```bash
grel -R sharkdp/fd
```

### Search for packages

```bash
# Search GitHub
grel -Ss ripgrep

# Search installed packages locally
grel -Qs ripgrep
```

### Check what you have

```bash
# Show info about an installed package
grel -Qi sharkdp/fd

# Verify downloaded archive checksums
grel -Qk

# Find which package owns a binary
grel -Qo fd
```

### Database housekeeping

```bash
# Clean orphaned records and stale caches
grel -D --clean

# Verify database integrity
grel -D --check
```

---

## Common Options

| Flag | Description |
|------|-------------|
| `--dry-run` | Simulate the operation without changing anything |
| `--noconfirm` | Skip all interactive prompts (auto-accept) |
| `--overwrite` | Replace existing binaries if they conflict |
| `--allow-keyword` | Allow assets that match excluded keywords (e.g., `setup`) |
| `--asset <NAME>` | Download a specific asset file instead of auto-selecting |

---

## Troubleshooting

**"No compatible assets found"**
- Check that the release has assets for your platform.
- Use `--platform linux/x86_64` (or your platform) to override auto-detection.
- Use `--allow-format *.AppImage` if the asset is in an ignored format.

**Rate limiting from GitHub**
- Set a GitHub token in config (`github_token`) or via `GREL_AUTH_GITHUB_TOKEN`.

**Proxy issues**
- `grel` auto-detects `http_proxy` / `all_proxy` environment variables.
- Override with `--proxy http://proxy:8080` or `GREL_GENERAL_PROXY`.

---

## Next Steps

- Read the full [Command Reference](COMMANDS.md)
- Read the [Configuration Guide](CONFIGURATION.md)
- Read the [Architecture Overview](ARCHITECTURE.md) if you want to contribute
