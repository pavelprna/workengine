# 0004. Worker profile is configuration

- Status: Accepted
- Date: 2026-09-09

## Context

A live CLI agent must run behind `WorkerRunner` without becoming a domain entity or a vendor branch. Channel errors toward a model must be classified by the reaction Workengine will take (retry, fail, park). Secrets must be references. Inbound and Publisher are still later ports.

Copying a checkout into the workspace is an adapter concern. Recopying on resume would wipe the save point.

## Decision

- A Worker profile is configuration (argv, env references, retry limit, optional checkout path), not a crate and not a domain type. The profile name on Work stays an opaque key.
- The first live adapter is a generic process (`ProcessWorkerRunner`): spawn the configured argv in the workspace, wrap output in the existing JSON stream, and decode `outcome.json` when present. The child does not have to speak that schema. Workengine does not parse stdout as a status. Missing artifact after exit is `failed`, not success from exit code 0. The stub adapter remains.
- Channel classification is an application reaction (`ChannelReaction`: retry, fail, park). It is not three new Work statuses and not three outcome kinds. Fail still completes as `channel_error`. Park calls `park` on the same Work. Retry re-spawns inside the same `start`, with the limit snapshotted for that call.
- Child environment is empty except declared `{ fromEnv = "NAME" }` entries. Inline env values are rejected.
- Checkout is copied only on first bind of that Work. The goal is written into the workspace as data (`workengine-goal.txt`).

## Consequences

- Adding Cursor, Claude, or Codex is a profile file, not an `if` in domain or application.
- A broken profile B must not prevent starting Work on profile A (lazy validation of the named profile).
- Real containment (bwrap/container) is still a later adapter behind `WorkspaceFactory`. Env isolation is not a sandbox.
- Inbound and Publisher remain absent.

## Related

- Ports: [../architecture/ports.md](../architecture/ports.md)
- Worker: [../spec/worker.md](../spec/worker.md)
- ADR 0003: [0003-worker-is-a-process.md](0003-worker-is-a-process.md)
