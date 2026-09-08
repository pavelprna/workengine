# 0002. Source of truth is internal

- Status: Accepted
- Date: 2026-09-08

## Context

Boards, forges, and chats are convenient to treat as status. That makes Workengine a client of someone else's column names, race conditions, and APIs. Resume, idempotency, and a single writer become impossible to guarantee.

## Decision

Workengine owns the source of truth:

- Current Work status lives in Workengine's store.
- History is an append-only event log.
- Status update and event append are one atomic operation.
- Replay reconstructs status.
- Trackers and git forges may be inbound sources or best-effort publish targets. They are not the store.

Inbound and Publisher are ports. They are absent from the first vertical slice so that the store is proven before any board is attached.

## Consequences

- Operators looking only at a board can see a lagging copy. The CLI and the store are authoritative.
- Publish failure never rolls back a completed transition.
- Store schema is a product contract (`schemaVersion`, compatibility charter). A breaking store change is a major version and a migration.
- Domain tests do not need a tracker.
