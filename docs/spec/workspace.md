# Workspace

A Workspace is the isolated copy of a codebase that belongs to one Work for that Work's entire life.

This document is the canonical behaviour of Workspace. Keywords follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119). Every `MUST` / `MUST NOT` is `[UNTESTED]` until a test or compile-time check in this repository covers it.

## Isolation

- **[UNTESTED]** Each Work MUST receive its own isolated workspace and its own isolated working copy of source.
- **[UNTESTED]** Two Work instances MUST NOT share a workspace directory.
- **[UNTESTED]** A Workspace MUST be uniquely addressable from its Work for the whole life of that Work.

## Containment

- **[UNTESTED]** A Worker MUST NOT be able to access paths outside the workspace root that Workengine assigned.
- **[UNTESTED]** The first runtime slice MAY implement containment as a dedicated directory. Later adapters MAY use git worktrees or containers behind the same `WorkspaceFactory` port.
- **[UNTESTED]** Workengine's own operations on a codebase MUST be isolated from user-supplied environment settings that a Worker could change.

## Lifecycle

- **[UNTESTED]** A Workspace MUST NOT be deleted automatically when a Work fails. After failure it is a diagnostic artifact, not garbage.
- **[UNTESTED]** Operator-driven cleanup of a workspace MUST be a separate, explicit action.

## Memory

- **[UNTESTED]** A workspace MAY keep cumulative memory. That memory is a property of the workspace, not of a single Work run.
- **[UNTESTED]** Workspace memory MUST be append-only.
- **[UNTESTED]** Workengine MUST write to workspace memory only after a confirmed Work outcome. A still-running or aborted Worker MUST NOT be treated as confirmed.

## Related

- Work: [work.md](work.md)
- Worker: [worker.md](worker.md)
- Workflow: [workflow.md](workflow.md)
- Ports: [../architecture/ports.md](../architecture/ports.md)
