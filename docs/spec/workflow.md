# Workflow

Workflow is the process that moves Work: a finite state machine, a single writer, persistence, resume, and publication. Delivery-phase names for a given team are Worker profiles, not this document.

This document is the canonical behaviour of Workflow. Keywords follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119). Every `MUST` / `MUST NOT` is `[UNTESTED]` until a test or compile-time check in this repository covers it. Covered rows use `[TESTED]` or `[ENFORCED]`.

## Transitions

- **[TESTED]** Allowed transitions between Work statuses MUST be a finite state machine known in advance.
- **[TESTED]** An illegal transition MUST be a domain error. Workengine MUST NOT silently coerce, skip, or invent a status.
- **[ENFORCED]** Decisions about transitions MUST be taken by Workengine code, not by an LLM.
- **[ENFORCED]** A Worker MUST NOT choose the next Work status.
- **[ENFORCED]** Parallelism MUST be between Work items. Inside one Work there is one status writer.

First-slice transitions. Status names are defined in [work.md](work.md). Workengine maps a closed Worker outcome onto a path; the Worker does not choose the status.

| From | To | How |
| --- | --- | --- |
| `ready` | `running` | `start` |
| `parked` | `running` | `start` (continue the same Work) |
| `running` | `succeeded` | `complete` after a succeeded outcome |
| `running` | `failed` | `complete` after a failed, timed-out, budget-exceeded, or fail-closed channel outcome |
| `running` | `parked` | `park` |
| `parked` | `succeeded` | `complete` after a succeeded outcome (recovery of leftover Work) |
| `parked` | `failed` | `complete` after a failed, timed-out, budget-exceeded, or fail-closed channel outcome (recovery) |

- **[TESTED]** A transition MUST be one of the rows above, or a domain error.

## Operations

The CLI exposes `create`, `next`, `start`, and `park`; the localhost Web intake
also exposes the narrow `create` operation. `complete` is an internal
control-plane operation, not a user command. Their rules:

- **[TESTED]** `create` MUST persist a new Work as `ready` with a `Created` event, atomically, and MUST NOT spawn a Worker.
- **[TESTED]** `next` MUST select Work according to the store and the FSM, not by asking a model which item or phase to take.
- **[TESTED]** First-slice `next` MUST return the oldest `ready` Work, else the oldest `parked` Work. It MUST NOT select `running`, `succeeded`, or `failed`. It MUST NOT spawn a Worker.
- **[TESTED]** `start` MUST bind a Workspace and spawn a Worker only when the Work is `ready` or `parked` and the FSM allows it. The Work then becomes `running`. The goal is written into the workspace as data. Channel retry re-spawns inside this invocation; the retry limit is snapshotted. Channel park uses `park`, not `complete`.
- **[TESTED]** After the Worker process exits with a closed outcome, `start` MUST apply `complete`. The CLI happy path is one `start` invocation: spawn, wait, record.
- **[TESTED]** `complete` MUST apply a closed outcome and persist status plus event atomically, following the table above.
- **[TESTED]** A CLI `complete --file` command MUST NOT exist: a user-provided or workspace-shared outcome has no provenance and cannot recover Work.
- **[TESTED]** If a workspace contains an outcome artifact, `start` MUST reject it. Attempt-scoped protected recovery artifacts are specified by ADR 0005 and remain pending.
- **[TESTED]** Repeating `next`, internal `complete`, or `park` on the same Work MUST NOT duplicate effects. `start` on already-`running` or terminal Work is an illegal transition.
- **[TESTED]** `park` MUST pause without losing progress: reach a save point, leave the Worker slot, and leave the Work `parked`.
- **[UNTESTED]** Live `park` uses a durable checkpoint request consumed by an active supervisor. The present foreground CLI exposes only the historical parked transition while this protocol is being completed.
- **[TESTED]** SQLite runs in WAL mode without a global data-directory lock, so unrelated Work is not blocked. Per-Work capture/CAS is still required before concurrent `start` is claimed safe.
- **[TESTED]** `park` MUST NOT be abort. Abort, timeout, and hang MUST terminate the process group without treating that path as a save-point pause. `park` MUST reach a save point; abort MUST NOT be required to.
- **[TESTED]** Parked Work MUST NOT spin, poll, or occupy a Worker slot.
- **[TESTED]** An answer to a park MUST continue the same Work. It MUST NOT create a new Work.
- **[UNTESTED]** Every Work lifecycle MUST be interruptible by an operator.
- **[UNTESTED]** Irreversible actions MUST require explicit consent. An automatic mode, if any, MUST be an explicit choice.

