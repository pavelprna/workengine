# Product roadmap

Workengine is a local-first control plane for coding agents. The Web UI becomes
the primary operator surface; the CLI remains useful for scripting and
break-glass recovery. Workengine remains the only writer of Work status.

This is a product roadmap, not a second behavioural specification. Normative
rules remain in [spec](spec/work.md), public compatibility in
[compatibility](product/compatibility.md), and architectural decisions in
[ADRs](adr/INDEX.md).

## Current baseline

The current CLI slice has the Work FSM, SQLite event history, workspace binding,
profile-based sandbox launch, process-group supervision, and JSON snapshots and
event cursors. It does not yet have a daemon, HTTP API, Web UI, durable attempt
provenance, live control, capture/CAS, inbound sources, or publishers.

The sandboxed-attempt decision is accepted, but its attempt IDs, protected
control artifacts, configuration snapshots, checkpoint/abort supervision, and
per-Work CAS remain pending. See [ADR 0005](adr/0005-sandboxed-attempt-protocol.md).

## v0.1 — Observer

Make observation safe and useful before adding Web controls.

- Separate read-only queries from recovery. Reading Work or events must never
  park, complete, or otherwise mutate Work.
- Define a stable query/event envelope with cursor, filtering, ordering, and
  pagination.
- Add a localhost-only `serve` mode with an HTTP query API and resumable SSE
  stream. The UI never reads SQLite directly.
- Deliver a read-only Web UI: system overview, filterable Work list, Work detail,
  event timeline, and runtime health/doctor page.
- Keep this first API explicitly internal until the execution model below is
  durable enough to freeze as public `v1`.

Exit criterion: an observer can reconnect after an event cursor without losing
events, and repeated polling of a running Work produces no new store event and
does not change its status.

## v0.2 — Durable execution

Finish the control-plane model needed for trustworthy execution and live
observation.

- Introduce `ExecutionId`, `AttemptId`, immutable `ExecutionSpec`, and
  `ConfirmedOutcome`.
- Persist profile, sandbox/runtime digest, budget, retry/channel policy, and
  secret references once per execution. Configuration may not silently change on
  resume.
- Replace unconditional snapshot writes with intent-specific, generation/CAS
  store operations and an active-attempt lease.
- Use protected attempt-scoped control artifacts; reject stale, foreign, or
  unproven outcomes.
- Move confirmed outcomes and workspace memory out of the Worker-writable
  workspace into control-plane storage.
- Account for a budget across retries and resumes; distinguish budget exhaustion
  from heartbeat timeout.
- Make recovery owner-aware: recovery may only reclaim a genuinely abandoned
  attempt.

Exit criterion: concurrent starts cannot run one Work twice, and only the
matching active attempt can complete it.

## v0.3 — Complete observation

Extend the UI and query model from Work status to the execution itself.

- Show execution and attempt history, profile/config digest, sandbox, budget,
  heartbeat, retries, checkpoint, and terminal reason.
- Provide a safe artifact catalogue and finite navigation from summary to proof
  artifacts.
- Surface redacted process records and explicit diagnostics instead of requiring
  operators to parse raw logs.

Exit criterion: an operator can explain the current state and outcome of every
Work from the UI without shelling into its workspace.

## v0.4 — Operator

Move lifecycle control into a long-lived, single-writer supervisor.

- Let the daemon own active supervisors; make CLI commands clients of the same
  control API.
- Add create, start, resume, park, abort, answer, and explicit-consent commands.
- Implement durable checkpoint requests for park and immediate sandbox/cgroup
  teardown for abort.
- Recover labelled OCI executions after a daemon crash.
- Add corresponding UI controls that wait for server-confirmed state changes.

Exit criterion: park reaches a validated save point, abort never masquerades as
park, and an operator can interrupt any active Work.

## v0.5 — Policy and containment

Close the security policy around real coding-agent workloads.

- Verify Bubblewrap rootfs digests and OCI image digests.
- Add resource limits, seccomp/capability policy, no-follow filesystem handling,
  and a deny-by-default permission profile.
- Replace direct network access with a policy-controlled egress broker.
- Scrub data before publication and require explicit consent for irreversible
  actions.

Exit criterion: a profile cannot obtain host access, network access, or an
irreversible permission implicitly.

## v0.6 — Queue and integrations

Scale from one local operator to independent projects and adapters.

- Add capture for several workers, project/repository isolation, and centralized
  quotas.
- Model Work relations such as parent/child, blocks, and follows.
- Add explicit-signal inbound adapters and best-effort publishers.
- Notify the intended operator when Work parks for external input.
- Add expected-context and compare-and-swap protection for remote mutations.

Exit criterion: a slow source, project, or publisher cannot block unrelated
Work, and no external system becomes the status source of truth.

## Beyond v0.x

Remote or multi-user operation is deliberately later: authentication, TLS,
roles, audit actors, multi-host coordination, and a shared store need their own
threat model and ADRs. They are not required for a valuable localhost operator
release.
