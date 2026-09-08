# Workflow

Workflow is the process that moves Work: a finite state machine, a single writer, persistence, resume, and publication. Delivery-phase names for a given team are Worker profiles, not this document.

This document is the canonical behaviour of Workflow. Keywords follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119). Every `MUST` / `MUST NOT` is `[UNTESTED]` until a test or compile-time check in this repository covers it.

## Transitions

- **[UNTESTED]** Allowed transitions between Work statuses MUST be a finite state machine known in advance.
- **[UNTESTED]** An illegal transition MUST be a domain error. Workengine MUST NOT silently coerce, skip, or invent a status.
- **[UNTESTED]** Decisions about transitions MUST be taken by Workengine code, not by an LLM.
- **[UNTESTED]** A Worker MUST NOT choose the next Work status.
- **[UNTESTED]** Parallelism MUST be between Work items. Inside one Work there is one status writer.

First-slice transitions. Status names are defined in [work.md](work.md). Workengine maps a closed Worker outcome onto a path; the Worker does not choose the status.

| From | To | How |
| --- | --- | --- |
| `ready` | `running` | `start` |
| `parked` | `running` | `start` (continue the same Work) |
| `running` | `succeeded` | `complete` after a succeeded outcome |
| `running` | `failed` | `complete` after a failed, timed-out, budget-exceeded, or fail-closed channel outcome |
| `running` | `parked` | `park` |

- **[UNTESTED]** A transition MUST be one of the rows above, or a domain error.

## Operations

The first CLI slice exposes `next`, `start`, `complete`, and `park`. Their rules:

- **[UNTESTED]** `next` MUST select Work according to the store and the FSM, not by asking a model which item or phase to take.
- **[UNTESTED]** `start` MUST bind a Workspace and spawn a Worker only when the Work is `ready` or `parked` and the FSM allows it. The Work then becomes `running`.
- **[UNTESTED]** `complete` MUST apply a closed outcome and persist status plus event atomically, following the table above.
- **[UNTESTED]** `complete` MUST be idempotent: repeating the same completion on the same Work MUST NOT duplicate effects.
- **[UNTESTED]** `park` MUST pause without losing progress: reach a save point, leave the Worker slot, and leave the Work `parked`.
- **[UNTESTED]** Parked Work MUST NOT spin, poll, or occupy a Worker slot.
- **[UNTESTED]** An answer to a park MUST continue the same Work. It MUST NOT create a new Work.
- **[UNTESTED]** Every Work lifecycle MUST be interruptible by an operator.
- **[UNTESTED]** Irreversible actions MUST require explicit consent. An automatic mode, if any, MUST be an explicit choice.

## Persistence (source of truth)

- **[UNTESTED]** Current Work status MUST live in exactly one place: Workengine's store.
- **[UNTESTED]** Trackers, boards, chats, and git forges MUST NOT be the source of truth for Work status.
- **[UNTESTED]** History MUST be reconstructable. Recorded events MUST NOT be edited or deleted.
- **[UNTESTED]** Updating Work status and appending the corresponding event MUST be one atomic operation.
- **[UNTESTED]** Between Worker runs, status MUST travel through persistent artifacts, not by continuing a model dialogue.
- **[UNTESTED]** Every long-lived artifact MUST carry `schemaVersion`.
- **[UNTESTED]** On Workengine restart, Work in an unconfirmed state MUST return to the queue automatically (resume from the failure point, not from the beginning of the Work).

## Publication and observation

- **[UNTESTED]** Effects visible outside Workengine MUST be performed by Workengine, not by the Worker process.
- **[UNTESTED]** Publish adapters MUST be best-effort. A channel failure MUST NOT roll back the source of truth.
- **[UNTESTED]** Any two observers MUST see the same snapshot of a given Work.
- **[UNTESTED]** A subscription to the event stream MUST be resumable without loss and without duplicates.
- **[UNTESTED]** A Work summary MUST be structured. Operators MUST NOT have to parse raw logs to know status.
- **[UNTESTED]** Each Worker run MUST emit a mandatory set of accounting fields, including `work_id`.
- **[UNTESTED]** Navigation from a summary to raw artifacts MUST be a finite number of steps. Each significant step MUST leave a visible proof artifact.

## Operator park notifications

- **[UNTESTED]** Parking caused by external input MUST be accompanied by a targeted notification.
- **[UNTESTED]** That notification is a publish-port concern. It MUST NOT write status itself.

## Configuration

- **[UNTESTED]** Secrets in configuration MUST be stored by reference, never as inline values.
- **[UNTESTED]** Configuration MUST be validated lazily: a broken part MUST NOT block unrelated Work.
- **[UNTESTED]** Changing configuration MUST NOT rewrite the rules of Work that is already in flight.

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

- **[UNTESTED]** Every external dependency MUST sit behind a port with a substitutable implementation.
- **[UNTESTED]** Domain tests MUST run without network, filesystem side effects, or an LLM.

## Related

- Work: [work.md](work.md)
- Worker: [worker.md](worker.md)
- Workspace: [workspace.md](workspace.md)
- Source of truth: [../adr/0002-source-of-truth-is-internal.md](../adr/0002-source-of-truth-is-internal.md)
- Fitness: [../architecture/fitness.md](../architecture/fitness.md)
