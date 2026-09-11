# 0008. Execution and attempt identity

- Status: Accepted
- Date: 2026-09-11

## Context

ADR 0005 requires attempt-scoped control artifacts and configuration resolved
once at first start. The existing runtime has only `WorkId`: retries are
indistinguishable from a new logical execution, a returned `Outcome` does not
name the process attempt that produced it, and the store cannot express which
configuration an active run owns.

Durable execution needs two different identities. A logical execution survives
retry and resume; each Worker process spawn is a separate attempt. Conflating
them would make budget accounting and stale-outcome rejection ambiguous.

## Decision

- `ExecutionId` identifies one logical execution of a Work. `AttemptId`
  identifies one Worker process attempt within that execution. Both are opaque
  domain value objects.
- An `ExecutionSpec` is created once for an execution and is immutable. It
  contains the Work identity, Worker profile, Worker configuration digest,
  sandbox/runtime digest, wall-clock budget, retry limit, channel policy, and
  secret references. It contains references and digests, never secret values.
- `Outcome` remains the candidate Worker result. `ConfirmedOutcome` is a
  control-plane record binding an `Outcome` to its Work, execution, and attempt
  after the active-attempt proof has been checked. Both `ExecutionSpec` and
  `ConfirmedOutcome` are independently versioned long-lived records.
- The application port will expose intent-specific creation, attempt-claim,
  and confirmation operations. The SQLite adapter will enforce generation/CAS
  and the single active-attempt lease transactionally. Those operations are a
  subsequent slice; adding the domain vocabulary alone does not claim lease or
  provenance enforcement.

## Consequences

- Retries keep one `ExecutionId`, receive fresh `AttemptId` values, and charge
  the same snapshotted budget.
- Resume loads the persisted `ExecutionSpec`; it does not re-resolve a profile
  and silently adopt changed configuration.
- A bare or foreign `Outcome` cannot become terminal proof. Completion will
  require the matching active attempt and will persist the confirmed record in
  control-plane storage.
- Store and artifact schemas must encode both identifiers before durable
  execution is exposed as a live operator control.
