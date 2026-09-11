# 0013. Project-scoped queue and integration boundaries

- Status: Accepted
- Date: 2026-09-11

## Context

The live local daemon owns one Work at a time safely, but v0.7 must let several
independent consumers drain a queue, keep repositories isolated, account for
shared capacity, and exchange data with external systems. Treating selection,
external delivery, or a remote revision as an informal convention would
reintroduce conflicting writers and make one slow dependency a global stall.

## Decision

- Every Work belongs to one opaque `ProjectId`. Its repository identity is
  snapshotted with its attributes. Workspace roots are nested below that
  project, and relations may connect Work only inside the same project.
- Parent/child, blocks, and follows are closed relation kinds persisted as
  Work data. Queue eligibility treats an unsatisfied `blocks` edge as a
  deterministic exclusion; Workers do not negotiate dependencies.
- Queue consumers first obtain a durable `CaptureLease`. SQLite chooses an
  eligible Work and installs the lease in one transaction using Work
  generation as compare-and-swap context. A captured start must consume the
  exact lease while claiming its attempt. Direct starts cannot bypass an
  outstanding capture.
- Shared capacity uses named quota leases in the control-plane store. Limits,
  current use, and lease acquisition are changed atomically. Projects and
  adapters do not keep authoritative local counters.
- An inbound port returns records with a closed explicit signal. Only `ready`
  records enter the create path, and a durable source/record receipt makes
  capture idempotent. Source text remains data.
- Publication uses a durable control-plane outbox. Status/event transactions
  enqueue facts; a publisher reads them after commit and records a best-effort
  attempt. A delivery error cannot undo the transition. External-input parks
  enqueue a targeted notification for the Work's intended operator.
- A remote-mutation port requires an immutable expected project, repository,
  and revision context. Its only write operation is compare-and-swap; context
  mismatch is a conflict and blind overwrite is not representable through the
  port.
- External polling and delivery are scheduled independently per source,
  publisher, and project. No store transaction is held while external code is
  called, so a slow adapter cannot block an unrelated queue or transition.

## Consequences

- Local Work created without v0.7 options belongs to the reserved `default`
  project, preserving the existing CLI and localhost intake path.
- Capture and quota leases need explicit release/reclamation. They are not
  Work status and never make an external system authoritative.
- SQLite gains private tables and columns under the existing store-v2 product
  contract. Event payloads gain additive optional project/repository fields.
- Concrete tracker, chat, and forge products remain adapter choices and do not
  enter domain vocabulary.
