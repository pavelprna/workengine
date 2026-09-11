# Ports

Ports are traits in `workengine-application`. Adapters implement them. Local
observation, daemon-owned lifecycle, project queue, explicit-signal inbound,
publication, quota, and remote mutation boundaries are present.

Trait signatures below are intent, not frozen Rust. The crate is the API. Behaviour (`MUST` / `MUST NOT`) lives in `docs/spec/`. This file maps each port to adapters.

`WorkspaceFactory::bind` takes a `BindRequest` (goal text, optional checkout). `start` takes a `StartRequest` (budget, retry limit snapshotted for the call, optional checkout).

## WorkQuery and WorkStore

`WorkQuery` reads current Work, its sequenced immutable events, and a typed
execution/attempt observation projection. It has no transition or recovery
operation. `WorkStore` extends it with the atomic status-plus-event write used
by control-plane use cases. `AttemptRecorder` is the narrow live-observation
and supervisor-control port used by a supervised Worker to record heartbeat
and payload-free process metadata and poll an attempt-scoped durable park/abort
request against the matching active lease.

Persists current Work status and the append-only event log.

| First adapter | Next |
| --- | --- |
| In-memory, then SQLite in a data directory | Postgres when more than one machine exists |

Behaviour: [workflow.md](../spec/workflow.md) (persistence: atomic status+event, immutable events, replay, capture).

## QueueStore, RelationStore, and QuotaStore

`QueueStore` atomically selects eligible project-scoped Work and installs a
generation-bound `CaptureLease`. `claim_attempt` consumes the exact capture;
direct start is rejected while a capture exists. `RelationStore` persists the
closed relation graph, and `QuotaStore` owns named capacity leases. Configured
`queue:global` and `project:<id>` quotas are acquired with capture/start and
released when Work leaves its active slot.

## WorkerRunner

Spawns a Worker process, verifies its runtime digest and deny-by-default
permission profile, applies resource/seccomp/capability and brokered-egress
containment, waits, enforces budget and hang detection, and returns a typed
outcome.

| First adapter | Next |
| --- | --- |
| Stub that writes a schema-valid outcome and exits; generic process from a profile (argv + env refs) | A named vendor CLI still behind the same trait |

Behaviour: [worker.md](../spec/worker.md) (process group, budget and hang outside the child, closed outcome).

Even the stub is part of the product: the core must run without an LLM.

## WorkspaceFactory

Creates and addresses the isolated directory for a Work. `record_memory` takes domain status and outcome kind; the adapter encodes the JSON line. `bind` writes the goal as data and, on first bind, may copy an operator checkout into that unique root.

| First adapter | Next |
| --- | --- |
| Dedicated directory for the life of the Work; optional copy of a checkout | git worktree, then a container |

Behaviour: [workspace.md](../spec/workspace.md) (unique root per `WorkId`, containment).

## Clock

Wall-clock timestamps for `create` and for `Started` / `Completed` / `Parked` events (`unix_ms`). Hang and budget deadlines belong to the `WorkerRunner` adapter, not this trait.

| First adapter | Next |
| --- | --- |
| `SystemClock` in application | Test fake (`FakeClock` in application tests) |

Used by `create`, `start`, `complete`, and `park`. No separate spec.

## Inbound

Creates Work from an external source after an explicit ready signal. A local
operator may also create Work through the localhost HTTP transport; that route
uses the same `create` use case and is not an external source.

| First adapter | Next |
| --- | --- |
| Localhost HTTP query/SSE observer and operator intake; JSONL explicit-signal inbox | Any tracker behind the port |

The domain does not mention a tracker. Adding a source is a new adapter crate or module, not a new entity. See [work.md](../spec/work.md) Source.

## Publisher

Best-effort effects visible outside Workengine (board, mail, chat).

| First adapter | Next |
| --- | --- |
| Durable outbox plus payload-minimal JSONL publisher | Any channel behind the port |

Behaviour: [workflow.md](../spec/workflow.md) Publication (best-effort; the Worker process does not publish).

## Use cases

One use case per module, named after the CLI verb:

- `create`
- `capture`
- `next`
- `start`
- `complete`
- `request_control`
- `record_operator_input`

`start` binds a workspace, writes the goal, creates an attempt-scoped control
directory, spawns through `WorkerRunner`, waits, and applies `complete` or a
checkpoint-confirmed park. A resume uses the same immutable execution and a
fresh attempt. Channel errors are classified by reaction: fail completes as
`channel_error`; a checkpointed park leaves the same Work parked; retry
re-spawns inside that `start` up to the snapshotted limit, then fails. Shared
workspace outcome files remain rejected.

Integration use cases poll one source or dispatch one publisher at a time.
Adapters schedule those calls independently and never hold a store transaction
while external code runs. `RemoteMutation` exposes compare-and-swap with an
expected project/repository/revision context; it has no blind-write method.

## Adding an adapter

1. Keep the trait in `workengine-application` unless the contract itself changes (then ADR + this file).
2. Implement in the matching `workengine-adapters-*` crate.
3. Translate DTO ↔ domain on the adapter boundary. Vendor schema MUST NOT leak into `workengine-domain`.
4. Wire the adapter only in `workengine-cli`.
5. Cover the port with a fake in tests; do not generate mocks.

## Related

- Overview: [overview.md](overview.md)
- Specs: [../spec/work.md](../spec/work.md), [../spec/worker.md](../spec/worker.md), [../spec/workspace.md](../spec/workspace.md), [../spec/workflow.md](../spec/workflow.md)
