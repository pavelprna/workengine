# Contributing

## What to change where

One fact has one canonical place. Other files link; they do not retell.

| Concern | Place |
| --- | --- |
| Behaviour | `docs/spec/` |
| Why this shape | `docs/adr/` |
| Layers and ports | `docs/architecture/` |
| Public contracts | `docs/product/compatibility.md` |
| Commit, branch, check | this file |
| Agent entry | `AGENTS.md` only |

`docs/adr/` records decisions about the running system. It does not record how to commit, rebase, or license a change.

Do not add a second agent file (`CLAUDE.md`, `.cursor/rules`, …). If a tool requires one, it must be a single-line pointer at this `AGENTS.md`.

## Commits

This section is the canon. `AGENTS.md` points here.

[Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):

```
type(scope): short imperative summary

optional body: why, not what
```

Types: `feat` `fix` `docs` `test` `refactor` `perf` `ci` `build` `chore`.

Scopes are seams: `domain` `application` `store` `worker` `workspace` `cli` `spec` `adr` `arch`.

Breaking CLI, outcome schema, or store schema: `BREAKING CHANGE:` footer.

A docs-only change is `docs(spec)` or `docs(adr)`, not `feat`.

Atomic means one reason to revert, not one file:

| One commit | Not one commit |
| --- | --- |
| FSM + illegal-transition test | skeleton + linter + README + CLI |
| ADR + the fitness check that enforces it | a feature plus a package rename |
| New port + fake adapter | "also fixed indentation" |

Enforcement: a `commit-msg` hook (shell, no npm) locally, and the same checker in CI on the branch range. Install with `just hooks`. The hook can be skipped locally; CI cannot. This is fitness F18, process hygiene, not a SemVer surface.

## History

The default branch is linear: rebase, then fast-forward or rebase-merge. **Do not squash** onto the default branch. A pull request is a review boundary; commits remain the changelog and revert boundary.

Release versioning is defined in `docs/product/compatibility.md`. Conventional
Commits drive the release proposal; do not manually bump a version or push a
release tag. Review and merge the generated release pull request instead.

## Check

The only local interface is `just check`. Same steps as CI. Do not put logic only in a host's YAML. Format and lints are not negotiated in review.

`just check` runs `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo test --locked --workspace`, and `cargo deny check`.

CI runs `just check` and `just commits` (Conventional Commits on the branch range, fitness F18). Do not put extra check logic only in a git-host YAML file.

`just` and `cargo-deny` must be on `PATH`. With the pinned toolchain:

```
. "$HOME/.cargo/env"
cargo install just cargo-deny --locked
```

## Architecture

If you change a layer, a port, the store, or the Worker model: read `docs/adr/INDEX.md`, add or supersede an ADR, update fitness if the decision is checkable, then change code.

If you change how people commit or rebase: edit this file. Do not add an ADR.

## License

Contributions are Apache-2.0, matching [LICENSE](LICENSE).
