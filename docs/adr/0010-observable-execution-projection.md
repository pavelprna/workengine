# 0010. Observable execution projection

- Status: Accepted
- Date: 2026-09-11

## Context

The v0.3 store owns immutable execution specifications, attempt leases, and
confirmed outcomes, but `WorkQuery` exposes only Work and lifecycle events.
The Web UI therefore cannot explain which configuration ran, whether an active
attempt is still supervised, why an attempt ended, or which control-plane
artifacts prove the result. The supervised subprocess stream is safe because it
removes child payloads, but it is ephemeral stderr output and is not available
to a reconnecting observer.

Reading a workspace or returning raw child output would cross the observer
boundary and could disclose Worker-controlled or secret-bearing data. Making
the HTTP adapter query SQLite directly would also duplicate store semantics in
an inbound adapter.

## Decision

- `WorkQuery` exposes a typed, read-only execution projection. It contains the
  immutable execution snapshot, ordered attempts, lease state, retry ordinal,
  budget, heartbeat, checkpoint availability, terminal reason, confirmed
  outcome, and redacted process-record summaries.
- A narrow `AttemptRecorder` application port accepts heartbeat and closed
  process event kinds for the currently leased attempt. `WorkerRunner` reports
  through that port while it supervises the process. The SQLite adapter checks
  the active Work/execution/attempt tuple before accepting a record.
- Child stdout and stderr bytes are never persisted. Their process records are
  aggregate counters with first/last observation times. Lifecycle records are
  likewise bounded by their closed event kind, so untrusted output cannot grow
  the catalogue by one database row per line.
- The local API exposes the projection plus a finite catalogue of same-origin
  proof links for the execution spec, process records, and confirmed outcome.
  It does not expose workspace paths or arbitrary filesystem navigation.
- Diagnostics are structured from the durable projection. They describe an
  active lease, retry, park, terminal reason, missing proof, or successful
  confirmation without requiring an operator to interpret raw logs.
- Execution snapshots record the runtime kind (`stub`, `bubblewrap`, or `oci`)
  in addition to its content digest. Existing immutable execution payloads may
  omit this additive field and are shown as `unknown`; they are never rewritten.

## Consequences

- Observation stays read-only and adapters continue to depend inward through
  application ports.
- Heartbeat writes are attempt-scoped and cannot revive or annotate a stale
  lease. They do not change Work status or append Work lifecycle events.
- Process diagnostics are deliberately metadata-only. Raw Worker output and
  arbitrary workspace files are not observation artifacts.
- The projection is internal `/api/v0`; its response shapes can evolve until a
  public v1 is adopted.
