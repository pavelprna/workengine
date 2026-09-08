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

Enforcement: a `commit-msg` hook (shell or Rust, no npm) locally, and the same checker in CI on the branch range. The hook can be skipped locally; CI cannot. This is fitness F18, process hygiene, not a SemVer surface.

## History

The default branch is linear: rebase, then fast-forward or rebase-merge. **Do not squash** onto the default branch. A pull request is a review boundary; commits remain the changelog and revert boundary.

Release versioning (tags `vX.Y.Z`) is a product decision in `docs/product/compatibility.md`, not an automatic side effect of every push.

## Check

Once the workspace lands, the only local interface is `just check` (or `make check` if that is the file we add). Same steps as CI. Do not put logic only in a host's YAML.

Until that target exists, this repository is constitution only: no runtime crate, no invented build tool.

Planned `check` (stage 1): `cargo fmt --check`, `cargo clippy --locked -- -D warnings`, `cargo test --workspace`, `cargo deny check`. Format and lints are not negotiated in review.

## Architecture

If you change a layer, a port, the store, or the Worker model: read `docs/adr/INDEX.md`, add or supersede an ADR, update fitness if the decision is checkable, then change code.

If you change how people commit or rebase: edit this file. Do not add an ADR.

## License

Contributions are Apache-2.0, matching [LICENSE](LICENSE).
