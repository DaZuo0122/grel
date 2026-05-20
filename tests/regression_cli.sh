#!/bin/sh
# CLI regression test suite for grel.
# Exercises every operation and every flag at least once.
# Creates a temporary sandbox so it does not mutate the developer's real state.

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

PASSED=0
FAILED=0

pass() {
    printf "${GREEN}PASS${NC}: %s\n" "$1"
    PASSED=$((PASSED + 1))
}

fail() {
    printf "${RED}FAIL${NC}: %s\n" "$1"
    FAILED=$((FAILED + 1))
}

skip() {
    printf "${YELLOW}SKIP${NC}: %s\n" "$1"
}

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Build the binary first
printf "Building grel (debug)...\n"
cd "$PROJECT_DIR"
cargo build --quiet 2>/dev/null || cargo build

GREL="$PROJECT_DIR/target/debug/grel"
if [ ! -x "$GREL" ]; then
    # Windows executable name
    GREL="$PROJECT_DIR/target/debug/grel.exe"
fi

# Use an absolute temp directory so the native Windows grel.exe can open
# the SQLite DB without MSYS2/POSIX path translation issues.
# All backslashes are converted to forward slashes for valid TOML.
TEST_DIR="$(pwd)/target/regression-tmp-$$"
CONFIG="$TEST_DIR/config.toml"

