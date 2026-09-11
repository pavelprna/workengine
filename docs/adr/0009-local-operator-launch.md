# 0009. Local operator launch through the foreground control path

- Status: Accepted
- Date: 2026-09-11

## Context

ADR 0007 deliberately limited the localhost Web surface to observation and
Work intake while execution had no exclusive attempt ownership. Durable
execution identities now make it possible to expose the smallest useful live
control without introducing the long-lived daemon planned for v0.5.

The CLI and Web server must not become independent status writers. A Web start
also cannot accept executable configuration from the browser or optimistically
claim that a Worker is running before the store has accepted its attempt.

## Decision

- `workengine serve` gains one same-origin local control:
  `POST /api/v0/works/{workId}/start`.
- The request has no body. The Work's persisted profile selects local
  configuration; argv, sandbox, checkout, budget, retry policy, and secret
  references never cross the browser boundary.
- CLI and HTTP launch call the same synchronous application `start` use case.
  The HTTP adapter receives a control implementation from the CLI composition
  root rather than depending on Worker or Workspace adapters.
- A start first claims an attempt transactionally. The store changes the Work
  to `running`, appends `Started`, and installs its exclusive active-attempt
  lease in one transaction. A competing start receives a conflict and does not
  spawn a Worker.
- The HTTP response waits for the foreground start invocation to reach its
  confirmed terminal or parked result. The event stream remains the live
  observation path and can expose the committed `running` transition while the
  request is in flight.
- This slice adds no park, abort, checkpoint, answer, remote bind, or
  browser-supplied execution settings.

## Consequences

- The Web UI can create and explicitly launch Work while preserving one
  application control path and one store authority.
- `serve` owns only foreground launches for its own process lifetime. It is not
  the durable supervisor daemon; crash reconciliation and live interruption
  remain v0.5 work.
- A launch may be a long-lived HTTP request. That is intentional for this
  bounded localhost slice and can be replaced by command resources when the
  daemon is introduced.
