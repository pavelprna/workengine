# Workengine

Workengine is a control plane for coding agents: durable runtime of **Work**. The outer loop is code. A Worker is a spawned process, not this session.

This file is the agent entry. It does not copy the spec.

## Read first

| When you touch | Read |
| --- | --- |
| Behaviour, FSM, outcomes | `docs/spec/` |
| Crates, layers, ports | `docs/architecture/overview.md`, `docs/architecture/ports.md` |
| Why a shape exists | `docs/adr/INDEX.md` then the matching ADR |
| SemVer, exit codes, schemas | `docs/product/compatibility.md` |
| Secrets, process kill, containment | `docs/product/threat-model.md` |
| Commits | `CONTRIBUTING.md` |

## Invariants

- Dependencies point inward: adapters → application → domain. Domain has no I/O and no product names.
- Workengine is the only writer of Work status. Workers, boards, and this chat do not write the source of truth.
- Transition decisions are code, not the model. Do not add "ask the agent what phase is next".
- Worker = OS process + process group + versioned outcome. Do not host the loop in an IDE task or SDK.
- Secrets in config are references, never inline values.
- Architecture changes: ADR first, then code. Checkable decisions get a row in `docs/architecture/fitness.md`.

## Check

`just check` is mandatory before you finish. It is what CI will run. Do not invent a second toolchain or spec process.

## Commits

Follow `CONTRIBUTING.md`.

## Not in this repository

Vendor IDE rule folders, nested `AGENTS.md`, an external spec runner, a tracker as source of truth, tokio in domain/application, a fifth domain entity named Workengine.
