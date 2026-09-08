# Local interface for the same steps CI will run. Do not put check logic
# only in a git-host YAML file.

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

# Format, lint, test, and cargo-deny. Mandatory before finishing a change.
check: fmt-check clippy test deny

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --locked --workspace --all-targets -- -D warnings

test:
    cargo test --locked --workspace

deny:
    cargo deny check

# Conventional Commits on base..HEAD (F18). CI cannot skip; local hook can.
commits base="":
    ./scripts/check-commits {{base}}

# What CI runs: local check plus the commit-message range.
ci: check commits

# Install the local commit-msg hook (skippable; CI cannot skip).
hooks:
    ln -sfn ../../scripts/commit-msg .git/hooks/commit-msg
