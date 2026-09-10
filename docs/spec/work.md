# Work

Work is the unit of durable execution. Workengine is a runtime for Work, not a fifth domain entity.

This document is the canonical behaviour of Work. Keywords follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119). Every `MUST` / `MUST NOT` is `[UNTESTED]` until a test or compile-time check in this repository covers it. Covered rows use `[TESTED]` or `[ENFORCED]`.

## Identity

- **[TESTED]** A Work MUST have a stable identifier (`WorkId`) that does not change for the lifetime of that Work.
- **[TESTED]** `WorkId` MUST be available at every stage of the lifecycle, including parked and terminal states.
- **[TESTED]** Two distinct Work instances MUST NOT share a `WorkId`.

## Source

- **[UNTESTED]** Work MAY originate from an external source through an inbound adapter, or from the local CLI.
- **[TESTED]** Until an inbound adapter exists, the CLI `create` verb MUST be the way an operator introduces Work. `create` MUST persist the Work as `ready` and MUST NOT spawn a Worker.
- **[TESTED]** The domain MUST NOT name, import, or branch on a concrete inbound product.
- **[UNTESTED]** A record in an external source MUST NOT become Work until an explicit signal exists: a ready state of that record, or an explicit operator command.

Local creation is an intentional product capability, not a temporary substitute
for a required remote source. External inbound adapters add ways to introduce
Work; they do not replace the operator-controlled CLI path.

## Status

- **[TESTED]** A Work MUST have a status that describes its place in the lifecycle.
- **[TESTED]** The set of statuses MUST be a closed enumeration in the domain crate. Unknown values MUST be a schema error, not a string that is stored and later interpreted.
- **[TESTED]** Statuses that require a different kind of exit MUST be distinct statuses.
- **[TESTED]** Names of a team's delivery phases (planning, apply, review, and similar) MUST NOT be canonical domain statuses. Those names belong to a Worker profile, not to Work itself.

The first runtime slice statuses are this closed set. They are lifecycle names, not delivery phases.

| Status | Role |
| --- | --- |
| `ready` | Work can be started |
| `running` | In-flight execution; occupies a Worker slot |
| `succeeded` | Successfully completed terminal |
| `failed` | Failed terminal |
| `parked` | Does not consume a Worker slot |

- **[TESTED]** The domain status enumeration for the first runtime slice MUST be exactly: `ready`, `running`, `succeeded`, `failed`, `parked`.

## Attributes

- **[TESTED]** A Work MUST carry a set of attributes that describe its content (what to do, which workspace root, which Worker profile).
- **[TESTED]** The first-slice attributes MUST be a goal string and a Worker profile name. The profile name is an opaque configuration key, not a vendor branch in the domain.
- **[TESTED]** Attribute names that identify a vendor or tracker product MUST NOT appear in the domain model.
- **[ENFORCED]** The first slice MUST NOT require a Work relation graph. Relations remain allowed later; they are not needed to run `create` / `next` / `start`.

## Relations

- **[UNTESTED]** Work MAY relate to other Work so that a graph can be built (parent/child, blocks, follows).
- **[UNTESTED]** Relations MUST be data on Work, not implicit conversation between Workers.

## Writer

- **[ENFORCED]** At any moment, the status of one Work MUST be writable by only one subject: Workengine.
- **[ENFORCED]** A Worker MUST NOT write Work status.
- **[ENFORCED]** An inbound or publish adapter MUST NOT write Work status.

## Related

- Worker: [worker.md](worker.md)
- Workspace: [workspace.md](workspace.md)
- Workflow (transitions, persistence, resume): [workflow.md](workflow.md)
