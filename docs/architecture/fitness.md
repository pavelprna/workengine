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
| F23 | An independent `create` is not blocked by an unrelated running Work; SQLite uses WAL rather than a data-directory lock | CLI test (Unix) | enforced |

## Worker and workspace

| ID | Invariant | Check | Status |
| --- | --- | --- | --- |
| F13 | Timeout kills the process group, not only the parent | worker adapter test | enforced |
| F14 | Two Work ids never share a workspace root | workspace test | enforced |
| F15 | A legacy shared workspace outcome is rejected and cannot complete Work; leftover `running` parks on recovery | durable-execution test on stub Worker | enforced |

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
| F25 | Process Worker env is empty except secret-file paths; secret values are transient host `0600` files and missing outcome is `failed`, not exit-code success | worker adapter test | enforced |
| F26 | Read observers (`list`, `show`, `events`, HTTP, SSE) do not recover or append a Work event | CLI / HTTP integration test | enforced |
| F27 | Tokio is confined to the HTTP adapter; domain and application stay synchronous | CLI architecture test | enforced |
| F28 | Local HTTP intake can create only `ready` Work through the application use case; it cannot set status or workspace data | HTTP integration test | enforced |
| F29 | HTTP overview is calculated from one read-only snapshot; resumable event cursors are exclusive and `Last-Event-ID` wins over query input | HTTP integration test | enforced |
| F30 | Execution and attempt ids are distinct types; execution specs are immutable, contain references rather than secret values, and confirmed outcomes bind both ids | domain types and unit tests | enforced |
| F31 | CLI and localhost HTTP start use one application path; only one active attempt can claim a Work and the browser cannot provide execution configuration | application, store, HTTP, and CLI tests | enforced |
| F32 | Execution observation is read-only, typed, and finite from Work summary to control-plane proof artifacts | store and HTTP integration tests | enforced |
| F33 | Attempt heartbeat and process records require the matching active lease; child stdout/stderr payloads are never persisted or returned | worker, store, and HTTP integration tests | enforced |
| F34 | The daemon is the lifecycle writer; CLI mutations use its local API, and a second start cannot create a second supervisor | CLI and HTTP integration tests | enforced |
| F35 | Live park commits only after a validated attempt checkpoint; abort tears down the process group and records `aborted`, never `parked` | worker, application, and store tests | enforced |
| F36 | Resume and operator input continue the same Work and execution with a fresh attempt id | application, store, and CLI tests | enforced |
| F37 | Daemon startup reclaims every unconfirmed active lease before accepting controls | store and CLI tests | enforced |
| F38 | Runtime roots/images are digest-verified; user Workers have finite resources, mandatory seccomp, no capabilities, and no direct network | profile and Worker adapter tests | enforced |
| F39 | Worker-writable control artifacts and Workengine store/workspace files are opened without following symlinks | store, Worker, and workspace adapter tests | enforced |
| F40 | Untrusted goal/input/output/artifact text is data and cannot select a Work transition; observable child output remains payload-free | application, Worker, HTTP, and CLI tests | enforced |
| F41 | Queue capture is generation-CAS protected, excludes a second consumer, and a captured attempt must consume the exact lease | application, store, and CLI tests | enforced |
| F42 | Work, relations, queue selection, and workspace roots are project-scoped; blocking relations are deterministic data | domain, store, and workspace tests | enforced |
| F43 | Named shared quotas are acquired and released atomically in the control-plane store | store tests | enforced |
| F44 | Inbound records require an explicit ready signal and durable idempotency receipt | application, store, and integration-adapter tests | enforced |
| F45 | Publications are best-effort outbox effects; external-input park notifications are targeted and never write status | application, store, and integration-adapter tests | enforced |
| F46 | Remote mutations require expected project/repository/revision context and expose only compare-and-swap | application tests | enforced |

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
