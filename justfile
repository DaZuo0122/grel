# Local test entrypoints for grel.
# Keep CI wiring separate; these recipes are for developer workflows.

set shell := ["sh", "-cu"]

default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

lint: fmt-check clippy

build:
    cargo build --workspace

test-unit:
    cargo test --workspace --lib

test-doc:
    cargo test --workspace --doc

test-cli:
    cargo test --test cli_smoke

test-integration:
    cargo test --test system_dep_cache
    cargo test --test upgrade
    cargo test --test integration
    cargo test -p grel-elf --test integration

test-live:
    cargo test --test integration -- --ignored --nocapture

test-offline: test-unit test-cli test-integration test-regression

test-all: test-offline test-live

verify: lint test-all

test-crate crate:
    cargo test -p {{crate}}

test-filter filter:
    cargo test --workspace {{filter}} -- --nocapture

# Run the full shell-based CLI regression suite
test-regression:
    @sh tests/regression_cli.sh
