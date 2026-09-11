# 0011. Local daemon owns lifecycle supervision

- Status: Accepted
- Date: 2026-09-11

## Context

The v0.3 local operator starts a Worker in the foreground of either a CLI
process or an HTTP request. The active-attempt lease prevents two starts, but
there is no durable owner that another operator command can address while the
Worker is running. A standalone CLI `park` can only repair a leftover status;
it cannot ask the live Worker for a checkpoint or distinguish that request
from an abort.

v0.5 needs one long-lived owner for supervisors, interruptible attempts, and
crash recovery without moving transition decisions into an async runtime or a
Worker.

## Decision

- `workengine serve` is the local daemon and the only process that accepts
  lifecycle mutations while it is running. CLI lifecycle commands are
  localhost clients of its `/api/v0` control API. An exclusive advisory lock
  permits only one daemon owner per data directory. Read-only CLI commands may
  continue to use the query port directly.
- The HTTP adapter owns transport and concurrency only. It receives a
  synchronous lifecycle-control implementation from the CLI composition root;
  application use cases and the domain remain synchronous and Tokio-free.
- Start, resume, park, abort, answer, and consent are server-confirmed
  commands. A start or resume request may remain open while the daemon-owned
  supervisor runs. Other requests remain serviceable and address that active
  attempt through its durable lease.
- Park and abort are durable attempt-scoped control requests. A supervisor
  polls them outside the Worker. Abort tears down the process group immediately
  and confirms a failed terminal result whose terminal reason is `aborted`.
  Park first requests and validates an attempt-scoped checkpoint, then tears
  down the process group and commits `parked`. It never commits `parked` merely
  because a process was killed.
- Each attempt gets a private control directory outside its workspace. A
  sandbox sees only that directory at `/run/workengine`. Request and candidate
  checkpoint documents bind schema version, Work, execution, attempt, and
  profile. Workengine validates the candidate and records the checkpoint before
  releasing the lease.
- Resuming parked Work creates a fresh `AttemptId` under the same
  `ExecutionId`. The current resolved profile must reproduce the immutable
  execution snapshot; otherwise resume fails closed.
- Answers and explicit consent are durable operator-input records and are
  materialized as data for the same Work before resume. They never create
  replacement Work and never directly choose a status.
- At daemon startup every active attempt is reconciled before new controls are
  accepted. This release does not reattach a generic process: it tears down an
  exactly identified owned runtime when possible, marks the unconfirmed
  attempt reclaimed, clears its lease, and returns the same Work to `parked`.
  A future backend may reattach only with backend-specific ownership proof.

## Consequences

- Lifecycle commands require a running local daemon; they cannot become a
  second status writer when it is absent.
- The internal `/api/v0` surface gains lifecycle routes and command payloads,
  but remains localhost-only and outside the public v1 compatibility promise.
- A Worker profile that supports live park must implement the checkpoint file
  protocol. Missing or invalid checkpoints fail closed and do not masquerade as
  a successful park.
- The store needs durable control requests, checkpoint state, operator inputs,
  and active-attempt reclamation. These are private migrations under the
  existing v2 data-directory contract.
