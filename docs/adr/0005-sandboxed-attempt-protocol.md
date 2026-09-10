# 0005. Sandboxed attempt protocol

- Status: Accepted
- Date: 2026-09-10

## Context

The first process adapter used a working directory as if it were containment.
It accepted a shared `outcome.json`, held a global data-directory lock, and
streamed child output verbatim. An untrusted Worker could therefore reach the
store and host, leak a secret, or complete Work using a stale artifact.

## Decision

- A user-configured Worker runs only in a sandbox backend. The first Linux
  backends are Bubblewrap and OCI (Docker or Podman); an absent or unavailable
  backend fails closed. A raw process profile is not supported.
- A Worker attempt will have a generated id and a private control directory.
  Candidate outcome and checkpoint artifacts will be scoped to that attempt and
  include the Work id, attempt id, and profile. The control plane will validate
  them before it changes status and stamp the confirmed outcome itself.
- Sandbox configuration will be resolved once at first start and will be part
  of the attempt record. Secrets are already references resolved to read-only
  files, never argv or environment values. Child output is metadata-only in
  the public stream.
- Long-lived state uses short SQLite transactions; WAL already removes the
  global data-directory lock. Per-Work CAS and durable operator requests remain
  required before concurrent capture or live control is enabled.
- The v1 store and artifacts are deliberately incompatible. They are rejected,
  not migrated or deleted.

## Consequences

- A profile needs an explicit sandbox declaration and a Worker harness that
  writes the versioned attempt artifact / checkpoint contract.
- Bubblewrap profiles require a read-only runtime root. OCI images must be
  digest-pinned. Network is disabled unless a future broker policy is wired.
- The current slice remains Linux-only for user Workers. Inbound, Publisher,
  relations, quotas, and remote CAS are still absent and must not be claimed as
  implemented.
- Attempt IDs, protected control directories, config snapshots, checkpoint/
  abort supervision, and per-Work CAS are accepted design work still pending;
  until then, legacy workspace outcome artifacts are rejected fail-closed.
