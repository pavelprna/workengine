# Architecture overview

Workengine is a hexagonal control plane. Layers are Cargo crates. A forbidden dependency does not compile.

## System (C4 container)

```
              inbound: HTTP observer + intake
                         |
                         v
+------------------------------------------------------+
|  workengine CLI  (composition root)                   |
|  clap; wires ports; no domain logic                    |
+------------------+-----------------------------------+
                   |
                   v
+------------------------------------------------------+
|  application                                          |
| create / capture / start / complete / integration   |
|  ports = traits                                       |
+------------------+-------------------+---------------+
                   |                   |
         +---------+                   +----------+
         v                                        v
  domain (FSM, ids,                    adapters
  outcomes, errors)                    store | worker | workspace
         ^                                        |
         |                                        v
         |                              SQLite / stub or process /
         |                              workspace directory
         +----------------------------------------+
```

The local HTTP adapter is the daemon transport, an inbound observer, and the
operator control surface. Its GET/SSE routes use the read-only query port and do not recover or
change Work. The query projection includes immutable execution specs, ordered
attempts, heartbeat, bounded redacted process metadata, diagnostics, and a
finite control-plane proof catalogue. Lifecycle routes invoke the daemon-owned
application path through a control supplied by the composition root. CLI
mutation commands are localhost clients of the same routes. Park, abort,
resume, answer, consent, and project-scoped queue capture wait for
server-confirmed state. External sources and publishers run independently
through integration ports and durable receipt/outbox records.

## Crates

| Directory | Package | Role |
| --- | --- | --- |
| `crates/domain` | `workengine-domain` | Entities, value objects, FSM, closed outcomes, domain errors. No I/O. |
| `crates/application` | `workengine-application` | Use cases and port traits. Depends only on `workengine-domain`. |
| `crates/adapters-store` | `workengine-adapters-store` | `WorkStore` implementations. |
| `crates/adapters-worker` | `workengine-adapters-worker` | `WorkerRunner` implementations (stub and generic process). |
| `crates/adapters-workspace` | `workengine-adapters-workspace` | `WorkspaceFactory` implementations. |
| `crates/adapters-integrations` | `workengine-adapters-integrations` | Explicit-signal JSONL inbound, best-effort JSONL publication, and isolated integration jobs. |
| `crates/adapters-http` | `workengine-adapters-http` | Localhost daemon transport, HTTP/SSE observer and control client, and embedded Web UI. |
| `crates/cli` | `workengine-cli` (bin `workengine`) | Composition root. |

`workengine-cli` depends on application and on adapters. `workengine-domain` depends on none of them.

```
workengine-cli
  ├── workengine-application
  │     └── workengine-domain
  ├── workengine-adapters-store      → application + domain
  ├── workengine-adapters-worker     → application + domain
  ├── workengine-adapters-workspace  → application + domain
  ├── workengine-adapters-integrations → application + domain
  └── workengine-adapters-http       → application + store
```

Do not split further on day one. Inside a crate, use modules.

## Direction of dependencies

Dependencies point inward.

```
adapters  →  application  →  domain
cli       →  application + adapters
```

- `workengine-domain` MUST NOT depend on application, adapters, clap, rusqlite, `std::process`, `std::fs`, or network crates.
- `workengine-application` MUST NOT depend on adapters or `workengine-cli`.
- Domain language has no product or tracker names.

The Cargo graph is the fitness function. Clippy `disallowed_methods` / `disallowed_types` is a second belt inside a crate. See [fitness.md](fitness.md).

## What is not a layer

- JSON Schema files for `outcome` and store events are product contracts, not a crate.
- Worker profiles (phase names for a team) are configuration, not a domain crate.
- CI YAML of a git host is an adapter around `just check`, not architecture.

## Related

- Ports: [ports.md](ports.md)
- Fitness: [fitness.md](fitness.md)
- ADR 0001: [../adr/0001-rust-workspace-hexagonal.md](../adr/0001-rust-workspace-hexagonal.md)