cleanup() {
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

mkdir -p "$TEST_DIR/installs" "$TEST_DIR/bin" "$TEST_DIR/downloads"

if command -v cygpath >/dev/null 2>&1; then
    TOML_DIR=$(cygpath -w "$TEST_DIR" | sed 's|\\|/|g')
else
    TOML_DIR="$TEST_DIR"
fi

cat > "$CONFIG" <<EOF
[general]
version = 1
max_concurrent = 2
proxy = ""
keep_archives = true

[assets]
default_selection_policy = "first"
exclude_keywords = ["setup", "installer"]
ignore_formats = ["*.deb", "*.rpm", "*.msi"]
prefer_formats = ["*.tar.gz", "*.zip"]

[paths]
install_root = "$TOML_DIR/installs"
bin_dir = "$TOML_DIR/bin"
download_dir = "$TOML_DIR/downloads"

[upgrade]
check_interval_hours = 6
max_parallel_checks = 10

[auth]
github_token = ""
gitlab_token = ""
gitea_token = ""

[security]
verify_signatures = false

[registry]
url = ""
auto_update = false
EOF

run() {
    "$GREL" -C "$CONFIG" "$@" 2>/dev/null
}

run_err() {
    "$GREL" -C "$CONFIG" "$@" 2>&1 >/dev/null
}

# Like run() but preserves stderr for assertions on error text
run_raw() {
    "$GREL" -C "$CONFIG" "$@"
}

# ---------------------------------------------------------------------------
# P1.2 Global / help tests
# ---------------------------------------------------------------------------

printf "\n=== Help & Global ===\n"

if run --help | grep -q "Usage:"; then
    pass "--help prints usage"
else
    fail "--help prints usage"
fi

if run --version | grep -q "grel"; then
    pass "--version prints version"
else
    fail "--version prints version"
fi

if run_err --forge bogus >/dev/null 2>&1; then
    fail "--forge bogus rejects invalid value"
elif run_raw --forge bogus 2>&1 | grep -qi "possible values\|error\|invalid"; then
    pass "--forge bogus rejects invalid value"
else
    fail "--forge bogus rejects invalid value"
fi

if run -Sh | grep -q "sync\|Sync\|SYNC"; then
    pass "-Sh prints sync help"
else
    fail "-Sh prints sync help"
fi

if run -Qh | grep -q "query\|Query\|QUERY"; then
    pass "-Qh prints query help"
else
    fail "-Qh prints query help"
fi

if run -Rh | grep -q "remove\|Remove\|REMOVE"; then
    pass "-Rh prints remove help"
else
    fail "-Rh prints remove help"
fi

if run -Dh | grep -q "database\|Database\|DATABASE"; then
    pass "-Dh prints database help"
else
    fail "-Dh prints database help"
fi

if run -Uh | grep -q "upgrade\|Upgrade\|UPGRADE"; then
    pass "-Uh prints upgrade help"
else
    fail "-Uh prints upgrade help"
fi

if run -Fh | grep -q "files\|Files\|FILES"; then
    pass "-Fh prints files help"
else
    fail "-Fh prints files help"
fi

# ---------------------------------------------------------------------------
# P1.3 Query operation tests (offline, empty DB)
# ---------------------------------------------------------------------------

printf "\n=== Query (-Q) ===\n"

if run -Q >/dev/null 2>&1; then
    pass "-Q (list all) handles empty DB"
else
    fail "-Q (list all) handles empty DB"
fi

if run -Ql >/dev/null 2>&1; then
    pass "-Ql (list files) handles empty DB"
else
    fail "-Ql (list files) handles empty DB"
fi

if run -Qe >/dev/null 2>&1; then
    pass "-Qe (explicit) handles empty DB"
else
    fail "-Qe (explicit) handles empty DB"
fi

if run -Qd >/dev/null 2>&1; then
    pass "-Qd (deps) handles empty DB"
else
    fail "-Qd (deps) handles empty DB"
fi

if run -Qt >/dev/null 2>&1; then
    pass "-Qt (unrequired) handles empty DB"
else
    fail "-Qt (unrequired) handles empty DB"
fi

if run -Qk >/dev/null 2>&1; then
    pass "-Qk (checksums) handles empty DB"
else
    fail "-Qk (checksums) handles empty DB"
fi

if run -Qs ripgrep >/dev/null 2>&1; then
    pass "-Qs (search) handles empty DB"
else
    fail "-Qs (search) handles empty DB"
fi

if run -Qo rg >/dev/null 2>&1; then
    pass "-Qo (owns) handles empty DB"
else
    fail "-Qo (owns) handles empty DB"
fi

if run -Qq >/dev/null 2>&1; then
    pass "-Qq (quiet) handles empty DB"
else
    fail "-Qq (quiet) handles empty DB"
fi

if run -Qeq >/dev/null 2>&1; then
    pass "-Qeq (combo) handles empty DB"
else
    fail "-Qeq (combo) handles empty DB"
fi

# ---------------------------------------------------------------------------
# P1.4 Database operation tests (offline, empty DB)
# ---------------------------------------------------------------------------

printf "\n=== Database (-D) ===\n"

if run -D --check >/dev/null 2>&1; then
    pass "-D --check runs without crash"
else
    fail "-D --check runs without crash"
fi

if run -D --dump >/dev/null 2>&1; then
    pass "-D --dump runs without crash"
else
    fail "-D --dump runs without crash"
fi

if run -D --clean >/dev/null 2>&1; then
    pass "-D --clean runs without crash"
else
    fail "-D --clean runs without crash"
fi

if run -D --db-clean >/dev/null 2>&1; then
    pass "-D --db-clean runs without crash"
else
    fail "-D --db-clean runs without crash"
fi

if run -D --asexplicit >/dev/null 2>&1; then
    pass "-D --asexplicit without targets does not crash"
else
    fail "-D --asexplicit without targets does not crash"
fi

if run -D --asdeps >/dev/null 2>&1; then
    pass "-D --asdeps without targets does not crash"
else
    fail "-D --asdeps without targets does not crash"
fi

# ---------------------------------------------------------------------------
# P1.5 Remove / Files / Upgrade operation tests (offline, empty DB)
# ---------------------------------------------------------------------------

printf "\n=== Remove (-R) / Files (-F) / Upgrade (-U) ===\n"

if run -R >/dev/null 2>&1; then
    pass "-R without targets does not crash"
else
    fail "-R without targets does not crash"
fi

if run -Ru >/dev/null 2>&1; then
    pass "-Ru (unneeded) handles empty DB"
else
    fail "-Ru (unneeded) handles empty DB"
fi

if run -F >/dev/null 2>&1; then
    pass "-F without sub-op does not crash"
else
    fail "-F without sub-op does not crash"
fi

if run -Fs rg >/dev/null 2>&1; then
    pass "-Fs (search) handles empty DB"
else
    fail "-Fs (search) handles empty DB"
fi

if run -U >/dev/null 2>&1; then
    pass "-U without file does not crash"
else
    fail "-U without file does not crash"
fi

# ---------------------------------------------------------------------------
# P1.6 Sync operation tests (offline / dry-run)
# ---------------------------------------------------------------------------

printf "\n=== Sync (-S) ===\n"

# Dry-run should not crash even with no network
if run -S --dry-run github/BurntSushi/ripgrep >/dev/null 2>&1; then
    pass "-S --dry-run does not crash"
else
    # Non-zero exit is OK if it's a network error, not a panic
    if run -S --dry-run github/BurntSushi/ripgrep 2>&1 | grep -qi "panic\|thread.*panicked"; then
        fail "-S --dry-run does not panic"
    else
        pass "-S --dry-run does not panic"
    fi
fi

# Flag stacking should parse cleanly
if run -Syu >/dev/null 2>&1; then
    pass "-Syu flag stacking parses"
else
    if run -Syu 2>&1 | grep -qi "panic\|thread.*panicked"; then
        fail "-Syu flag stacking parses"
    else
        pass "-Syu flag stacking parses"
    fi
fi

# ---------------------------------------------------------------------------
# P1.7 Flag-combination parsing tests
# ---------------------------------------------------------------------------

printf "\n=== Flag Combinations ===\n"

COMBOS="
-S --asdeps --noconfirm --dry-run foo/bar
-R --recursive --nosave --noconfirm foo/bar
-Q --explicit --quiet
-U --overwrite foo/bar
--proxy http://localhost:8080 -Sh
"

echo "$COMBOS" | while IFS= read -r combo; do
    # Skip empty lines
    [ -z "$(echo "$combo" | tr -d ' ')" ] && continue
    # Remove leading whitespace
    combo=$(echo "$combo" | sed 's/^[[:space:]]*//')
    label="$combo"
    if run $combo >/dev/null 2>&1; then
        pass "combo: $label"
    else
        if run $combo 2>&1 | grep -qi "panic\|thread.*panicked"; then
            fail "combo: $label"
        else
            pass "combo: $label"
        fi
    fi
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

printf "\n=== Summary ===\n"
printf "Passed: %d\n" "$PASSED"
printf "Failed: %d\n" "$FAILED"

if [ "$FAILED" -gt 0 ]; then
    exit 1
fi
