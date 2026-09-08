# Fitness functions

A fitness function is an automated check that fails CI (and `just check`, once the workspace exists) when an invariant is violated. Prose is not a fitness function.

Accepted architecture ADRs that can be checked by a machine MUST have a row here. Process and toolchain hygiene (commit messages, `cargo deny`) may also have rows; they do not require an ADR. Until the check exists, the row stays `[UNTESTED]`.

## Layer graph

Enforced by the Cargo workspace as soon as crates exist. Clippy and `cargo deny` are the second belt.

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F1 | `workengine-domain` does not depend on application, adapters, or cli | Cargo.toml graph / `cargo check --workspace` | [UNTESTED] |
| F2 | `workengine-application` does not depend on adapters or cli | Cargo.toml graph | [UNTESTED] |
| F3 | `workengine-domain` does not use filesystem, process, or network APIs | clippy `disallowed_methods` / `disallowed_types` | [UNTESTED] |
| F4 | `workengine-domain` contains no product or tracker names | review + grep in architecture tests when they exist | [UNTESTED] |
| F5 | `workengine-application` does not spawn processes or open sockets | clippy + crate graph | [UNTESTED] |

## Domain behaviour

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F6 | Illegal transition is a domain error, not a coerced status | domain unit test | [UNTESTED] |
| F7 | `next`, `start`, `complete`, and `park` are idempotent | domain / application test | [UNTESTED] |
| F8 | Outcome kinds are a closed enum; unknown kind is a schema error | parse test | [UNTESTED] |
| F9 | FSM `match` is exhaustive | `cargo test` / compiler | [UNTESTED] |
| F10 | First-slice Work statuses are `ready`, `running`, `succeeded`, `failed`, `parked` | domain enum test | [UNTESTED] |

## Store

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F11 | Status update and event append are atomic | store test | [UNTESTED] |
| F12 | Replay reconstructs status | store test | [UNTESTED] |

## Worker and workspace

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F13 | Timeout kills the process group, not only the parent | worker adapter test | [UNTESTED] |
| F14 | Two Work ids never share a workspace root | workspace test | [UNTESTED] |
| F15 | Kill/restart mid-`complete` resumes; second `complete` is idempotent | durable-execution test on stub Worker | [UNTESTED] |

## Product contracts

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F16 | Long-lived artifacts carry `schemaVersion` | schema tests | [UNTESTED] |
| F17 | Exit codes are a published table, not a single non-zero | CLI tests | [UNTESTED] |

## Process and hygiene

These are not SemVer surfaces and not architecture. Canon for commits is `CONTRIBUTING.md`.

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F18 | Commits on the default branch match Conventional Commits | commit-msg hook + CI range check | [UNTESTED] |
| F19 | `cargo deny check` is part of `just check` | `just check` | [UNTESTED] |

## Operator, memory, and observation

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F20 | `park` is not abort: save point and free slot, without process-group kill as the park path | worker / application test | [UNTESTED] |
| F21 | Workspace has append-only memory; write only after a confirmed outcome | workspace test | [UNTESTED] |
| F22 | Supervised subprocess output uses one streaming format with `work_id` | CLI / runner tests | [UNTESTED] |

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
