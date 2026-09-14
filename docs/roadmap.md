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
queue, Work detail and timeline, and the diagnostic boundary. The v0.3 operator
slice adds explicit localhost launch through the same application path as the
CLI, backed by an exclusive active-attempt lease and control-plane confirmed
outcomes. v0.4 completes observation with an execution ledger, attempt
heartbeat and retry history, immutable configuration and runtime fingerprints,
structured terminal diagnostics, bounded redacted process records, and a
finite catalogue of control-plane proof artifacts. v0.5 adds the localhost
daemon as lifecycle owner, daemon-client CLI commands, checkpointed park,
distinct abort, same-execution resume, durable answers and consent, and
ownership-checked crash reclamation. External dependencies sit behind
application ports. v0.6 closes the local Worker policy with verified runtime digests,
finite resource limits, mandatory seccomp and capability dropping,
deny-by-default brokered egress, no-follow control-plane file handling, and a
cleared runtime environment. v0.7 adds project-scoped capture, relation-aware
queueing, centralized quotas, explicit-signal inbound receipts, a best-effort
publication outbox with targeted park notification, and expected-context CAS
remote mutation boundaries.

The sandboxed-attempt decision is accepted. Attempt IDs, immutable execution
snapshots, per-Work active leases, and confirmed outcomes are present;
protected Worker-writable control artifacts and checkpoint/abort supervision
are present; resource and egress containment are enforced by the v0.6 profile
policy.
See [ADR 0005](adr/0005-sandboxed-attempt-protocol.md).

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

## v0.8 — Real Worker vertical slice

Prove that the existing control plane can run useful coding-agent work rather
than only demonstrate its mechanics with the built-in stub.

- Add one production-shaped, product-neutral harness for an external coding
  agent CLI. The selected executable remains Worker-profile configuration and
  does not enter the domain or application crates.
- Materialize an approved task packet, repository context, and validation
  contract into the isolated Workspace as versioned data.
- Return a typed outcome and bounded proof artifacts through the existing
  attempt protocol. Free-form Worker text remains data and cannot choose Work
  status.
- Exercise the complete live lifecycle with a real workload: start, heartbeat,
  outcome confirmation, park for a precise question or consent, answer, and
  same-execution resume.
- Keep this milestone to one Work at a time. Do not add planning, graph
  expansion, or multi-agent coordination before the vertical slice exposes the
  real integration gaps.

Exit criterion: a representative change in a real repository can run inside a
contained Workspace and finish with a reviewable change plus observable
validation evidence, or park with a structured request for operator input.

## v0.9 — Closed validation loop

Let a Worker obtain and act on deterministic feedback without requiring the
operator to relay every failed check.

- Make the repository validation contract explicit and immutable for an
  execution. The Worker can run the declared checks, inspect their result, fix
  the change, and repeat within its budget.
- Record structured validation evidence without persisting raw child output or
  trusting a Worker-authored success claim as the authority.
- Classify implementation failure, validation failure, environment failure,
  dependency unavailability, missing permission, ambiguous intent, hang, and
  budget exhaustion so that retry, park, and fail remain code decisions.
- Extend accounting where evidence requires it, including attempt duration,
  retries, validation cycles, operator interventions, and declared non-time
  budgets.
- Add an adapter-level silence or progress watchdog if real workloads show that
  a wall-clock deadline alone is insufficient.

Exit criterion: a representative Worker can iterate on a change for a
substantial unattended run and finish with deterministic checks passed, or
stop at the correct boundary with a specific, durable question. An ordinary
test failure does not require a human relay step.

## v0.10 — Approved change to Work graph

Scale from one durable execution to one complete engineering change composed
of several explicit Work items.

- Accept an approved, versioned change description and decomposition through
  an adapter. A model may propose the decomposition, but Workengine executes
  only the persisted graph accepted by the operator or source policy.
- Represent implementation, review, verification, and similar delivery phases
  as Work data and Worker profiles, never as new canonical Work statuses.
- Materialize parent/child, blocks, and follows relations and let the existing
  deterministic queue select eligible Work.
- Add a git-worktree Workspace adapter so independent Work can modify the same
  repository identity without sharing a working copy.
- Run independent Work in parallel under project and shared-resource quotas;
  keep each Work under one status writer and one active-attempt lease.
- Carry outputs between Work as versioned, attributed artifacts rather than as
  implicit conversations between Workers.

Exit criterion: one approved feature can execute as a mixed sequential and
parallel Work graph, with every dependency, attempt, artifact, and operator
intervention attributable from intake to verification.

## v0.11 — Review, CI, and integration feedback

Connect agent execution to the independent quality and integration systems
that decide whether a change is ready to land.

- Add git and forge adapters for expected branches, commits, reviews, and
  compare-and-swap integration. A Worker never blind-writes the target branch.
- Ingest CI results as explicit external signals. CI is verification evidence,
  not the source of truth for Work status and not the lifecycle orchestrator.
- Create bounded repair Work from failed deterministic checks without reopening
  successful Work or allowing an unbounded retry loop.
- Treat reviewer and verifier results as attributed evidence with explicit
  acceptance policy. A model's approval alone cannot bypass mandatory checks.
- Stop on changed repository context, conflicting integration state, missing
  consent, or an exhausted budget and surface the decision to the operator.

Exit criterion: an approved change can progress from its Work graph through
implementation, local validation, review, CI feedback, bounded repair, and a
review-ready integration candidate with a complete control-plane history.

## v0.12 — Dogfood and operational hardening

Use Workengine as a normal development system long enough to distinguish
missing product capability from speculative infrastructure.

- Run representative backlogs across more than one repository and retain the
  evidence needed to compare task shapes, profiles, budgets, and failure modes.
- Surface operational measures such as unattended completion, validation
  cycles, attempts, park reasons, operator interventions, recovery events,
  elapsed time, and declared spend.
- Add explicit Workspace retention and cleanup policy, store migrations,
  backup and restore, packaging, upgrade diagnostics, and reusable profile
  distribution.
- Exercise crash, stale lease, partial adapter failure, CI delay, integration
  conflict, and unavailable dependency scenarios against the same invariants
  used in normal execution.
- Refine the Web operator around decisions and proof: what is running, why it
  stopped, what was verified, what input is required, and what can safely
  happen next.

Exit criterion: Workengine is used repeatedly for real engineering work, no
observed failure can create duplicate execution or divergent canonical status,
and an operator can diagnose and control the system without reading SQLite or
attaching to Worker processes.

## 1.0 readiness

The pre-1.0 line is ready to graduate when the local product has demonstrated
the whole path above under routine use, its public contracts match actual
operator workflows, supported store migrations and recovery paths are tested,
and the remaining breaking changes are understood rather than speculative.

Remote or multi-user operation remains deliberately later. Authentication,
TLS, roles, audit actors, multi-host coordination, and a shared store require a
separate threat model and ADRs. They are not prerequisites for a valuable,
trustworthy local 1.0.
