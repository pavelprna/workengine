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
