# Worker

A Worker is an agent as a process. Workengine starts it, bounds it, and reads a typed outcome. The Worker does not own the outer loop.

This document is the canonical behaviour of Worker. Keywords follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119). Every `MUST` / `MUST NOT` is `[UNTESTED]` until a test or compile-time check in this repository covers it.

## Abstraction

- **[UNTESTED]** A Worker MUST be reachable only through the `WorkerRunner` port.
- **[UNTESTED]** The domain and application crates MUST NOT depend on a concrete agent binary, SDK, or vendor crate.
- **[UNTESTED]** Choosing which Worker configuration to run for a Work MUST be a deterministic function of Work attributes, not a judgement by an LLM.

## Process

- **[UNTESTED]** A Worker MUST run as an isolated operating-system process (or a containment boundary that still appears as a process to the runner).
- **[UNTESTED]** Workengine MUST NOT use an in-process agent host (chat session, IDE task, cloud agent API that owns the loop) as the Worker.
- **[UNTESTED]** The runner MUST place the Worker in its own process group.
- **[UNTESTED]** On timeout, hang, or abort, the runner MUST signal the process group (terminate, then kill), not only the parent PID.
- **[UNTESTED]** The core MUST remain synchronous: spawn, wait, record. The domain and application crates MUST NOT depend on an async runtime.

## Contract

- **[UNTESTED]** The channel between Workengine and a Worker MUST be machine-readable.
- **[UNTESTED]** Free text from a Worker MUST NOT be interpreted as a control command (next status, publish, park, complete).
- **[UNTESTED]** A Worker response MUST conform to an explicit schema with `schemaVersion`.
- **[UNTESTED]** Every Worker run MUST return an outcome from a closed set of kinds. Unknown kinds MUST be a schema error.
- **[UNTESTED]** The first runtime slice MUST include at least these outcome kinds: succeeded, failed, timed out / hung, budget exceeded, and a classified channel error.

## Budget and hang

- **[UNTESTED]** Each Worker run MUST have a hard budget (time, and later tokens or other meters the profile declares).
- **[UNTESTED]** Accounting of spend and remaining budget MUST be Workengine's duty, not the Worker's.
- **[UNTESTED]** Workengine MUST detect a hung Worker independently of the Worker process (watchdog outside the child).
- **[UNTESTED]** Channel errors toward a model MUST be classified by the reaction Workengine will take (retry, fail, park), not by parsing prose.

## Isolation and safety

- **[UNTESTED]** A Worker's permission profile MUST be deny-by-default. Extra rights require an explicit enable.
- **[UNTESTED]** A Worker MUST NOT be able to perform known-destructive actions against the host environment (for example wipe paths outside its workspace).
- **[UNTESTED]** A Worker MUST receive only the minimum resources and secrets required for that run.
- **[UNTESTED]** Secrets passed to a Worker MUST NOT leak through Workengine logs or process inspection surfaces that Workengine controls.
- **[UNTESTED]** Text from untrusted sources (inbound tickets, web, prior artifacts) MUST be treated as data, not as instructions to Workengine.

## Authorship

- **[UNTESTED]** Every artifact a Worker produces MUST carry explicit authorship: which Worker configuration produced it.

## Related

- Work: [work.md](work.md)
- Workspace: [workspace.md](workspace.md)
- Workflow: [workflow.md](workflow.md)
- Ports: [../architecture/ports.md](../architecture/ports.md)
- Decision: [../adr/0003-worker-is-a-process.md](../adr/0003-worker-is-a-process.md)
