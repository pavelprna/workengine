# 0012. Deny-by-default Worker policy and brokered egress

- Status: Accepted
- Date: 2026-09-11

## Context

ADR 0005 made Bubblewrap or OCI mandatory, removed the inherited environment,
and disabled container networking. It did not authenticate a Bubblewrap root,
prove that a locally cached OCI image still had the configured digest, or give
both backends one finite resource and permission policy. Several
control-plane files were also opened by pathname after a Worker had been able
to write in the containing directory.

Coding agents sometimes need network access, but joining the host network
would turn a profile flag into ambient access. Network policy must remain a
control-plane capability rather than a convention in the Worker prompt.

## Decision

- Every Bubblewrap profile identifies its runtime root with a deterministic
  SHA-256 tree digest. Workengine verifies it while resolving the profile and
  immediately before every spawn. The digest covers entry type, relative path,
  Unix mode, file bytes, and symlink target without following symlinks.
- Every OCI profile uses an exact `name@sha256:<digest>` reference. Before a
  lease is claimed and again before spawn, the selected Docker or Podman engine
  must report that digest for the local image.
- Every user Worker has a finite memory, process, file-descriptor, file-size,
  CPU/wall-time policy. OCI applies engine limits. Bubblewrap is launched
  through `prlimit`. Both backends drop every capability, set no-new-privileges
  where the backend exposes it, and require an explicit digest-verified seccomp
  profile.
- The permission profile is deny-by-default. The Worker has no host network.
  A profile may explicitly mount one already-running Unix-domain egress broker
  socket. The Worker receives only `WORKENGINE_EGRESS_SOCKET`; Workengine never
  exposes a direct network namespace as the way to satisfy that permission.
  Destination policy and protocol handling belong to the broker.
- Commands launched by the runtime adapter receive a cleared, fixed
  Workengine-owned environment. Secret values remain transient files. Child
  output remains payload-free at every Workengine observation boundary.
- Workengine opens Worker-writable outcome, checkpoint, owner, memory, goal,
  operator-input, database, and daemon-lock files without following their final
  symlink. Directory inputs and broker/policy paths are type-checked before
  use.
- Goal text, operator input, prior memory, child output, and future inbound/web
  text are data. Only closed typed application operations and validated
  attempt-scoped artifacts can request a control-plane transition.

## Consequences

- Existing user profiles must add a runtime digest and seccomp profile. This is
  an intentional pre-1.0 fail-closed policy change. `digest-rootfs` computes
  the exact digest used by Workengine.
- A Bubblewrap host must provide both `bwrap` and `prlimit`. An OCI host must
  have the pinned image present in the selected engine before Work is started.
- Networked Workers need a separately supervised policy broker. Merely setting
  proxy variables, sharing the host network, or naming an arbitrary socket is
  not an implicit permission.
- Rootfs verification is proportional to the size of the runtime tree and is
  performed again at the spawn boundary to detect mutation after profile
  resolution.
