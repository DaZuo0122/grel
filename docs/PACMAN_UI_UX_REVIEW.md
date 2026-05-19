> **Note:** This is a historical review from 2026-05-16. Many issues identified here have since been addressed. For current behavior, see [COMMANDS.md](COMMANDS.md) and [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

# Current CLI pacman Command Style & UI/UX Review
Date: 2026-05-16
Branch: `codex-test-automation-just`
Review Objective: Determine whether the current `grel` CLI's "command structure, help system, interaction style, and output tone" align with pacman user expectations.

## Summary Conclusion
**Verdict: `Partially close to pacman, but overall still not quite like pacman`**

The current implementation has the skeleton of a pacman-style CLI:
- Uses `-S/-Q/-R/-D/-U/-F` as operations
- Supports flag stacking (e.g., `-Syu`)
- Most core actions are designed around pacman command semantics

However, several CLI details and UX elements still break pacman users' muscle memory, particularly:
- Help system is not operation-scoped
- Multiple long option names deviate from pacman conventions
- Some actual flags contradict their help text
- Help output contains numerous empty or placeholder descriptions
- Output copy resembles "generic tool prompts" rather than pacman's unified, restrained, and predictable terminal tone

## 1. Command Structure: Does it feel like pacman?
### What's done well
| Item | Evaluation |
| --- | --- |
| `-S/-Q/-R/-D/-U/-F` | Correct direction, matches pacman expectations |
| Flag stacking like `-Syu` | Aligns with pacman muscle memory |
| Query semantics like `-Qe/-Qd/-Qt` | Correct direction |
| `-c/-n/-u` under `-R` | Naming direction is generally correct |

### What doesn't feel like pacman
| Issue | Evidence | Impact |
| --- | --- | --- |
| Recursive removal uses `-r`, not `-s` as in docs/pacman convention | `crates/grel-cli/src/commands.rs:223-228` | Directly breaks muscle memory |
| Long option `--deps-filter` doesn't match pacman's `--deps` | `crates/grel-cli/src/commands.rs:189` | Unnatural style, inconsistent with docs |
| `-D` actually uses `--db-clean/--db-check/--db-dump`, not `--clean/--check/--dump` | `crates/grel-cli/src/commands.rs:251-257` | Forces users to memorize non-pacman names |
| `-U` path argument is `--local-asset`, not the more natural `--asset <path>` per docs/pacman context | `crates/grel-cli/src/commands.rs:263` | Naming feels implementation-driven rather than user-driven |

## 2. Help System: Does it feel like pacman?
**Verdict: `No`**

### 2.1 `-Sh` / `-Qh` do not provide scoped help
Actual run results:
- `cargo run -- -Sh`
- `cargo run -- -Qh`
Both commands return the same global help instead of operation-specific help.

**Why this matters:**
- Pacman users expect `-Sh` for sync help and `-Qh` for query help.
- `grel` currently encourages this in its prompts, but the actual behavior doesn't deliver.

### 2.2 Help output is too "flat"
`cargo run -- --help` exposes a single global options table, flattening flags from all operations.
This causes two problems:
1. Users struggle to quickly establish which flag belongs to which operation.
2. When identical short flags carry different meanings across operations, the help becomes confusing.
Pacman's UX typically favors:
- A very short global entry point
- Clearer operation-level help
- No need for users to manually parse semantics from a massive table

## 3. Help Text Quality Assessment
**Verdict: `Currently low quality`**

### 3.1 Multiple flags lack real user descriptions
In the actual `--help` output, the following items show "empty descriptions" or just repeat the flag name:
`--db-clean`, `--db-check`, `--db-dump`, `--local-asset`, `--dry-run`, `--noconfirm`, `--overwrite`, `--asset`, `--platform`, `--exclude-keywords`, `--allow-keyword`, `--allow-format`, `--proxy`
Such help text provides almost zero informational value to end users.

### 3.2 Self-contradictions within the same help output
Evidence:
- `-R` long help at `crates/grel-cli/src/commands.rs:73-75` still says `-s (recursive)`
- Actual flag is `-r, --recursive`
Impact:
- Users cannot fully trust the help output, significantly damaging CLI credibility.

## 4. Output & Interaction Tone: Does it feel like pacman?
**Verdict: `Only partially`**

### Closer to pacman
- Pure text terminal interaction
- Uses confirmation prompts instead of complex TUI
- Includes common CLI structures like "checking / cleaning / upgrade summary"

### Not like pacman
| Item | Current State | Why it doesn't feel like pacman |
| --- | --- | --- |
| Download selection prompt | `Proceed with download? [1-n, s=skip]` | Pacman favors unified, restrained, low-branching confirmation prompts |
| Explanatory copy | `Other compatible assets:`, `Selected:`, `Install your first package with:` | Feels like beginner CLI onboarding, not pacman's concise style |
| Warning copy | Heavy use of `Warning:`, `Info:` prefixes | Pacman typically uses fixed styles like `warning:` / `error:` / `::`, not app-level custom phrasing |
| Color/label rendering | Lists print literal `[green]` / `[yellow]` | Resembles unfinished UI placeholders rather than a mature CLI style |

Evidence:
- `src/commands/query.rs:65-79` defines `status_icon` as strings `"green" / "yellow"`, which are directly formatted into literals like `[green]`

## 5. Core Gaps Between Current CLI and pacman Style
### 5.1 Command names look like pacman, but details lack "strictness"
The biggest issue isn't that `grel` "doesn't look like pacman", but rather:
- Visually, it's very close
- In details, it's not strict enough
This creates stronger dissonance because users assume it follows pacman rules, only to trip over details.

### 5.2 Documentation, help, and actual implementation are not yet aligned
There are currently at least three "sources of truth":
1. `docs/COMMANDS_DESIGN.md`
2. `cargo run -- --help`
3. Actual clap/handler routing
These three are not fully consistent. For a CLI aiming for pacman-compatible UX, this is highly damaging.

## 6. Style Rating
Using a 4-tier scale:
- `A`: Highly aligned with pacman; veteran users barely need to relearn
- `B`: Generally aligned, with minor inconsistencies
- `C`: Only borrows pacman operation letters; details and experience haven't caught up
- `D`: Essentially not pacman-style

**Current Rating: `C`**
Reasoning:
- Operation letters and basic combinations already resemble pacman
- However, help system, long flag naming, operation-level help, prompt copy, and documentation consistency have not reached the point where pacman users can "migrate seamlessly".

## 7. Highest Priority Improvement Recommendations
**Priority 1: Unify the "external command surface"**
Align these three first:
- `docs/COMMANDS_DESIGN.md`
- Clap-exposed `--help`
- Actual handler routing
Do not expand the CLI feature surface until this is complete.

**Priority 2: Make help operation-scoped**
Goal:
- `grel -Sh` shows only sync-related help
- `grel -Qh` shows only query-related help
- Same for `grel -Rh`, `grel -Dh`, etc.

**Priority 3: Revert flag naming to pacman-natural forms**
Fix first:
- `--db-clean` -> `--clean`
- `--db-check` -> `--check`
- `--db-dump` -> `--dump`
- `--deps-filter` -> `--deps`
- `--local-asset` -> Name closer to docs/semantics
- Unify recursive remove short flag and help text

**Priority 4: Clean up help text**
Goal:
- Every option must have a real user-facing description
- Eliminate placeholder outputs like "`--dry-run` describes itself as `--dry-run`"
- Remove contradictions with documentation

**Priority 5: Unify output tone**
Recommended direction:
- Fewer "tutorial-style" sentences
- More consistent `warning:` / `error:` / `::` styling
- Avoid debug artifacts like literal `[green]`
- Compress interaction prompts into shorter, more stable terminal phrasing

## Final Conclusion
The current `grel` has the shell of a pacman-style CLI, but lacks the "discipline" of one.
If the goal is merely to "look like pacman", it's mostly there.
If the goal is to "let pacman users operate it without breaking immersion", it still needs to tighten:
- Consistent external command surface
- Operation-scoped help
- Strict flag naming alignment
- Complete, conflict-free help copy
- Output tone unified with pacman CLI conventions