# 0007. Local operator intake before live control

- Status: Accepted
- Date: 2026-09-11

## Context

The observer is useful only after an operator has used a separate terminal to
create Work. That delays the first useful feedback loop and makes the Web
surface feel disconnected from the runtime.

The present execution model does not yet have attempt provenance or per-Work
capture/CAS. Letting a browser start a Worker now would add a second competing
controller beside the foreground CLI and could run the same Work twice.

## Decision

- This ADR supersedes the `serve`-is-only-an-observer decision in ADR 0006;
  its read-only query boundary remains in force.
- `workengine serve` remains localhost-only and same-origin, but gains a
  narrow operator-intake route: it may create a new `ready` Work through the
  application `create` use case.
- The route does not accept a workspace path, secrets, a sandbox definition,
  an argv, or a status. It accepts only a goal and an opaque Worker profile
  name. Configuration remains local operator configuration, not browser data.
- The HTTP adapter is the transport boundary; it owns a mutation-capable store
  solely for this explicit route. Observer routes continue to use the
  read-only query port and must not recover or mutate Work.
- `start`, `park`, `abort`, checkpoints, and answers remain unavailable over
  HTTP until durable attempts and an exclusive active-attempt lease exist.

## Consequences

- The Web UI can put a real Work item into the durable queue immediately and
  show its event history without a terminal round trip.
- The public contract expands from an observer-only document to an internal
  localhost API. It is still not a public or remote API.
- v0.2 remains the prerequisite for live execution controls; after it, the
  minimal `start` control moves ahead of complete observation and the daemon.
