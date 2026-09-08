# 0003. Worker is a process

- Status: Accepted
- Date: 2026-09-08

## Context

Coding agents already have a harness (inner loop: tools, context, ReAct). Hosting that loop inside Workengine — as an SDK, an IDE task, or a cloud agent that owns the session — would make the model the outer loop. Workengine would become a chat supervisor.

Killing only the parent PID leaves grandchild processes (the actual agent) running after timeout.

## Decision

A Worker is an isolated OS process behind `WorkerRunner`.

- Workengine is not an agent and not a harness. It spawns a harness.
- The channel is a versioned, machine-readable outcome. Free text is not a command.
- Outcome kinds are a closed enum.
- Budget and hang detection belong to Workengine, outside the child.
- The child is placed in its own process group. Abort signals the group, then kills it.
- Workengine does not ask a model which Work or which next status to take.
- A fake/stub Worker that writes a valid outcome is a first-class adapter, not a test-only toy.

IDE agents, cloud agents, and in-process LLM clients are not Workers.

## Consequences

- A new coding agent is a new adapter, not a domain change.
- Timeout tests must prove group teardown, even when the first stub has no children.
- No `if agent == "…"` in domain or application.
- Cursor / Claude / Codex as host of the Workengine loop is a product violation, not a convenience.
