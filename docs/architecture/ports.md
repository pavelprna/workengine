# Ports

Ports are traits in `workengine-application`. Adapters implement them. The first vertical slice implements the four runtime ports; inbound and publish wait.

Trait signatures below are intent, not frozen Rust. The crate is the API. Behaviour (`MUST` / `MUST NOT`) lives in `docs/spec/`. This file maps each port to adapters.

## WorkStore

Persists current Work status and the append-only event log.

| First adapter | Next |
| --- | --- |
| In-memory, then SQLite in a data directory | Postgres when more than one machine exists |

Behaviour: [workflow.md](../spec/workflow.md) (persistence: atomic status+event, immutable events, replay, capture).

## WorkerRunner

Spawns a Worker process, waits, enforces budget and hang detection, returns a typed outcome.

| First adapter | Next |
| --- | --- |
| Stub that writes a schema-valid outcome and exits | A real CLI agent behind the same trait |

Behaviour: [worker.md](../spec/worker.md) (process group, budget and hang outside the child, closed outcome).

Even the stub is part of the product: the core must run without an LLM.

## WorkspaceFactory

Creates and addresses the isolated directory for a Work. `record_memory` takes domain status and outcome kind; the adapter encodes the JSON line.

| First adapter | Next |
| --- | --- |
| Dedicated directory for the life of the Work | git worktree, then a container |

Behaviour: [workspace.md](../spec/workspace.md) (unique root per `WorkId`, containment).

## Clock

Wall-clock timestamps for `create` and for `Started` / `Completed` / `Parked` events (`unix_ms`). Hang and budget deadlines belong to the `WorkerRunner` adapter, not this trait.

| First adapter | Next |
| --- | --- |
| `SystemClock` in application | Test fake (`FakeClock` in application tests) |

Used by `create`, `start`, `complete`, and `park`. No separate spec.

## Inbound (not first slice)

Creates Work from an external source after an explicit ready signal.

| First adapter | Next |
| --- | --- |
| None. CLI creates Work | Any tracker behind the port |

The domain does not mention a tracker. Adding a source is a new adapter crate or module, not a new entity. See [work.md](../spec/work.md) Source.

## Publisher

Best-effort effects visible outside Workengine (board, mail, chat).

| First adapter | Next |
| --- | --- |
| No-op | Any channel behind the port |

Behaviour: [workflow.md](../spec/workflow.md) Publication (best-effort; the Worker process does not publish).

## Use cases

One use case per module, named after the CLI verb:

- `create`
- `next`
- `start`
- `complete`
- `park`

`start` binds a workspace, spawns through `WorkerRunner`, waits, and applies `complete`. If an outcome artifact already exists (leftover `running` or `parked`), `start` applies `complete` and MUST NOT spawn. CLI `complete --file` is the recovery path for leftover `running` or `parked` Work; it does not park first.

Later: `capture`. Not a god-object orchestrator.

## Adding an adapter

1. Keep the trait in `workengine-application` unless the contract itself changes (then ADR + this file).
2. Implement in the matching `workengine-adapters-*` crate.
3. Translate DTO ↔ domain on the adapter boundary. Vendor schema MUST NOT leak into `workengine-domain`.
4. Wire the adapter only in `workengine-cli`.
5. Cover the port with a fake in tests; do not generate mocks.

## Related

- Overview: [overview.md](overview.md)
- Specs: [../spec/work.md](../spec/work.md), [../spec/worker.md](../spec/worker.md), [../spec/workspace.md](../spec/workspace.md), [../spec/workflow.md](../spec/workflow.md)
