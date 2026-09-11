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

Start the local daemon, then create a Work and run the built-in stub Worker
from another shell:

```bash
./target/release/workengine serve

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
3. `start` asks the daemon to bind a workspace, write the goal, run the Worker,
   and confirm its result.
4. `park` waits for a validated attempt checkpoint; `abort` tears the process
   group down immediately and records `aborted` as the terminal reason.
5. On restart the daemon tears down any exactly identified orphan process and
   returns its unconfirmed Work to `parked` from the last confirmed point. A
   shared `outcome.json` is never trusted as a recovery authority.

Workengine never asks a model which Work to take, or which status comes next.
Agent and tracker names stay in configuration and adapters, not in the core.

## Commands

| Command | What it does |
| --- | --- |
| `create --goal "<text>" [--profile stub]` | Persist a new Work as `ready` |
| `next` | Print the next startable Work id |
| `start --work <id>` | Ask the daemon to bind, run, and confirm Work |
| `resume --work <id>` | Continue parked Work under the same execution |
| `park --work <id>` | Request a checkpoint, then park running Work |
| `abort --work <id>` | Tear down running Work without treating it as parked |
| `answer --work <id> --answer "<text>"` | Record an answer and resume the same Work |
| `consent --work <id> --action "<text>"` | Record explicit consent and resume the same Work |
| `digest-rootfs --rootfs <path>` | Calculate a Bubblewrap runtime tree digest |
| `serve [--port 9410]` | Run the localhost daemon, Web operator, and observer |
| `version` | Print `workengine <semver>` |

`--data-dir` (or `WORKENGINE_DATA_DIR`) chooses the SQLite store and workspace
directories. Default: `.workengine`.

Lifecycle commands use `--daemon-port` (or `WORKENGINE_DAEMON_PORT`) and require
the matching local `serve` process. `serve` binds only to `127.0.0.1` and
`::1`. Its browser UI and `/api/v0` API
never read SQLite directly. The UI can create and start `ready` Work, resume
parked Work, checkpoint-park or abort an active attempt, and record answers or
explicit consent. It cannot supply argv, secrets, sandbox, workspace, or budget
settings. Each Work record includes its immutable
execution configuration, attempt/heartbeat/retry history, terminal diagnostics,
redacted process metadata, and a finite catalogue of control-plane proof
artifacts. Raw child output and workspace paths are not exposed. The API remains
intentionally internal.

`--config-dir` (or `WORKENGINE_CONFIG_DIR`) is a directory of Worker profiles:
one `<profile>.toml` file per profile. That makes validation truly lazy: a bad
unrelated profile cannot block a Work. The former monolithic `--config` is
retained temporarily for compatibility.

## Worker profiles

The default profile is `stub`. Any other name is configuration: the argv to
spawn, secret-file references, an optional checkout path. Point that argv at Cursor,
Claude, Codex, or whatever CLI you already run — it is a profile file, not a
vendor `if` in the core.

```toml
[profile.coder]
argv = ["my-agent", "--print"]
retry_limit = 2
checkout = "/src"
sandbox = { type = "bubblewrap", rootfs = "/opt/workengine/rootfs", digest = "sha256:<64 lowercase hex>", seccomp = "/etc/workengine/worker.bpf", seccomp_digest = "sha256:<64 lowercase hex>" }

[profile.coder.policy]
memory_bytes = 1073741824
max_processes = 64
max_open_files = 256
max_file_bytes = 1073741824
cpu_seconds = 900
# Optional. Without this, the Worker has no egress capability.
# egress_broker_socket = "/run/workengine/egress.sock"

[profile.coder.secret_file]
API_TOKEN = { fromEnv = "API_TOKEN" }
```

Secrets are references (`fromEnv`), never inline values. The Worker receives
only a transient read-only path in `API_TOKEN_FILE`, never a secret value in
its environment. A user Worker also
needs an explicit verified `bubblewrap` runtime root or digest-pinned `oci`
image and a seccomp profile; profiles without them fail closed. Use
`workengine digest-rootfs --rootfs /opt/workengine/rootfs` to calculate the
Bubblewrap digest. `seccomp_digest` is the ordinary SHA-256 of the seccomp file
bytes. Bubblewrap consumes a compiled BPF seccomp program; OCI consumes the
engine's JSON seccomp profile. Resource limits have finite
defaults and can be tightened or explicitly changed in the policy table. Both
backends drop every capability and have no direct network. Networked harnesses
must speak to the explicitly mounted Unix broker socket named by
`WORKENGINE_EGRESS_SOCKET`. A broken profile does not block Work that uses a
different one.

## What this is not

Workengine is not an agent, not an LLM supervisor, and not a tracker client.

- The Worker is a spawned process, not a chat, a skill, or an IDE-hosted loop.
- The agent harness stays where it belongs: inside the CLI you spawn.
- A board can show a copy. It is not the source of truth.
- Workers do not talk to each other. One writer per Work — this is not a swarm.

## Status

Workengine is pre-1.0. What you can do today is the local live-operator slice
above. Inbound adapters and publishers come later. The exact released version is
reported by `workengine version` and by the Git tag.

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
| Product direction and release milestones | [roadmap.md](docs/roadmap.md) |
| Released changes | [CHANGELOG.md](CHANGELOG.md) |
| Secrets, process kill, containment | [threat model](docs/product/threat-model.md) |
| How to change this repo | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Entry for coding agents | [AGENTS.md](AGENTS.md) |

Before you finish a change, run `just check`. That is what CI runs.

## License

Apache License 2.0. See [LICENSE](LICENSE).
