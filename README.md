# Workengine

Control plane for coding agents: a Work runtime whose steps are CLI agents. Code owns the outer loop; trackers and harnesses are adapters.

Workengine is not an agent, not an LLM supervisor, and not a tracker client. It is the only writer of Work status and the only component that starts a Worker or publishes external effects.

## Domain

The core knows four nouns. Everything else is an adapter.

| Entity | Role |
| --- | --- |
| **Work** | Unit of durable execution: stable id, closed statuses (`ready`, `running`, `succeeded`, `failed`, `parked`), first-slice attributes (goal, Worker profile) |
| **Worker** | Agent as a process: versioned outcome, closed kind, budget and hang detected outside the child |
| **Workspace** | Isolated copy for the life of one Work; containment; append-only memory after a confirmed outcome |
| **Workflow** | Finite state machine, single writer, idempotent operations, resume, publish by the system |

If domain code branches on an agent or tracker product name, the layer is wrong.

## CLI

The binary name is `workengine`. `workengine --help` and `workengine version` exist. The first runtime slice:

```
workengine create --goal "<text>" [--profile stub]
workengine next
workengine start --work <id> [--checkout <dir>]
workengine complete --work <id> --file outcome.json
workengine park --work <id>
```

`--data-dir` (or `WORKENGINE_DATA_DIR`) selects the SQLite store and workspace directories. Default: `.workengine`. `--config` (or `WORKENGINE_CONFIG`) is a TOML file of Worker profiles. Env values in that file are `{ fromEnv = "NAME" }` only.

`start` is the happy path: bind a workspace, write the goal, optionally copy `--checkout`, spawn the Worker, wait, and apply `complete`. Default Worker is the stub (`--profile stub`). A TOML `--config` (or `WORKENGINE_CONFIG`) supplies argv and env references for any other profile name. `complete --file` recovers leftover `running` or `parked` Work from an outcome artifact (including after `next` auto-parked a crash). The CLI does not ask a model which Work or which next status to take.

Crate layers: `workengine-domain` through `workengine-cli`. Run `just check` as in [CONTRIBUTING.md](CONTRIBUTING.md).

## What this is not

- A chat, skill, or IDE-hosted loop
- An agent harness (that remains the inner loop of the spawned CLI)
- A client of a board that treats columns as source of truth
- A multi-agent swarm (Workers do not talk to each other; one writer per Work)

## Documents

| Document | What it owns |
| --- | --- |
| [docs/spec](docs/spec/work.md) | Behaviour (`MUST` / `MUST NOT`) |
| [docs/architecture/overview.md](docs/architecture/overview.md) | Crates and dependency direction |
| [docs/architecture/ports.md](docs/architecture/ports.md) | Port contracts |
| [docs/architecture/fitness.md](docs/architecture/fitness.md) | What CI must be able to fail |
| [docs/adr/INDEX.md](docs/adr/INDEX.md) | Why a shape was chosen |
| [docs/product/compatibility.md](docs/product/compatibility.md) | SemVer, schemas, exit codes |
| [docs/product/threat-model.md](docs/product/threat-model.md) | Threat model |
| [AGENTS.md](AGENTS.md) | Entry for coding agents |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Commits, rebase, how to check |

## License

Apache License 2.0. See [LICENSE](LICENSE).
