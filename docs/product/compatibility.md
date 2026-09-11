# Compatibility charter

Exit codes and schemas are binding for this runtime slice. A breaking change is a major version.

Workengine versions with SemVer. A tag `vX.Y.Z` is created only by a reviewed
release change; ordinary commits never create tags directly.

The default-branch history is the human changelog of the contracts below. Message format, atomicity, and squash policy are contribution process ([../../CONTRIBUTING.md](../../CONTRIBUTING.md)), not a SemVer surface and not an architecture decision. Schema compatibility is this file.

## Covered by SemVer

These are public contracts. A breaking change is a major version, plus a migration if the store is involved.

| Surface | Compatibility |
| --- | --- |
| CLI command names (`create`, `next`, `start`, `park`, `serve`, `version`) | Stable |
| CLI `version` prefix | `workengine <semver>` where semver is `CARGO_PKG_VERSION` |
| CLI flags and env vars documented in `--help` | Stable (`--config-dir` / `WORKENGINE_CONFIG_DIR`, `start --checkout`) |
| Process exit codes | Stable; see table below |
| Outcome JSON schema (`schemaVersion`) | Stable within a major; unknown fields must fail closed or be reserved |
| Store artifact schema (`schemaVersion`) | Store v2 rejects v1 directories without mutation; operators create a new data directory |
| Workspace memory line schema (`schemaVersion`) | Stable within a major |
| Supervised subprocess stream record (`schemaVersion`, camelCase) | Stable within a major; same JSON dialect as outcome and event |
| Domain status and outcome enumerations that appear in those schemas (`ready`, `running`, `succeeded`, `failed`, `parked`; outcome kinds in [../spec/worker.md](../spec/worker.md)) | Stable |

## Not covered

Changing these is not a SemVer break.

| Surface | Notes |
| --- | --- |
| Log line text | Structured fields (`workId`, `event`, `schemaVersion`) are the contract, not the message |
| Optional ` (<git sha>)` on `version` | Forensic build id when compiled from git; not a version scheme |
| JSON key order | Parsers MUST NOT depend on order |
| Internal crate and module names | `workengine-domain` is not a public library API |
| Adapter internals, SQL without `schemaVersion` | Private |
| `/api/v0` local API and `contracts/observer.openapi.yaml` | Explicitly internal until durable execution permits a public v1 |
| Help wording, man page prose | Behaviour and flags matter |
| Worker profile names (delivery phases) | Configuration, versioned separately from the core |

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Work completed with a failed outcome |
| 2 | Usage / invocation error |
| 10 | Illegal transition |
| 11 | Store conflict / capture lost |
| 20 | Budget exceeded |
| 21 | Worker hang / timeout |
| 30 | Outcome schema error |
| 40 | Workspace error |

Exact numbers freeze with this slice. Do not treat "non-zero" as a single failure class.

## Schema evolution

- Every long-lived artifact carries `schemaVersion`.
- Readers MUST reject an unknown version rather than guess.
- Additive optional fields MAY appear in a minor version if unknown fields on write are not silently dropped from the store.
- Removing or reinterpreting a field is a major version.
- SQLite `user_version=1` is intentionally unsupported by this v2 runtime and
  is never migrated or deleted automatically.

## Release process

The repository uses one version for the workspace, the CLI binary, Git tags,
GitHub Releases, and `CHANGELOG.md`. The current version lives in the root
`Cargo.toml`; tags use the matching `vX.Y.Z` form.

[Release Please](https://github.com/googleapis/release-please) reads the
Conventional Commit history on `master`. It opens or updates one release pull
request with the workspace version and generated changelog. Merging that pull
request creates the matching tag and the corresponding GitHub Release. It does
not publish packages or binaries by itself.

Before `1.0.0`, Workengine follows the SemVer convention that incompatible
changes advance the minor version. Release Please is configured accordingly:

| Change on `master` | Next version |
| --- | --- |
| `fix(scope): ...` | Patch (`0.1.0` → `0.1.1`) |
| `feat(scope): ...` | Minor (`0.1.0` → `0.2.0`) |
| `type(scope)!: ...` or `BREAKING CHANGE:` | Minor while pre-1.0; major after `1.0.0` |
| `docs`, `test`, `refactor`, `chore`, `ci`, `build` only | No release by themselves |

Use `Release-As: X.Y.Z` in a reviewed commit only when an exceptional explicit
version is needed. Do not manually edit the version, manifest, generated release
section, tag, or GitHub Release during normal development.

## Related

- Spec: [../spec/workflow.md](../spec/workflow.md), statuses in [../spec/work.md](../spec/work.md)
- Commits: [../../CONTRIBUTING.md](../../CONTRIBUTING.md)
- Product milestones: [../roadmap.md](../roadmap.md)
- Released changes: [../../CHANGELOG.md](../../CHANGELOG.md)
