# 0006. Local Web observer boundary

- Status: Accepted (partially superseded by ADR 0007)
- Date: 2026-09-11

## Context

The Web UI needs a stable way to observe Work without becoming a second writer
or reading SQLite directly. The foreground CLI previously ran recovery whenever
it opened the store, so an apparently read-only command could park Work.

## Decision

- Read access is the `WorkQuery` application port. It is separate from the
  mutation-capable `WorkStore` port.
- SQLite observation uses a read-only connection. HTTP, SSE, `list`, `show`,
  and `events` do not recover or write Work state.
- Until a durable active-attempt lease exists, `start` is the foreground
  controller and runs recovery before it launches a Worker.
- `workengine serve` is localhost-only (`127.0.0.1` and `::1`), has no live
  execution controls, authentication, or remote bind option, and serves a
  same-origin embedded UI plus `/api/v0`. ADR 0007 adds its narrow create-only
  operator intake route.
- `contracts/observer.openapi.yaml` is the canonical internal local API
  contract. It is intentionally not a public v1 API.
- Tokio is permitted only in the HTTP adapter. Domain and application remain
  synchronous; the Worker execution loop is still spawn, wait, record.

## Consequences

- The UI is a client of HTTP/SSE, never of the SQLite schema or a workspace.
- SSE resumes from the append-only event sequence. A later daemon may replace
  polling with a live broadcaster without changing the cursor contract.
- Remote or multi-user serving needs a threat-model and ADR covering identity,
  TLS, authorization, audit, and a shared store.