## Persistence (source of truth)

- **[TESTED]** Current Work status MUST live in exactly one place: Workengine's store.
- **[ENFORCED]** Trackers, boards, chats, and git forges MUST NOT be the source of truth for Work status.
- **[TESTED]** History MUST be reconstructable. Recorded events MUST NOT be edited or deleted.
- **[TESTED]** Updating Work status and appending the corresponding event MUST be one atomic operation.
- **[TESTED]** Between Worker runs, status MUST travel through persistent artifacts, not by continuing a model dialogue.
- **[TESTED]** Every long-lived artifact MUST carry `schemaVersion`.
- **[TESTED]** On Workengine restart, Work in an unconfirmed state MUST return to the queue automatically (resume from the failure point, not from the beginning of the Work).
- **[TESTED]** An unconfirmed `running` Work is parked by recovery. There is no `running` → `ready` transition. A workspace directory is not a trusted checkpoint or outcome authority.

## Publication and observation

- **[ENFORCED]** Effects visible outside Workengine MUST be performed by Workengine, not by the Worker process.
- **[UNTESTED]** Publish adapters MUST be best-effort. A channel failure MUST NOT roll back the source of truth.
- **[TESTED]** Any two observers MUST see the same snapshot of a given Work.
- **[TESTED]** A subscription to the event stream MUST be resumable without loss and without duplicates. The cursor is exclusive, and `Last-Event-ID` takes precedence when both resume mechanisms are present.
- **[TESTED]** A Work summary MUST be structured. Operators MUST NOT have to parse raw logs to know status.
- **[TESTED]** Output of every Workengine-supervised subprocess, including the Worker, MUST use one streaming format.
- **[TESTED]** Each record in that stream MUST carry a mandatory set of accounting fields, including `work_id`.
- **[UNTESTED]** Navigation from a summary to raw artifacts MUST be a finite number of steps. Each significant step MUST leave a visible proof artifact.

## Operator park notifications

- **[UNTESTED]** Parking caused by external input MUST be accompanied by a targeted notification.
- **[UNTESTED]** That notification is a publish-port concern. It MUST NOT write status itself.

## Configuration

- **[TESTED]** Secrets in configuration MUST be stored by reference, never as inline values.
- **[TESTED]** Configuration MUST be validated lazily: a broken part MUST NOT block unrelated Work. `--config-dir` loads only the selected `<profile>.toml`.
- **[TESTED]** Changing configuration MUST NOT rewrite the rules of Work that is already in flight. First-slice retry limit and checkout are snapshotted at `start`.

## Capture and scale

These are not the first vertical slice. They remain invariants of the product.

- **[UNTESTED]** Several Workers MAY drain a queue only through a capture protocol that excludes conflicting writers.
- **[UNTESTED]** Workengine MUST serve several independent codebases without shared Work state between them.
- **[UNTESTED]** A shared external resource with a quota MUST be accounted centrally, not locally by each instance.
- **[UNTESTED]** Slowdown of one external dependency MUST NOT block work against others.

## Infrastructure mutations

- **[UNTESTED]** Remote mutations (for example git compare-and-swap) MUST use compare-and-swap semantics, not blind overwrite.
- **[UNTESTED]** Mutating a codebase MUST happen only in an explicitly expected context. A mismatch MUST stop the operation.
- **[UNTESTED]** Secrets that appear in logs or external output MUST be scrubbed before publication.

## Truth in code

- **[UNTESTED]** When prose in this repository disagrees with the actual behaviour of the code, the code wins. The spec MUST then be updated in the same change, or the code MUST be fixed to match the spec.

## Testability

- **[TESTED]** Every external dependency MUST sit behind a port with a substitutable implementation.
- **[ENFORCED]** Domain tests MUST run without network, filesystem side effects, or an LLM.

## Related

- Work: [work.md](work.md)
- Worker: [worker.md](worker.md)
- Workspace: [workspace.md](workspace.md)
- Source of truth: [../adr/0002-source-of-truth-is-internal.md](../adr/0002-source-of-truth-is-internal.md)
- Fitness: [../architecture/fitness.md](../architecture/fitness.md)
