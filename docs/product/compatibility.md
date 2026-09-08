# Compatibility charter

Draft. Exit codes and schemas become binding when `complete` exists. Until then this file is the intended contract, not a promise of a released binary.

Workengine versions with SemVer. A tag `vX.Y.Z` is a product decision, not an automatic side effect of a commit.

The default-branch history is the human changelog of the contracts below. Message format, atomicity, and squash policy are contribution process ([../../CONTRIBUTING.md](../../CONTRIBUTING.md)), not a SemVer surface and not an architecture decision. Schema compatibility is this file.

## Covered by SemVer

These are public contracts. A breaking change is a major version, plus a migration if the store is involved.

| Surface | Compatibility |
| --- | --- |
| CLI command names (`create`, `next`, `start`, `complete`, `park`, `version`) | Stable |
| CLI flags and env vars documented in `--help` | Stable |
| Process exit codes | Stable; see table below |
| Outcome JSON schema (`schemaVersion`) | Stable within a major; unknown fields must fail closed or be reserved |
| Store artifact schema (`schemaVersion`) | Stable within a major; breaking change requires a migrator |
| Domain status and outcome enumerations that appear in those schemas (`ready`, `running`, `succeeded`, `failed`, `parked`; outcome kinds in [../spec/worker.md](../spec/worker.md)) | Stable |

## Not covered

Changing these is not a SemVer break.

| Surface | Notes |
| --- | --- |
| Log line text | Structured fields (`work_id`, `event`, `schema_version`) are the contract, not the message |
| JSON key order | Parsers MUST NOT depend on order |
| Internal crate and module names | `workengine-domain` is not a public library API |
| Adapter internals, SQL without `schemaVersion` | Private |
| Help wording, man page prose | Behaviour and flags matter |
| Worker profile names (delivery phases) | Configuration, versioned separately from the core |

## Exit codes (draft)

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

Exact numbers freeze when the CLI first ships `complete`. Do not treat "non-zero" as a single failure class.

## Schema evolution

- Every long-lived artifact carries `schemaVersion`.
- Readers MUST reject an unknown version rather than guess.
- Additive optional fields MAY appear in a minor version if unknown fields on write are not silently dropped from the store.
- Removing or reinterpreting a field is a major version.

## Related

- Spec: [../spec/workflow.md](../spec/workflow.md), statuses in [../spec/work.md](../spec/work.md)
- Commits: [../../CONTRIBUTING.md](../../CONTRIBUTING.md)
