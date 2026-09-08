# Workengine

A durable runtime for coding agents.

You give Workengine a goal. It starts a **Worker** — a real OS process, with a
budget and a private workspace — and records a typed outcome. Status lives
here, not on a board and not in a chat.

The outer loop is code. The agent is a child process, not this session.

## Why it exists

Coding agents already know how to edit files. What they usually lack is a
control plane: something that owns the lifecycle, survives a crash, and does
not ask the model which phase comes next.

A tracker column is a lagging copy. An IDE chat is not a runtime. Workengine
sits in between. **Work** is the unit of durable execution, and Workengine is
the only writer of its status.

## Try it

Rust is pinned in `rust-toolchain.toml`. With [rustup](https://rustup.rs/)
installed, the right toolchain is picked up for you.

```bash
cargo build --release
./target/release/workengine version
```

Create a Work, then run the built-in stub Worker:

```bash
./target/release/workengine create --goal "say hello"
# → <id> ready

./target/release/workengine start --work <id>
# → <id> succeeded
```

The stub writes a valid outcome and exits. It is a real Worker, not a test
double: enough to walk the happy path before you plug in an agent CLI.

`workengine --help` lists every command.

## How it works

Four nouns. Everything else is an adapter.

| Noun | Role |
| --- | --- |
| **Work** | A durable job: stable id, a goal, a Worker profile, and a status |
| **Worker** | An agent as a process. Workengine spawns it, watches the budget, and reads the outcome |
| **Workspace** | A private copy for the life of one Work. Memory is append-only, and only after a confirmed outcome |
| **Workflow** | The state machine. Transitions are code. Resume is idempotent. One writer |

A Work is `ready`, `running`, `succeeded`, `failed`, or `parked`.

A typical run looks like this:

1. `create` saves the Work as `ready`. Nothing is spawned yet.
2. `next` prints the next startable id.
3. `start` binds a workspace, writes the goal, optionally copies a checkout,
   runs the Worker, and completes.
4. If a run is left `running` or `parked`, `complete --file` or `park` recover
   it — including after `next` auto-parked a crash.

Workengine never asks a model which Work to take, or which status comes next.
Agent and tracker names stay in configuration and adapters, not in the core.

## Commands

| Command | What it does |
| --- | --- |
| `create --goal "<text>" [--profile stub]` | Persist a new Work as `ready` |
| `next` | Print the next startable Work id |
| `start --work <id> [--checkout <dir>]` | Bind a workspace, run the Worker, complete |
| `complete --work <id> --file outcome.json` | Apply an outcome to leftover `running` or `parked` Work |
| `park --work <id>` | Park leftover `running` Work |
| `version` | Print `workengine <semver>` |

`--data-dir` (or `WORKENGINE_DATA_DIR`) chooses the SQLite store and workspace
directories. Default: `.workengine`.

`--config` (or `WORKENGINE_CONFIG`) is a TOML file of Worker profiles.

## Worker profiles

The default profile is `stub`. Any other name is configuration: the argv to
spawn, env references, an optional checkout path. Point that argv at Cursor,
Claude, Codex, or whatever CLI you already run — it is a profile file, not a
vendor `if` in the core.

```toml
[profile.coder]
argv = ["my-agent", "--print"]
retry_limit = 2
checkout = "/src"

[profile.coder.env]
API_TOKEN = { fromEnv = "API_TOKEN" }
```

Secrets are references (`fromEnv`), never inline values. A broken profile does
not block Work that uses a different one.

## What this is not

Workengine is not an agent, not an LLM supervisor, and not a tracker client.

- The Worker is a spawned process, not a chat, a skill, or an IDE-hosted loop.
- The agent harness stays where it belongs: inside the CLI you spawn.
- A board can show a copy. It is not the source of truth.
- Workers do not talk to each other. One writer per Work — this is not a swarm.

## Status

This is `0.0.0`. What you can do today is the CLI above: create, run,
complete, park. Inbound adapters and publishers come later.

Public contracts — command names, exit codes, schemas — live in
[compatibility.md](docs/product/compatibility.md). Behaviour lives in
[docs/spec](docs/spec/work.md). This README is the front door, not a second
spec.

## Learn more

| If you want | Read |
| --- | --- |
| Behaviour (`MUST` / `MUST NOT`) | [docs/spec](docs/spec/work.md) |
| Crates and dependency direction | [architecture overview](docs/architecture/overview.md) |
| Port contracts | [ports.md](docs/architecture/ports.md) |
| What CI must be able to fail | [fitness.md](docs/architecture/fitness.md) |
| Why a shape was chosen | [ADRs](docs/adr/INDEX.md) |
| SemVer, schemas, exit codes | [compatibility.md](docs/product/compatibility.md) |
| Secrets, process kill, containment | [threat model](docs/product/threat-model.md) |
| How to change this repo | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Entry for coding agents | [AGENTS.md](AGENTS.md) |

Before you finish a change, run `just check`. That is what CI runs.

## License

Apache License 2.0. See [LICENSE](LICENSE).
