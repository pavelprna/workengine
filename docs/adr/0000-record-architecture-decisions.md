# 0000. Record architecture decisions

- Status: Accepted
- Date: 2026-09-08

## Context

Workengine is a long-lived control plane. Layering, persistence, and the Worker boundary will be argued again as adapters appear. Chat history is not a source of truth. A spec-runner outside this repository would become a second process and a second canon.

## Decision

Architecture decisions are recorded in `docs/adr/` using the Nygard template: Context, Decision, Consequences.

- One file, one decision. Filename: `NNNN-short-title.md`.
- `INDEX.md` is the table of number, title, status.
- Spec of behaviour lives in `docs/spec/`. ADRs explain *why* a shape was chosen.
- Contribution process (commits, branch history, license, review ceremony) lives in `CONTRIBUTING.md`. It is not an ADR.
- There is no external specification runner, change-proposal tool, or spec framework in this repository. Changes go through pull requests and, when the shape changes, an ADR.
- Agent instructions do not copy decisions. `AGENTS.md` points here.

## Consequences

- Before changing a layer, a port, the store, or the Worker model, authors read this index and the relevant ADR.
- A checkable architecture decision gets a row in [../architecture/fitness.md](../architecture/fitness.md) in the same change.
- If an ADR is wrong, it is superseded by a new ADR, not edited into a different decision.
