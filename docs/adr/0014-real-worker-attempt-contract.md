# 0014. Real Worker task packet and attempt response

- Status: Accepted
- Date: 2026-09-14

## Context

The generic process runner can launch a configured CLI, but its legacy contract
only asks that process to leave `outcome.json` in the writable checkout. It
does not give a coding harness one versioned task packet, cannot return bounded
review evidence, and cannot distinguish a precise question or consent request
from free-form output. Those gaps prevent the v0.8 vertical slice while the
existing lifecycle, containment, and attempt lease are otherwise sufficient.

Putting a named coding product in the core would violate ADRs 0003 and 0004.
Treating a Worker-authored validation claim as independent verification would
also pre-empt the v0.9 closed validation loop.

## Decision

- A process profile may select `task-packet-v1`. The selected executable and
  its arguments remain profile configuration; no product name enters domain or
  application code. Profiles that omit the field keep the legacy outcome
  contract during the pre-1.0 transition.
- Before spawn, the runner materializes an attempt-specific, versioned task
  packet in the retained Workspace. It binds Work, execution, attempt, profile,
  goal, repository identity and detected base revision, declared validation
  commands, and the durable operator-input path. The packet is mounted
  read-only at its sandbox path.
- The harness receives only fixed path variables for the task packet,
  attempt response, and checkpoint candidate. The response lives in the
  protected attempt control directory and repeats Work, execution, attempt,
  and profile identity.
- A response is either a closed outcome or a structured `question` / `consent`
  request. Application code maps the latter to `parked` only after the matching
  checkpoint candidate is validated. The Worker never supplies a status name.
- A completed response may carry at most eight proof artifacts, each at most
  64 KiB and at most 256 KiB in total. Each proof has a closed kind, name,
  media type, SHA-256 digest, and inline content. Workengine verifies the digest,
  stamps the confirmed outcome, persists the bounded proof, and exposes it as
  `worker_reported` evidence through the existing outcome proof endpoint.
- Worker-reported validation proof is review evidence, not control-plane
  verification authority. Running declared checks and independently deciding
  whether they passed remains v0.9 work.

## Consequences

- A product-specific wrapper may translate the neutral JSON packet into the
  invocation style of any external coding-agent CLI. That wrapper is part of
  the configured Worker process, not Workengine's outer loop.
- Same-execution resume receives a new attempt packet and the same append-only
  operator-input log. Existing execution-spec digest comparison prevents a
  changed profile or validation contract from silently replacing the approved
  configuration.
- The store adds structured input requests and proof payloads to its read-only
  execution projection. Arbitrary workspace files and child output remain
  outside the observer boundary.

## Related

- Worker: [../spec/worker.md](../spec/worker.md)
- Workflow: [../spec/workflow.md](../spec/workflow.md)
- Workspace: [../spec/workspace.md](../spec/workspace.md)
- ADR 0003: [0003-worker-is-a-process.md](0003-worker-is-a-process.md)
- ADR 0005: [0005-sandboxed-attempt-protocol.md](0005-sandboxed-attempt-protocol.md)
