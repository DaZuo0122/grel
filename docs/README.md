# grel Documentation

Welcome to the `grel` documentation. This folder contains everything you need to get started, use, and contribute to `grel`.

## For Users

| Document | What you'll learn |
|----------|-------------------|
| [QUICK_START.md](QUICK_START.md) | Install `grel`, configure it, and run your first commands |
| [COMMANDS.md](COMMANDS.md) | Complete, accurate reference for every CLI operation and flag |
| [CONFIGURATION.md](CONFIGURATION.md) | How to configure `grel` via TOML files and environment variables |

## For Contributors

| Document | What you'll learn |
|----------|-------------------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Workspace layout, crate responsibilities, data flows, and database schema |
| [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) | What is implemented, what is stubbed, and known limitations |

## Historical / Design Documents

The following documents capture design decisions, audits, and gap analyses from earlier development phases. They are kept for historical context but may contain outdated information. For current behavior, refer to the user and contributor guides above.

- `COMMANDS_DESIGN.md` — Original command design (superseded by [COMMANDS.md](COMMANDS.md))
- `TECHNICAL_DESIGN.md` — Original technical design (superseded by [ARCHITECTURE.md](ARCHITECTURE.md))
- `ELF_DEPENDENCY_AUTO_RESOLUTION_DESIGN.md` — Design of the Linux ELF system-dependency resolver
- `PACMAN_COMPATIBILITY_AUDIT.md` — Compatibility audit against pacman conventions
- `PACMAN_UI_UX_REVIEW.md` — UI/UX review against pacman style
- `CLI_COMPATIBILITY_FIX_PLAN.md` — Original plan for fixing CLI gaps
- `CLI_IMPLEMENTATION_GAP_REPORT.md` — Detailed gap report for CLI flags
- `CLI_REGRESSION_REVIEW.md` — Regression review from a specific development branch
- `PRODUCTION_GAP_ANALYSIS.md` — Strategic analysis of production-readiness gaps
