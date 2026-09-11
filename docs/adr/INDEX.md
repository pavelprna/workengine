# Architecture decision records

| Number | Title | Status |
| --- | --- | --- |
| [0000](0000-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0001](0001-rust-workspace-hexagonal.md) | Rust workspace as hexagonal layers | Accepted |
| [0002](0002-source-of-truth-is-internal.md) | Source of truth is internal | Accepted |
| [0003](0003-worker-is-a-process.md) | Worker is a process | Accepted |
| [0004](0004-worker-profile-is-configuration.md) | Worker profile is configuration | Accepted |
| [0005](0005-sandboxed-attempt-protocol.md) | Sandboxed attempt protocol | Accepted |
| [0006](0006-local-observer-boundary.md) | Local Web observer boundary | Partially superseded by 0007 |
| [0007](0007-local-operator-intake.md) | Local operator intake before live control | Accepted |
| [0008](0008-execution-attempt-identity.md) | Execution and attempt identity | Accepted |
| [0009](0009-local-operator-launch.md) | Local operator launch through the foreground control path | Accepted |
| [0010](0010-observable-execution-projection.md) | Observable execution projection | Accepted |

Status values: Proposed, Accepted, Superseded, Deprecated.

Read this index before changing layers, store, or runner. Then read the relevant ADR. An `Accepted` decision is an invariant until a new ADR supersedes it.

Commit, branch, and license policy live in [../../CONTRIBUTING.md](../../CONTRIBUTING.md), not here.
