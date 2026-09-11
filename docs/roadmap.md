# Product roadmap

Workengine is a local-first control plane for coding agents. The Web UI becomes
the primary operator surface; the CLI remains useful for scripting and
break-glass recovery. Workengine remains the only writer of Work status.

This is a product roadmap, not a second behavioural specification. Normative
rules remain in the [Work](spec/work.md), [Worker](spec/worker.md),
[Workspace](spec/workspace.md), and [Workflow](spec/workflow.md) specifications;
public compatibility remains in [compatibility](product/compatibility.md), and
architectural decisions remain in [ADRs](adr/INDEX.md).

A milestone schedules capabilities; its bullets do not restate or weaken the
normative rules. A milestone is complete only when the `MUST` / `MUST NOT`
rules needed for the capabilities it claims are `[TESTED]` or `[ENFORCED]` in
the canonical specification.

## Current baseline

The current CLI slice has stable Work identity and attributes, the Work FSM,
atomic SQLite status and append-only event history, workspace binding and
append-only memory, versioned typed Worker outcomes, profile-based sandbox
launch, process-group supervision, structured subprocess records, lazy profile
validation, and JSON snapshots and event cursors. v0.1 is complete: the
localhost-only HTTP/SSE API offers read-only observation plus narrow local
intake, and the embedded Web UI provides overview, a filterable paginated
queue, Work detail and timeline, and the diagnostic boundary. External
dependencies sit behind application ports. It does not yet have a daemon,
durable attempt provenance, live control, capture/CAS, external inbound
sources, or publishers.

The sandboxed-attempt decision is accepted, but its attempt IDs, protected
control artifacts, configuration snapshots, checkpoint/abort supervision, and
per-Work CAS remain pending. See [ADR 0005](adr/0005-sandboxed-attempt-protocol.md).

## v0.1 — Observe and intake

Make observation safe and useful while adding the first, deliberately narrow
Web control.

- Separate read-only queries from recovery. Reading Work or events must never
  park, complete, or otherwise mutate Work.
- Define a stable query/event envelope with cursor, filtering, ordering, and
  pagination.
- Add a localhost-only `serve` mode with an HTTP query API and resumable SSE
  stream. The UI never reads SQLite directly.
- Deliver a Web UI: system overview, filterable Work list, Work detail, event
  timeline, and runtime health/doctor page.
- Let a local operator create `ready` Work from that UI. The browser supplies
  only a goal and profile name; it never supplies an argv, secrets, a workspace
  path, a sandbox policy, or a status.
- Keep this first API explicitly internal until the execution model below is
  durable enough to freeze as public `v1`.

Exit criterion: an observer can reconnect after an event cursor without losing
or duplicating events, any two observers see the same Work snapshot, a locally
created Work is immediately durable and visible to all observers, and repeated
polling of a running Work produces no new store event and does not change its
status.

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
- Persist the mandatory accounting record for every Worker run, including its
  Work, execution, attempt, Worker configuration, budget, and terminal reason.
- Keep workspace memory logically bound to its Workspace, but move that memory
  and confirmed outcomes out of the Worker-writable directory into
  control-plane storage.
- Retain a failed Work's workspace as a diagnostic artifact. Cleanup is a
  separate, explicit operator action.
- Account for a budget across retries and resumes; distinguish budget exhaustion
  from heartbeat timeout.
- Make recovery owner-aware: recovery may only reclaim a genuinely abandoned
  attempt.

Exit criterion: concurrent starts cannot run one Work twice, only the matching
active attempt can complete it, and a failed workspace remains available until
an operator explicitly cleans it up.

## v0.3 — Operator launch

Put the first live execution control where an operator can use it, as soon as
durable attempts make it safe.

- Add an explicit local `start` control for `ready` or `parked` Work through
  the same single-writer control path as the CLI.
- Show the confirmed transition to `running` and its terminal outcome; refuse a
  second active start for the same Work.
- Keep park, abort, checkpoint, answers, and remote serving out of this slice.

Exit criterion: the UI can create a Work, explicitly start it once, and show
the confirmed terminal result without bypassing the active-attempt lease.

## v0.4 — Complete observation

Extend the UI and query model from Work status to the execution itself.

- Show execution and attempt history, profile/config digest, sandbox, budget,
  heartbeat, retries, checkpoint, and terminal reason.
- Provide a safe artifact catalogue and finite navigation from summary to proof
  artifacts.
- Surface redacted process records and explicit diagnostics instead of requiring
  operators to parse raw logs.

Exit criterion: an operator can explain the current state and outcome of every
Work from the UI without shelling into its workspace.

## v0.5 — Live operator

Move lifecycle control into a long-lived, single-writer supervisor.

- Let the daemon own active supervisors; make CLI commands clients of the same
  control API.
- Add create, start, resume, park, abort, answer, and explicit-consent commands.
  An answer continues the same parked Work; it never creates replacement Work.
- Implement durable checkpoint requests for park and immediate sandbox/cgroup
  teardown for abort.
- Reconcile every unconfirmed active attempt after a daemon crash. Reattach only
  when the runtime can prove ownership; otherwise reclaim it safely and return
  the same Work to the queue from its last confirmed point. OCI labels are one
  backend-specific source of that proof.
- Add corresponding UI controls that wait for server-confirmed state changes.

Exit criterion: park reaches a validated save point, abort never masquerades as
park, and an operator can interrupt any active Work.

## v0.6 — Policy and containment

Close the security policy around real coding-agent workloads.

- Verify Bubblewrap rootfs digests and OCI image digests.
- Add resource limits, seccomp/capability policy, no-follow filesystem handling,
  and a deny-by-default permission profile.
- Replace direct network access with a policy-controlled egress broker.
- Treat text from inbound sources, web content, and prior artifacts only as
  untrusted data; it cannot become a Workengine control command.
- Run Workengine's own codebase operations in a controlled environment isolated
  from Worker- or user-supplied environment settings.
- Keep secret values out of logs and Workengine-controlled process inspection
  surfaces, scrub data before publication, and require explicit consent for
  irreversible actions.

Exit criterion: a profile cannot obtain host access, network access, or an
irreversible permission implicitly; untrusted text cannot become a control
command; and secret values do not appear in observable process records or
published output.

## v0.7 — Queue and integrations

Scale from one local operator to independent projects and adapters.

- Add capture for several workers, project/repository isolation, and centralized
  quotas.
- Model Work relations such as parent/child, blocks, and follows.
- Add explicit-signal inbound adapters and best-effort publishers. Only the
  control plane invokes publishers; Workers never publish directly.
- Notify the intended operator when Work parks for external input.
- Add expected-context and compare-and-swap protection for remote mutations.

Exit criterion: a slow source, project, or publisher cannot block unrelated
Work, and no external system becomes the status source of truth.

## Beyond v0.x

Remote or multi-user operation is deliberately later: authentication, TLS,
roles, audit actors, multi-host coordination, and a shared store need their own
threat model and ADRs. They are not required for a valuable localhost operator
release.
