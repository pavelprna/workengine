# Fitness functions

A fitness function is an automated check that fails CI and `just check` when an invariant is violated. Prose is not a fitness function.

Accepted architecture ADRs that can be checked by a machine MUST have a row here. Process and toolchain hygiene (commit messages, `cargo deny`) may also have rows; they do not require an ADR. Until the check exists, the row stays `[UNTESTED]`.

## Layer graph

Enforced by the Cargo workspace as soon as crates exist. Clippy and `cargo deny` are the second belt.

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F1 | `workengine-domain` does not depend on application, adapters, or cli | Cargo.toml graph / `cargo check --workspace` | enforced |
| F2 | `workengine-application` does not depend on adapters or cli | Cargo.toml graph | enforced |
| F3 | `workengine-domain` does not use filesystem, process, network, environment, or thread APIs | `crates/domain/clippy.toml` / clippy `disallowed_methods` / `disallowed_types` | enforced |
| F4 | `workengine-domain` contains no product or tracker names | `crates/cli/tests/no_product_names.rs` | enforced |
| F5 | `workengine-application` does not spawn processes, open sockets, or touch the filesystem | `crates/application/clippy.toml` / clippy + crate graph | enforced |

## Domain behaviour

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F6 | Illegal transition is a domain error, not a coerced status | domain unit test | enforced |
| F7 | Repeating `next`, `complete`, and `park` does not duplicate effects; repeating `start` does not spawn twice | domain / application test | enforced |
| F8 | Outcome kinds are a closed enum; unknown kind is a schema error | parse test | enforced |
| F9 | FSM `match` is exhaustive | `cargo test` / compiler | enforced |
| F10 | First-slice Work statuses are `ready`, `running`, `succeeded`, `failed`, `parked` | domain enum test | enforced |

## Store

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F11 | Status update and event append are atomic | store test | enforced |
| F12 | Replay reconstructs status | store test | enforced |
| F23 | Two CLI processes MUST NOT share a data directory: exclusive lock, second open is a store conflict | store test + CLI test (Unix) | enforced |

## Worker and workspace

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F13 | Timeout kills the process group, not only the parent | worker adapter test | enforced |
| F14 | Two Work ids never share a workspace root | workspace test | enforced |
| F15 | Leftover `running` parks; `complete --file` applies to leftover `running` or `parked`; start with an outcome artifact does not spawn; second `complete` is idempotent | durable-execution test on stub Worker | enforced |

## Product contracts

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F16 | Long-lived artifacts carry `schemaVersion`; `schemas/*.json` enums match the domain | schema file tests + parse tests | enforced |
| F17 | Exit codes are a published table, not a single non-zero | CLI tests | enforced |

## Process and hygiene

These are not SemVer surfaces and not architecture. Canon for commits is `CONTRIBUTING.md`.

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F18 | Commits on the default branch match Conventional Commits | `scripts/commit-msg` + `just commits` in CI | enforced |
| F19 | `cargo deny check` is part of `just check` | `just check` | enforced |

## Operator, memory, and observation

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F20 | `park` is not abort: save point and free slot, without process-group kill as the park path | worker / application test | enforced |
| F21 | Workspace has append-only memory; write only after a confirmed outcome; repeat complete does not duplicate the last line | workspace / application test | enforced |
| F24 | Channel errors are classified as retry, fail, or park; fail completes as `channel_error`; park does not complete | application test | enforced |
| F25 | Process Worker env is empty except declared references; missing `outcome.json` is `failed`, not exit-code success | worker adapter test | enforced |

## How to add a check

1. If an architecture ADR is `Accepted` and checkable, add a row here in the same change.
2. Process or toolchain checks come from `CONTRIBUTING.md` / `just check`, not from an ADR.
3. Implement the check in the workspace (`clippy.toml`, a test, or `cargo deny`).
4. Flip `[UNTESTED]` only when the check actually fails on a violation.

Do not add a markdown rule instead of a failing check.

## Related

- Overview: [overview.md](overview.md)
- ADRs: [../adr/INDEX.md](../adr/INDEX.md)
- Specs: [../spec/workflow.md](../spec/workflow.md), statuses in [../spec/work.md](../spec/work.md)
- Commits (F18): [../../CONTRIBUTING.md](../../CONTRIBUTING.md)
