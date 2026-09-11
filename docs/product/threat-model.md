# Threat model

Short model for a control plane that spawns processes, holds workspace copies, and may later attach trackers. Not a full STRIDE spreadsheet.

## Assets

- Work status and the event log (integrity of the source of truth).
- Workspace contents (source, secrets the Worker was allowed to see, diagnostic artifacts).
- Secrets referenced from configuration (tokens, keys).
- Host filesystem and processes outside the workspace.

## Actors

- Operator (trusted to run the CLI).
- Worker process (untrusted: model-directed, may be prompt-injected).
- Inbound source (untrusted: tickets and comments are data).
- Publish channel (untrusted as a writer; must not feed back into status).

## Boundaries

```
 operator CLI / config
        |
        v
  Workengine process          <-- trusted computing base for SoT
        |
        +-- store (data dir)
        |
        +-- spawn process group --> Worker   <-- untrusted
        |                              |
        |                              v
        +-- workspace directory  <----+
```

## Threats and controls

| Threat | Control |
| --- | --- |
| Worker writes Work status or publishes as if it were the system | Worker is a process; only Workengine writes the store and publish port |
| Prompt injection from a ticket or webpage | Inbound text is data. Free text is never a control command |
| Secret in config committed or logged | Secrets by reference only. Scrub before publish. No values in issues or logs |
| Worker escapes the workspace | User profiles select Bubblewrap or OCI; the runner mounts only the declared runtime and workspace, and fails closed without a backend |
| Timeout or abort kills parent, children remain | Process group terminate then kill (Unix in this slice) |
| Daemon crash leaves an unconfirmed Worker | Protected runtime owner record binds PID, process group, process start identity, Work, execution, and attempt; restart kills only an exact match before reclaiming the lease |
| Worker forges a park or stale checkpoint | Attempt-private control mount; schema and Work/execution/attempt/profile binding validated before `parked` is committed |
| Tracker column treated as status | Internal store is SoT; boards are best-effort copies |
| Workengine phones home | No default telemetry exporter |
| Destructive host actions | Deny-by-default Worker profile; known-destructive actions forbidden |
| Two local clients double-start the same Work | One daemon owns supervisors and the store claims one active attempt transactionally; multi-host capture remains later work |

## Out of scope for this note

- Full container/bwrap design (adapter later, same `WorkspaceFactory`).
- Multi-tenant SaaS isolation.
- Windows process-group teardown and file locking (first slice is Unix).
- Supply-chain of third-party agent binaries (operators choose the binary; Workengine bounds the process).

## Reporting

Do not file secrets, tokens, or workspace dumps in public issues. Describe the class of bug and reproduction without credentials.

## Related

- Worker spec: [../spec/worker.md](../spec/worker.md)
- Workspace spec: [../spec/workspace.md](../spec/workspace.md)
- ADR 0003: [../adr/0003-worker-is-a-process.md](../adr/0003-worker-is-a-process.md)
