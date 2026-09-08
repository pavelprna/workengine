# 0001. Rust workspace as hexagonal layers

- Status: Accepted
- Date: 2026-09-08

## Context

Workengine must keep domain logic free of agent vendors, trackers, filesystems, and process APIs. Folder conventions and linters can be skipped. The first language choice also has to serve a finite state machine, a closed set of outcomes, and a CLI that spawns and kills process groups.

Go was a viable alternative (fast first slice, familiar `os/exec`). The stake for this product is a durable control plane whose invariants should fail at compile time.

## Decision

Implementation language is Rust. The compiler channel is pinned in `rust-toolchain.toml`. Edition is pinned in the workspace `Cargo.toml` when that file exists.

Layers are **crates in a Cargo workspace**, not folders inside one crate:

| Crate | Responsibility |
| --- | --- |
| `workengine-domain` | FSM, identifiers, outcomes, errors. No I/O. |
| `workengine-application` | Port traits and use cases. |
| `workengine-adapters-store` | Store adapters. |
| `workengine-adapters-worker` | Worker process adapters. |
| `workengine-adapters-workspace` | Workspace adapters. |
| `workengine-cli` | Composition root, clap, binary name `workengine`. |

Dependencies point inward. `workengine-domain` cannot compile a dependency on adapters, clap, rusqlite, or `std::process`.

Further constraints:

- No async runtime in domain or application. The loop is spawn, wait, record.
- Errors: `thiserror` in domain/application, `anyhow` (or equivalent) only at the CLI edge.
- Logging: `tracing` with `work_id` when runtime exists. No telemetry exporter by default.

Format, clippy, and `cargo deny` are the check recipe in `CONTRIBUTING.md`, not this decision.

## Consequences

- Adding a vendor adapter cannot leak into the domain without changing `workengine-domain/Cargo.toml`, which is a review event.
- Exhaustive `match` on status and outcome is a compiler check, not only a test.
- The first vertical slice is slower than it would be in Go. Process-group teardown must be designed in the worker adapter from the first real spawn, including the stub's contract.
- `AGENTS.md` does not restate layer bans. The crate graph is the ban.
